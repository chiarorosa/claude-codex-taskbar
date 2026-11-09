//! The switch transaction (#56): back up the outgoing login, activate the target slot, and roll
//! back on any failure. Everything runs under Claude Code's own locks and makes no network
//! calls; tokens are never refreshed here.

use super::lock::{ClaudeLocks, DEFAULT_TIMEOUT, LockError};
use super::store::{
    Expiry, Ineligible, LastSeen, Store, identify_slot, live_config_path, live_credentials_path,
    login_expiry_state, parse_identity, parse_login, plan_eligible,
};
use super::{Secret, fsio};
use serde_json::{Map, Value};
use std::io;
use std::path::Path;
use std::time::Duration;

/// Credential keys that belong to the machine, not the account: on activation the live copy
/// wins, absence included. Every other key travels with the slot.
pub const SHARED_CREDENTIAL_KEYS: [&str; 5] = [
    "mcpOAuth",
    "mcpOAuthClientConfig",
    "mcpXaaIdp",
    "mcpXaaIdpConfig",
    "pluginSecrets",
];

/// What the live credential is, relative to the slot it is filed under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outgoing {
    /// Byte-identical to the stored copy: nothing to back up.
    SameBytes,
    /// Same refresh token, rotated access token: back up.
    SameLineage,
    /// Claude Code emptied the tokens after a rejected refresh: never back up.
    Wiped,
    /// No saved slot holds this login: keep a safety copy.
    Unsaved,
    /// A different refresh token (a normal rotation): back up, keeping `.prev`.
    DifferentLineage,
}

pub fn classify_outgoing(stored: Option<&str>, live: &str) -> Outgoing {
    if is_wiped(live) {
        return Outgoing::Wiped;
    }
    let Some(stored) = stored else {
        return Outgoing::Unsaved;
    };
    if stored == live {
        return Outgoing::SameBytes;
    }
    match (refresh_token(stored), refresh_token(live)) {
        (Some(a), Some(b)) if a == b => Outgoing::SameLineage,
        _ => Outgoing::DifferentLineage,
    }
}

fn oauth_of(creds: &str) -> Option<Map<String, Value>> {
    let v: Value = serde_json::from_str(creds).ok()?;
    v.get("claudeAiOauth")?.as_object().cloned()
}

fn refresh_token(creds: &str) -> Option<String> {
    oauth_of(creds)?
        .get("refreshToken")?
        .as_str()
        .filter(|t| !t.is_empty())
        .map(String::from)
}

/// A login whose access and refresh tokens are both empty or missing.
fn is_wiped(creds: &str) -> bool {
    let Some(oauth) = oauth_of(creds) else {
        return false;
    };
    let empty = |k: &str| {
        oauth
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
    };
    empty("accessToken") && empty("refreshToken")
}

fn credential_object(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// The credential to write: the target slot's, with the machine-shared keys from `live`.
pub fn compose_activation(target: &str, live: &str) -> String {
    let (Some(mut composed), Some(live)) = (credential_object(target), credential_object(live))
    else {
        return target.to_string();
    };
    if !composed.contains_key("claudeAiOauth") {
        return target.to_string();
    }
    for key in SHARED_CREDENTIAL_KEYS {
        composed.remove(key);
        if let Some(v) = live.get(key) {
            composed.insert(key.to_string(), v.clone());
        }
    }
    Value::Object(composed).to_string()
}

#[derive(Debug, PartialEq, Eq)]
pub struct TornConfig;

/// `~/.claude.json` with only its `oauthAccount` replaced.
pub fn splice_oauth_account(config_text: &str, oauth: &Value) -> Result<String, TornConfig> {
    let mut config = credential_object(config_text).ok_or(TornConfig)?;
    config.insert("oauthAccount".into(), oauth.clone());
    serde_json::to_string_pretty(&Value::Object(config)).map_err(|_| TornConfig)
}

/// Which file a transaction write goes to; lets tests fail a chosen step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Credentials,
    Config,
    Index,
}

/// Writes one committed file of the transaction.
pub type StepWriter<'a> = dyn FnMut(Step, &Path, &[u8]) -> io::Result<()> + 'a;

#[derive(Debug, PartialEq)]
pub enum SwitchError {
    ConfigDirOverride,
    NoSuchSlot,
    Ineligible(Ineligible),
    /// The target slot's login has expired: run `/login` as that account and save it again.
    Expired,
    /// `~/.claude.json` is missing or does not parse; nothing was changed.
    UnreadableConfig,
    Lock(LockError),
    /// The live login could not be preserved, so it was not overwritten.
    StashFailed(String),
    /// A write failed; both Claude files were restored.
    RolledBack(String),
    Io(String),
}

impl From<io::Error> for SwitchError {
    fn from(e: io::Error) -> Self {
        SwitchError::Io(e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Switched {
    pub from: Option<u32>,
    pub to: u32,
    pub outgoing: Outgoing,
    /// The target was already the live login; nothing was written.
    pub no_op: bool,
}

/// Switches to slot `target`. `outgoing_last_seen` is the strip's current usage, stored on the
/// slot being left.
pub fn switch_to(
    store: &Store,
    home: &Path,
    target: u32,
    config_dir_override: bool,
    outgoing_last_seen: Option<LastSeen>,
    now: i64,
) -> Result<Switched, SwitchError> {
    if config_dir_override {
        return Err(SwitchError::ConfigDirOverride);
    }
    switch_with(
        store,
        home,
        target,
        outgoing_last_seen,
        now,
        DEFAULT_TIMEOUT,
        &mut |_, path, bytes| fsio::atomic_write(path, bytes),
    )
}

/// [`switch_to`] with an injectable lock timeout and writer for the three committed files.
pub fn switch_with(
    store: &Store,
    home: &Path,
    target: u32,
    outgoing_last_seen: Option<LastSeen>,
    now: i64,
    lock_timeout: Duration,
    write: &mut StepWriter<'_>,
) -> Result<Switched, SwitchError> {
    // Everything that can be refused is checked before any lock is taken.
    let mut accounts = store.load()?;
    let slot = accounts
        .slots
        .iter()
        .find(|s| s.id == target)
        .cloned()
        .ok_or(SwitchError::NoSuchSlot)?;
    if login_expiry_state(slot.login_expires_at, now) == Expiry::Expired {
        return Err(SwitchError::Expired);
    }
    let target_creds = store.read_credential(target)?;
    let target_oauth = store.read_oauth(target)?;
    let login = parse_login(target_creds.expose())
        .ok_or(SwitchError::Ineligible(Ineligible::NotSubscription))?;
    let target_identity =
        parse_identity(&serde_json::json!({ "oauthAccount": target_oauth }).to_string())
            .ok_or(SwitchError::Ineligible(Ineligible::NoIdentity))?;
    plan_eligible(&login, &target_identity, now).map_err(SwitchError::Ineligible)?;
    if login_expiry_state(login.refresh_expires_at, now) == Expiry::Expired {
        return Err(SwitchError::Expired);
    }

    let _locks = ClaudeLocks::acquire_all(home, lock_timeout).map_err(SwitchError::Lock)?;

    // Under the locks, the live files can't change except through us.
    let creds_path = live_credentials_path(home);
    let config_path = live_config_path(home);
    let live_creds = match std::fs::read_to_string(&creds_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let live_config =
        std::fs::read_to_string(&config_path).map_err(|_| SwitchError::UnreadableConfig)?;
    let new_config = splice_oauth_account(&live_config, &target_oauth)
        .map_err(|_| SwitchError::UnreadableConfig)?;
    let from = parse_identity(&live_config).and_then(|id| identify_slot(&accounts.slots, &id));

    let live_text = live_creds.clone().unwrap_or_default();
    if from == Some(target)
        && classify_outgoing(Some(target_creds.expose()), &live_text) != Outgoing::Wiped
    {
        // Already the live account. Re-file a rotated credential so the slot stays current.
        if live_creds.is_some() && live_text != target_creds.expose() {
            store.write_credential(target, &Secret::new(live_text))?;
        }
        return Ok(Switched {
            from,
            to: target,
            outgoing: Outgoing::SameBytes,
            no_op: true,
        });
    }

    // Back up the outgoing login.
    let stored = match from {
        Some(id) => Some(store.read_credential(id)?),
        None => None,
    };
    let outgoing = match &live_creds {
        None => Outgoing::Wiped, // logged out: nothing to preserve
        Some(text) => classify_outgoing(stored.as_ref().map(Secret::expose), text),
    };
    match (outgoing, from) {
        (Outgoing::SameLineage | Outgoing::DifferentLineage, Some(id)) => {
            store.write_credential(id, &Secret::new(live_text.clone()))?;
        }
        (Outgoing::Unsaved, _) => {
            store
                .stash(&Secret::new(live_text.clone()), now)
                .map_err(|e| SwitchError::StashFailed(e.to_string()))?;
        }
        _ => {}
    }

    // Commit: credentials, then ~/.claude.json, then the index. Roll back on any failure.
    let composed = compose_activation(target_creds.expose(), &live_text);
    if let Some(id) = from
        && let Some(s) = accounts.slots.iter_mut().find(|s| s.id == id)
        && outgoing_last_seen.is_some()
    {
        s.last_seen = outgoing_last_seen;
    }
    accounts.active_id = Some(target);
    let index = Store::index_bytes(&accounts)?;

    let mut done: Vec<Step> = Vec::new();
    let result = (|| -> io::Result<()> {
        write(Step::Credentials, &creds_path, composed.as_bytes())?;
        done.push(Step::Credentials);
        write(Step::Config, &config_path, new_config.as_bytes())?;
        done.push(Step::Config);
        write(Step::Index, &store.index_path(), &index)?;
        Ok(())
    })();
    if let Err(e) = result {
        for step in done.into_iter().rev() {
            let _ = match step {
                Step::Credentials => match &live_creds {
                    Some(text) => fsio::atomic_write(&creds_path, text.as_bytes()),
                    None => std::fs::remove_file(&creds_path),
                },
                Step::Config => fsio::atomic_write(&config_path, live_config.as_bytes()),
                Step::Index => Ok(()),
            };
        }
        return Err(SwitchError::RolledBack(e.to_string()));
    }
    Ok(Switched {
        from,
        to: target,
        outgoing,
        no_op: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::switcher::fsio::temp_dir;
    use crate::limits::switcher::store::LiveLogin;
    use std::path::PathBuf;

    const NOW: i64 = 1_800_000_000;

    fn creds(rt: &str, at: &str, extra: Value) -> String {
        let mut v = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": at,
                "refreshToken": rt,
                "subscriptionType": "max",
                "refreshTokenExpiresAt": (NOW + 30 * 86400) * 1000,
            }
        });
        if let Value::Object(extra) = extra {
            v.as_object_mut().unwrap().extend(extra);
        }
        v.to_string()
    }

    fn config(uuid: &str, email: &str) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "numStartups": 7,
            "projects": { "C:/dev": { "allowedTools": [] } },
            "oauthAccount": {
                "accountUuid": uuid,
                "emailAddress": email,
                "organizationUuid": "org-1",
            },
            "zzz": true,
        }))
        .unwrap()
    }

    fn real_home() -> PathBuf {
        PathBuf::from(std::env::var_os("USERPROFILE").expect("USERPROFILE"))
    }

    #[test]
    #[ignore = "takes and releases Claude Code's real locks for a moment (no credential access)"]
    fn live_claude_locks_round_trip() {
        let home = real_home();
        let locks = ClaudeLocks::acquire_all(&home, DEFAULT_TIMEOUT).unwrap();
        assert_eq!(locks.dirs().len(), 3);
        drop(locks);
        let specs = super::super::lock::claude_lock_specs(&home);
        assert!(specs.iter().all(|s| !s.dir.exists()));
    }

    // ---- classify ----

    #[test]
    fn classify_covers_every_outcome() {
        let a = creds("rt-a", "at-1", Value::Null);
        let a_rotated = creds("rt-a", "at-2", Value::Null);
        let b = creds("rt-b", "at-3", Value::Null);
        let wiped =
            r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","subscriptionType":"max"}}"#;
        assert_eq!(classify_outgoing(Some(&a), &a), Outgoing::SameBytes);
        assert_eq!(
            classify_outgoing(Some(&a), &a_rotated),
            Outgoing::SameLineage
        );
        assert_eq!(classify_outgoing(Some(&a), &b), Outgoing::DifferentLineage);
        assert_eq!(classify_outgoing(Some(&a), wiped), Outgoing::Wiped);
        assert_eq!(classify_outgoing(None, wiped), Outgoing::Wiped);
        assert_eq!(classify_outgoing(None, &a), Outgoing::Unsaved);
    }

    // ---- compose ----

    #[test]
    fn shared_keys_come_from_live_and_the_rest_from_target() {
        let target = creds(
            "rt-t",
            "at-t",
            serde_json::json!({ "mcpOAuth": {"old": 1}, "pluginSecrets": {"p": 1},
                                "trustedDeviceToken": "td-target", "futureKey": 5 }),
        );
        let live = creds(
            "rt-l",
            "at-l",
            serde_json::json!({ "mcpOAuth": {"new": 2}, "trustedDeviceToken": "td-live" }),
        );
        let out: Value = serde_json::from_str(&compose_activation(&target, &live)).unwrap();
        assert_eq!(out["claudeAiOauth"]["refreshToken"], "rt-t");
        assert_eq!(out["mcpOAuth"], serde_json::json!({"new": 2}));
        assert!(
            out.get("pluginSecrets").is_none(),
            "absent in live means absent"
        );
        assert_eq!(out["trustedDeviceToken"], "td-target");
        assert_eq!(out["futureKey"], 5);
    }

    #[test]
    fn compose_passes_through_a_non_json_side() {
        let target = creds("rt-t", "at-t", Value::Null);
        assert_eq!(compose_activation(&target, ""), target);
        assert_eq!(compose_activation(&target, "sk-ant-api03-FAKE"), target);
        assert_eq!(compose_activation("opaque", &target), "opaque");
    }

    // ---- splice ----

    #[test]
    fn splice_replaces_only_oauth_account() {
        let before = config("u-1", "a@x.com");
        let oauth = serde_json::json!({ "accountUuid": "u-2", "emailAddress": "b@x.com" });
        let after = splice_oauth_account(&before, &oauth).unwrap();
        let (mut b, a): (Value, Value) = (
            serde_json::from_str(&before).unwrap(),
            serde_json::from_str(&after).unwrap(),
        );
        assert_eq!(a["oauthAccount"], oauth);
        b["oauthAccount"] = oauth;
        assert_eq!(a, b, "every other key and value is preserved");
        assert!(after.contains("\n  \""), "pretty-printed like Claude Code");
    }

    #[test]
    fn splice_adds_a_missing_key_and_rejects_torn_json() {
        let oauth = serde_json::json!({ "emailAddress": "b@x.com" });
        let after: Value =
            serde_json::from_str(&splice_oauth_account(r#"{"a":1}"#, &oauth).unwrap()).unwrap();
        assert_eq!(after["oauthAccount"], oauth);
        assert_eq!(after["a"], 1);
        assert_eq!(splice_oauth_account("{torn", &oauth), Err(TornConfig));
        assert_eq!(splice_oauth_account("[1]", &oauth), Err(TornConfig));
    }

    // ---- transaction ----

    struct World {
        home: PathBuf,
        store: Store,
    }

    impl World {
        /// Two saved accounts, A (slot 1) and B (slot 2), with A live.
        fn new(name: &str) -> World {
            let home = temp_dir(name);
            std::fs::create_dir_all(home.join(".claude")).unwrap();
            let store = Store::new(home.join("cct-accounts"));
            let w = World { home, store };
            w.login(
                &creds("rt-b", "at-b", Value::Null),
                &config("u-b", "b@x.com"),
            );
            w.store.save_current(&w.live(), false, NOW).unwrap();
            w.login(
                &creds("rt-a", "at-a", Value::Null),
                &config("u-a", "a@x.com"),
            );
            w.store.save_current(&w.live(), false, NOW).unwrap();
            w
        }

        fn login(&self, creds: &str, config: &str) {
            std::fs::write(live_credentials_path(&self.home), creds).unwrap();
            std::fs::write(live_config_path(&self.home), config).unwrap();
        }

        fn live(&self) -> LiveLogin {
            super::super::store::read_live(&self.home).unwrap()
        }

        fn live_creds(&self) -> String {
            std::fs::read_to_string(live_credentials_path(&self.home)).unwrap()
        }

        fn live_config(&self) -> String {
            std::fs::read_to_string(live_config_path(&self.home)).unwrap()
        }

        fn switch(&self, target: u32) -> Result<Switched, SwitchError> {
            switch_to(&self.store, &self.home, target, false, None, NOW)
        }
    }

    #[test]
    fn switches_to_the_target_and_back() {
        let w = World::new("sw-basic");
        let out = w.switch(1).unwrap();
        assert_eq!(
            (out.from, out.to, out.outgoing),
            (Some(2), 1, Outgoing::SameBytes)
        );
        assert!(w.live_creds().contains("rt-b"));
        let cfg: Value = serde_json::from_str(&w.live_config()).unwrap();
        assert_eq!(cfg["oauthAccount"]["emailAddress"], "b@x.com");
        assert_eq!(cfg["numStartups"], 7);
        assert_eq!(w.store.load().unwrap().active_id, Some(1));

        w.switch(2).unwrap();
        assert!(w.live_creds().contains("rt-a"));
        let locks = super::super::lock::claude_lock_specs(&w.home);
        assert!(locks.iter().all(|l| !l.dir.exists()), "locks released");
    }

    #[test]
    fn a_rotated_live_login_is_backed_up_before_leaving() {
        let w = World::new("sw-rotate");
        // Claude Code refreshed A since it was saved.
        std::fs::write(
            live_credentials_path(&w.home),
            creds("rt-a2", "at-a2", Value::Null),
        )
        .unwrap();
        let out = w.switch(1).unwrap();
        assert_eq!(out.outgoing, Outgoing::DifferentLineage);
        assert!(
            w.store
                .read_credential(2)
                .unwrap()
                .expose()
                .contains("rt-a2")
        );
        assert!(w.store.dir().join("2.cred.prev").exists());
        w.switch(2).unwrap();
        assert!(
            w.live_creds().contains("rt-a2"),
            "the fresh generation comes back"
        );
    }

    #[test]
    fn wiped_tokens_are_never_backed_up() {
        let w = World::new("sw-wiped");
        std::fs::write(
            live_credentials_path(&w.home),
            r#"{"claudeAiOauth":{"accessToken":"","refreshToken":"","subscriptionType":"max"}}"#,
        )
        .unwrap();
        assert_eq!(w.switch(1).unwrap().outgoing, Outgoing::Wiped);
        assert!(
            w.store
                .read_credential(2)
                .unwrap()
                .expose()
                .contains("rt-a")
        );
    }

    #[test]
    fn an_unsaved_login_is_stashed_before_being_replaced() {
        let w = World::new("sw-unsaved");
        w.login(
            &creds("rt-c", "at-c", Value::Null),
            &config("u-c", "c@x.com"),
        );
        let out = w.switch(1).unwrap();
        assert_eq!((out.from, out.outgoing), (None, Outgoing::Unsaved));
        let stash = w.store.dir().join("stash");
        assert_eq!(std::fs::read_dir(&stash).unwrap().count(), 1);
    }

    #[test]
    fn switching_to_the_live_account_is_a_no_op() {
        let w = World::new("sw-noop");
        let before = (w.live_creds(), w.live_config());
        let out = w.switch(2).unwrap();
        assert!(out.no_op);
        assert_eq!((w.live_creds(), w.live_config()), before);
    }

    #[test]
    fn refuses_without_touching_anything() {
        let w = World::new("sw-refuse");
        let before = (w.live_creds(), w.live_config());
        assert_eq!(
            switch_to(&w.store, &w.home, 1, true, None, NOW),
            Err(SwitchError::ConfigDirOverride)
        );
        assert_eq!(w.switch(9), Err(SwitchError::NoSuchSlot));
        // Slot 1's login has since expired.
        let mut acc = w.store.load().unwrap();
        acc.slots
            .iter_mut()
            .find(|s| s.id == 1)
            .unwrap()
            .login_expires_at = Some(NOW - 1);
        w.store.save_index(&acc).unwrap();
        assert_eq!(w.switch(1), Err(SwitchError::Expired));
        assert_eq!((w.live_creds(), w.live_config()), before);
    }

    #[test]
    fn a_torn_config_is_refused() {
        let w = World::new("sw-torn");
        std::fs::write(live_config_path(&w.home), "{torn").unwrap();
        let creds_before = w.live_creds();
        assert_eq!(w.switch(1), Err(SwitchError::UnreadableConfig));
        assert_eq!(w.live_creds(), creds_before);
        assert_eq!(w.live_config(), "{torn");
    }

    #[test]
    fn a_held_claude_lock_times_out_without_changes() {
        let w = World::new("sw-locked");
        let before = (w.live_creds(), w.live_config());
        std::fs::create_dir(w.home.join(".claude.json.lock")).unwrap();
        let err = switch_with(
            &w.store,
            &w.home,
            1,
            None,
            NOW,
            Duration::from_millis(300),
            &mut |_, p, b| fsio::atomic_write(p, b),
        );
        assert!(matches!(err, Err(SwitchError::Lock(LockError::Timeout(_)))));
        assert_eq!((w.live_creds(), w.live_config()), before);
    }

    #[test]
    fn a_failure_at_any_step_restores_both_files() {
        for fail_at in [Step::Credentials, Step::Config, Step::Index] {
            let w = World::new("sw-rollback");
            let before = (w.live_creds(), w.live_config());
            let index_before = w.store.load().unwrap();
            let err = switch_with(
                &w.store,
                &w.home,
                1,
                None,
                NOW,
                Duration::from_secs(2),
                &mut |step, p, b| {
                    if step == fail_at {
                        Err(io::Error::other("disk full"))
                    } else {
                        fsio::atomic_write(p, b)
                    }
                },
            )
            .unwrap_err();
            assert!(
                matches!(err, SwitchError::RolledBack(_)),
                "{fail_at:?}: {err:?}"
            );
            assert_eq!((w.live_creds(), w.live_config()), before, "{fail_at:?}");
            assert_eq!(w.store.load().unwrap().active_id, index_before.active_id);
        }
    }

    #[test]
    fn the_outgoing_slot_keeps_the_last_seen_usage() {
        let w = World::new("sw-lastseen");
        let seen = LastSeen {
            five_hour_pct: Some(92.0),
            weekly_pct: Some(40.0),
            at: NOW,
        };
        switch_to(&w.store, &w.home, 1, false, Some(seen.clone()), NOW).unwrap();
        let acc = w.store.load().unwrap();
        assert_eq!(
            acc.slots.iter().find(|s| s.id == 2).unwrap().last_seen,
            Some(seen)
        );
    }
}
