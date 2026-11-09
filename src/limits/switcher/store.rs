//! Saved Claude accounts ("slots") and the rules for which logins may be saved (#55).
//!
//! Layout under `%APPDATA%\claude-codex-taskbar\accounts\`:
//! - `accounts.json`: the slot list and the active slot. Never contains tokens.
//! - `<id>.cred`: the full `.credentials.json` text, DPAPI-encrypted.
//! - `<id>.cred.prev`: the previous generation, kept on every overwrite.
//! - `<id>.oauth.json`: the slot's `oauthAccount` block from `~/.claude.json`.
//!
//! Only CCT's own directory is written here; the live login is read, never modified.

use super::{Secret, fsio, vault};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};

/// Plans that may be saved. Anything else, including a missing field, is refused.
pub const PAID_PLANS: [&str; 4] = ["pro", "max", "team", "enterprise"];
/// A login expiring within this many seconds is flagged.
pub const EXPIRY_WARNING_SECS: i64 = 7 * 24 * 3600;
/// Safety copies of displaced, unsaved logins kept in `stash\`.
pub const STASH_KEEP: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSeen {
    pub five_hour_pct: Option<f32>,
    pub weekly_pct: Option<f32>,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub id: u32,
    pub email: String,
    #[serde(default)]
    pub account_uuid: String,
    #[serde(default)]
    pub organization_uuid: String,
    #[serde(default)]
    pub organization_name: String,
    pub plan: String,
    /// Unix seconds when the stored login's refresh token expires, if Claude Code recorded it.
    #[serde(default)]
    pub login_expires_at: Option<i64>,
    #[serde(default)]
    pub last_seen: Option<LastSeen>,
    #[serde(default)]
    pub saved_at: i64,
    /// Shown before the Claude gauges while this account is live (#74). Not a secret.
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accounts {
    #[serde(default)]
    pub slots: Vec<Slot>,
    #[serde(default)]
    pub active_id: Option<u32>,
    #[serde(default)]
    pub next_id: u32,
}

/// Who a login belongs to, from the `oauthAccount` block of `~/.claude.json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub account_uuid: String,
    pub email: String,
    pub organization_uuid: String,
    pub organization_name: String,
    /// Unix seconds when a Claude Code trial ends, if the account is on one.
    pub trial_ends_at: Option<i64>,
}

/// What the credential says about the login, without its tokens.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoginInfo {
    pub plan: Option<String>,
    /// Unix seconds.
    pub refresh_expires_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ineligible {
    /// Not a Claude subscription login (API key, empty or unreadable credential).
    NotSubscription,
    /// The plan is not on [`PAID_PLANS`]; carries what was found.
    NotPaid(Option<String>),
    /// A Claude Code trial is still running.
    Trial,
    /// `~/.claude.json` has no usable `oauthAccount`.
    NoIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    Unknown,
    Ok,
    Soon { days: i64 },
    Expired,
}

/// Parses the `oauthAccount` block of `~/.claude.json`.
pub fn parse_identity(config_text: &str) -> Option<Identity> {
    let config: Value = serde_json::from_str(config_text).ok()?;
    let oauth = config.get("oauthAccount")?;
    let text = |key: &str| {
        oauth
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let email = text("emailAddress");
    if email.is_empty() {
        return None;
    }
    Some(Identity {
        account_uuid: text("accountUuid"),
        email,
        organization_uuid: text("organizationUuid"),
        organization_name: text("organizationName"),
        trial_ends_at: oauth.get("claudeCodeTrialEndsAt").and_then(timestamp_secs),
    })
}

/// A timestamp Claude Code wrote either as RFC 3339 text or as Unix milliseconds.
fn timestamp_secs(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => crate::timeutil::parse_rfc3339(s),
        Value::Number(n) => n.as_i64().map(|ms| ms / 1000),
        _ => None,
    }
}

/// Parses `.credentials.json`. `None` when it is not a Claude subscription login, including
/// a blank shell whose keys exist but whose tokens are empty (#45): saving that and switching
/// back to it would log Claude Code out.
pub fn parse_login(creds_text: &str) -> Option<LoginInfo> {
    let creds: Value = serde_json::from_str(creds_text).ok()?;
    let oauth = creds.get("claudeAiOauth")?.as_object()?;
    let has_token = |key: &str| {
        oauth
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|t| !t.is_empty())
    };
    if !has_token("accessToken") || !has_token("refreshToken") {
        return None;
    }
    Some(LoginInfo {
        plan: oauth
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(String::from),
        refresh_expires_at: oauth.get("refreshTokenExpiresAt").and_then(timestamp_secs),
    })
}

/// The label shown before the Claude gauges: the live saved account's own label, else the
/// config default (#74).
pub fn claude_label(default: &str, slots: &[Slot], live_slot: Option<u32>) -> String {
    live_slot
        .and_then(|id| slots.iter().find(|s| s.id == id))
        .and_then(|s| s.label.clone())
        .unwrap_or_else(|| default.to_string())
}

/// The paid-plans gate (#53, "Paid plans only"). Fails closed.
pub fn plan_eligible(login: &LoginInfo, identity: &Identity, now: i64) -> Result<(), Ineligible> {
    let paid = login
        .plan
        .as_deref()
        .is_some_and(|p| PAID_PLANS.contains(&p));
    if !paid {
        return Err(Ineligible::NotPaid(login.plan.clone()));
    }
    if identity.trial_ends_at.is_some_and(|end| end > now) {
        return Err(Ineligible::Trial);
    }
    Ok(())
}

/// Whether a saved login is about to expire, or already has.
pub fn login_expiry_state(expires_at: Option<i64>, now: i64) -> Expiry {
    match expires_at {
        None => Expiry::Unknown,
        Some(at) if at <= now => Expiry::Expired,
        Some(at) if at - now <= EXPIRY_WARNING_SECS => Expiry::Soon {
            days: (at - now) / (24 * 3600),
        },
        Some(_) => Expiry::Ok,
    }
}

/// The slot holding `identity`: by account uuid, else by email and organization. A uuid
/// conflict means a different account reusing the email, so no match.
pub fn identify_slot(slots: &[Slot], identity: &Identity) -> Option<u32> {
    let uuid = identity.account_uuid.as_str();
    let org_agrees = |s: &Slot| {
        s.organization_uuid.is_empty()
            || identity.organization_uuid.is_empty()
            || s.organization_uuid == identity.organization_uuid
    };
    if !uuid.is_empty()
        && let Some(s) = slots
            .iter()
            .find(|s| s.account_uuid == uuid && org_agrees(s))
    {
        return Some(s.id);
    }
    slots
        .iter()
        .find(|s| {
            s.email == identity.email
                && s.organization_uuid == identity.organization_uuid
                && (s.account_uuid.is_empty() || uuid.is_empty() || s.account_uuid == uuid)
        })
        .map(|s| s.id)
}

/// The live login, read from the user profile.
#[derive(Clone)]
pub struct LiveLogin {
    pub credentials: Secret,
    pub config_text: String,
}

impl std::fmt::Debug for LiveLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LiveLogin(<redacted>)")
    }
}

/// `%USERPROFILE%\.claude\.credentials.json`.
pub fn live_credentials_path(home: &Path) -> PathBuf {
    home.join(".claude").join(".credentials.json")
}

/// `%USERPROFILE%\.claude.json`.
pub fn live_config_path(home: &Path) -> PathBuf {
    home.join(".claude.json")
}

pub fn read_live(home: &Path) -> io::Result<LiveLogin> {
    Ok(LiveLogin {
        credentials: Secret::new(std::fs::read_to_string(live_credentials_path(home))?),
        config_text: std::fs::read_to_string(live_config_path(home))?,
    })
}

#[derive(Debug, PartialEq)]
pub enum SaveError {
    Ineligible(Ineligible),
    /// `CLAUDE_CONFIG_DIR` is set, so the live login is somewhere else.
    ConfigDirOverride,
    Io(String),
}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        SaveError::Io(e.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    pub id: u32,
    pub created: bool,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Store { dir }
    }

    /// `%APPDATA%\claude-codex-taskbar\accounts`.
    pub fn default_dir() -> PathBuf {
        crate::config::default_path()
            .parent()
            .map(|p| p.join("accounts"))
            .unwrap_or_else(|| PathBuf::from("accounts"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The slot list. A missing file is an empty list; a corrupt one is an error, so it is
    /// never silently replaced.
    pub fn load(&self) -> io::Result<Accounts> {
        match std::fs::read_to_string(self.index_path()) {
            Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Accounts::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save_index(&self, accounts: &Accounts) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_string_pretty(accounts).map_err(io::Error::other)?;
        fsio::atomic_write(&self.index_path(), json.as_bytes())
    }

    pub fn read_credential(&self, id: u32) -> io::Result<Secret> {
        let blob = std::fs::read(self.file(id, "cred"))?;
        let plain = vault::unprotect(&blob)?;
        String::from_utf8(plain)
            .map(Secret::new)
            .map_err(io::Error::other)
    }

    /// Stores a slot's credential, keeping the current one as `.prev`.
    pub fn write_credential(&self, id: u32, creds: &Secret) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.file(id, "cred");
        match std::fs::read(&path) {
            Ok(old) => fsio::atomic_write(&self.file(id, "cred.prev"), &old)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let blob = vault::protect(creds.expose().as_bytes())?;
        fsio::atomic_write(&path, &blob)
    }

    /// The slot's `oauthAccount` block.
    pub fn read_oauth(&self, id: u32) -> io::Result<Value> {
        let text = std::fs::read_to_string(self.file(id, "oauth.json"))?;
        serde_json::from_str(&text).map_err(io::Error::other)
    }

    pub fn write_oauth(&self, id: u32, oauth: &Value) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_string_pretty(oauth).map_err(io::Error::other)?;
        fsio::atomic_write(&self.file(id, "oauth.json"), json.as_bytes())
    }

    pub(crate) fn index_path(&self) -> PathBuf {
        self.dir.join("accounts.json")
    }

    pub(crate) fn index_bytes(accounts: &Accounts) -> io::Result<Vec<u8>> {
        serde_json::to_vec_pretty(accounts).map_err(io::Error::other)
    }

    /// Keeps an encrypted safety copy of a live login that belongs to no slot, before it is
    /// overwritten. Only the newest [`STASH_KEEP`] copies are kept.
    pub fn stash(&self, creds: &Secret, now: i64) -> io::Result<PathBuf> {
        let dir = self.dir.join("stash");
        std::fs::create_dir_all(&dir)?;
        let mut path = dir.join(format!("{now}.cred"));
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{now}-{n}.cred"));
            n += 1;
        }
        fsio::atomic_write(&path, &vault::protect(creds.expose().as_bytes())?)?;
        let mut copies: Vec<_> = std::fs::read_dir(&dir)?
            .flatten()
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        copies.sort();
        let excess = copies.len().saturating_sub(STASH_KEEP);
        for (_, old) in copies.into_iter().take(excess) {
            let _ = std::fs::remove_file(old);
        }
        Ok(path)
    }

    fn file(&self, id: u32, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    /// Saves the live login as a slot, or refreshes its existing slot (#53, "Save current
    /// account"). The live login becomes the active slot.
    pub fn save_current(
        &self,
        live: &LiveLogin,
        config_dir_override: bool,
        now: i64,
    ) -> Result<Saved, SaveError> {
        if config_dir_override {
            return Err(SaveError::ConfigDirOverride);
        }
        let login = parse_login(live.credentials.expose())
            .ok_or(SaveError::Ineligible(Ineligible::NotSubscription))?;
        let identity = parse_identity(&live.config_text)
            .ok_or(SaveError::Ineligible(Ineligible::NoIdentity))?;
        plan_eligible(&login, &identity, now).map_err(SaveError::Ineligible)?;
        let oauth = serde_json::from_str::<Value>(&live.config_text)
            .ok()
            .and_then(|c| c.get("oauthAccount").cloned())
            .ok_or(SaveError::Ineligible(Ineligible::NoIdentity))?;

        let mut accounts = self.load()?;
        let existing = identify_slot(&accounts.slots, &identity);
        let id = existing.unwrap_or_else(|| {
            let max = accounts.slots.iter().map(|s| s.id).max().unwrap_or(0);
            accounts.next_id.max(max + 1).max(1)
        });
        let previous = existing.and_then(|id| accounts.slots.iter().find(|s| s.id == id));
        let last_seen = previous.and_then(|s| s.last_seen.clone());
        // A renewed login keeps the label the user gave the account.
        let label = previous.and_then(|s| s.label.clone());
        let slot = Slot {
            id,
            email: identity.email,
            account_uuid: identity.account_uuid,
            organization_uuid: identity.organization_uuid,
            organization_name: identity.organization_name,
            plan: login.plan.unwrap_or_default(),
            login_expires_at: login.refresh_expires_at,
            last_seen,
            saved_at: now,
            label,
        };
        self.write_credential(id, &live.credentials)?;
        self.write_oauth(id, &oauth)?;
        match accounts.slots.iter_mut().find(|s| s.id == id) {
            Some(s) => *s = slot,
            None => accounts.slots.push(slot),
        }
        accounts.next_id = accounts.next_id.max(id + 1);
        accounts.active_id = Some(id);
        self.save_index(&accounts)?;
        Ok(Saved {
            id,
            created: existing.is_none(),
        })
    }

    /// Sets the label of saved account `id` (normalized like the config labels).
    pub fn set_label(&self, id: u32, label: &str) -> io::Result<()> {
        let mut accounts = self.load()?;
        let slot = accounts
            .slots
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such account"))?;
        slot.label = Some(crate::config::normalize_label(label));
        self.save_index(&accounts)
    }

    /// Deletes a slot and its files.
    pub fn remove_slot(&self, id: u32) -> io::Result<()> {
        let mut accounts = self.load()?;
        accounts.slots.retain(|s| s.id != id);
        if accounts.active_id == Some(id) {
            accounts.active_id = None;
        }
        self.save_index(&accounts)?;
        for ext in ["cred", "cred.prev", "oauth.json"] {
            match std::fs::remove_file(self.file(id, ext)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::switcher::fsio::temp_dir;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 24 * 3600;

    fn creds(plan: Option<&str>, refresh_token: &str) -> String {
        let mut oauth = serde_json::json!({
            "accessToken": "sk-ant-oat01-FAKE",
            "refreshToken": refresh_token,
            "expiresAt": 1_893_456_000_000_i64,
            "refreshTokenExpiresAt": (NOW + 30 * DAY) * 1000,
        });
        if let Some(p) = plan {
            oauth["subscriptionType"] = p.into();
        }
        serde_json::json!({ "claudeAiOauth": oauth, "mcpOAuth": { "x": 1 } }).to_string()
    }

    fn config(uuid: &str, email: &str, org: &str) -> String {
        serde_json::json!({
            "numStartups": 3,
            "projects": { "C:/dev": {} },
            "oauthAccount": {
                "accountUuid": uuid,
                "emailAddress": email,
                "organizationUuid": org,
                "organizationName": "Org",
            }
        })
        .to_string()
    }

    fn identity(uuid: &str, email: &str, org: &str) -> Identity {
        Identity {
            account_uuid: uuid.into(),
            email: email.into(),
            organization_uuid: org.into(),
            organization_name: "Org".into(),
            trial_ends_at: None,
        }
    }

    fn slot(id: u32, uuid: &str, email: &str, org: &str) -> Slot {
        Slot {
            id,
            email: email.into(),
            account_uuid: uuid.into(),
            organization_uuid: org.into(),
            organization_name: String::new(),
            plan: "pro".into(),
            login_expires_at: None,
            last_seen: None,
            saved_at: 0,
            label: None,
        }
    }

    fn live(plan: Option<&str>, rt: &str, uuid: &str, email: &str) -> LiveLogin {
        LiveLogin {
            credentials: Secret::new(creds(plan, rt)),
            config_text: config(uuid, email, "org-1"),
        }
    }

    #[test]
    #[ignore = "reads the real Claude login on this machine (read-only)"]
    fn live_login_parses_and_is_eligible() {
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        let live = read_live(&home).unwrap();
        let login = parse_login(live.credentials.expose()).expect("subscription login");
        let identity = parse_identity(&live.config_text).expect("oauthAccount");
        assert_eq!(
            plan_eligible(&login, &identity, crate::timeutil::now_unix()),
            Ok(())
        );
        assert!(login.refresh_expires_at.is_some());
    }

    // ---- parsing ----

    #[test]
    fn parses_identity_from_oauth_account() {
        let id = parse_identity(&config("u-1", "a@x.com", "org-1")).unwrap();
        assert_eq!(id, identity("u-1", "a@x.com", "org-1"));
    }

    #[test]
    fn identity_trial_end_accepts_rfc3339_and_millis() {
        let iso = r#"{"oauthAccount":{"emailAddress":"a@x.com","claudeCodeTrialEndsAt":"2027-01-15T00:00:00Z"}}"#;
        assert_eq!(
            parse_identity(iso).unwrap().trial_ends_at,
            Some(1_799_971_200)
        );
        let ms =
            r#"{"oauthAccount":{"emailAddress":"a@x.com","claudeCodeTrialEndsAt":1799971200000}}"#;
        assert_eq!(
            parse_identity(ms).unwrap().trial_ends_at,
            Some(1_799_971_200)
        );
        let null = r#"{"oauthAccount":{"emailAddress":"a@x.com","claudeCodeTrialEndsAt":null}}"#;
        assert_eq!(parse_identity(null).unwrap().trial_ends_at, None);
    }

    #[test]
    fn identity_requires_an_email() {
        assert_eq!(
            parse_identity(r#"{"oauthAccount":{"accountUuid":"u"}}"#),
            None
        );
        assert_eq!(parse_identity(r#"{"projects":{}}"#), None);
        assert_eq!(parse_identity("{torn"), None);
    }

    #[test]
    fn parses_login_plan_and_expiry_in_seconds() {
        let info = parse_login(&creds(Some("max"), "rt")).unwrap();
        assert_eq!(info.plan.as_deref(), Some("max"));
        assert_eq!(info.refresh_expires_at, Some(NOW + 30 * DAY));
    }

    #[test]
    fn api_keys_and_garbage_are_not_subscription_logins() {
        assert_eq!(parse_login("sk-ant-api03-FAKE"), None);
        assert_eq!(parse_login(""), None);
        assert_eq!(parse_login(r#"{"other":1}"#), None);
    }

    /// A credentials file can be a blank shell: the keys are there, the tokens are empty (#45).
    fn blank_shell(plan: &str, access: &str, refresh: &str) -> String {
        serde_json::json!({
            "claudeAiOauth": {
                "accessToken": access,
                "refreshToken": refresh,
                "expiresAt": 0,
                "scopes": [],
                "subscriptionType": plan,
                "rateLimitTier": "",
            },
            "organizationUuid": "",
        })
        .to_string()
    }

    #[test]
    fn blank_or_tokenless_credentials_are_not_subscription_logins() {
        for (access, refresh) in [("", ""), ("sk-ant-oat01-FAKE", ""), ("", "rt")] {
            assert_eq!(
                parse_login(&blank_shell("max", access, refresh)),
                None,
                "access={access:?} refresh={refresh:?}"
            );
        }
        let no_refresh = r#"{"claudeAiOauth":{"accessToken":"a","subscriptionType":"pro"}}"#;
        assert_eq!(parse_login(no_refresh), None);
    }

    // ---- plan gate ----

    #[test]
    fn paid_plans_are_eligible() {
        for plan in PAID_PLANS {
            let info = LoginInfo {
                plan: Some(plan.into()),
                refresh_expires_at: None,
            };
            assert_eq!(
                plan_eligible(&info, &identity("u", "a@x", "o"), NOW),
                Ok(()),
                "{plan}"
            );
        }
    }

    #[test]
    fn missing_empty_or_unknown_plans_are_refused() {
        for plan in [
            None,
            Some(""),
            Some("free"),
            Some("PRO "),
            Some("claude_free"),
        ] {
            let info = LoginInfo {
                plan: plan.map(String::from),
                refresh_expires_at: None,
            };
            assert_eq!(
                plan_eligible(&info, &identity("u", "a@x", "o"), NOW),
                Err(Ineligible::NotPaid(plan.map(String::from))),
            );
        }
    }

    #[test]
    fn an_active_trial_is_refused_but_an_ended_one_is_fine() {
        let info = LoginInfo {
            plan: Some("pro".into()),
            refresh_expires_at: None,
        };
        let mut id = identity("u", "a@x", "o");
        id.trial_ends_at = Some(NOW + DAY);
        assert_eq!(plan_eligible(&info, &id, NOW), Err(Ineligible::Trial));
        id.trial_ends_at = Some(NOW - DAY);
        assert_eq!(plan_eligible(&info, &id, NOW), Ok(()));
    }

    // ---- expiry ----

    #[test]
    fn expiry_states() {
        assert_eq!(login_expiry_state(None, NOW), Expiry::Unknown);
        assert_eq!(login_expiry_state(Some(NOW + 30 * DAY), NOW), Expiry::Ok);
        assert_eq!(
            login_expiry_state(Some(NOW + 3 * DAY + 10), NOW),
            Expiry::Soon { days: 3 }
        );
        assert_eq!(
            login_expiry_state(Some(NOW + 3600), NOW),
            Expiry::Soon { days: 0 }
        );
        assert_eq!(login_expiry_state(Some(NOW), NOW), Expiry::Expired);
        assert_eq!(login_expiry_state(Some(NOW - DAY), NOW), Expiry::Expired);
    }

    // ---- identify ----

    #[test]
    fn identify_by_uuid_first() {
        let slots = [
            slot(1, "u-1", "old@x.com", "org-1"),
            slot(2, "u-2", "a@x.com", "org-1"),
        ];
        assert_eq!(
            identify_slot(&slots, &identity("u-1", "new@x.com", "org-1")),
            Some(1)
        );
    }

    #[test]
    fn identify_falls_back_to_email_and_org() {
        let slots = [
            slot(1, "", "a@x.com", "org-1"),
            slot(2, "", "a@x.com", "org-2"),
        ];
        assert_eq!(
            identify_slot(&slots, &identity("u-9", "a@x.com", "org-2")),
            Some(2)
        );
        assert_eq!(
            identify_slot(&slots, &identity("u-9", "a@x.com", "org-3")),
            None
        );
    }

    #[test]
    fn a_uuid_conflict_is_not_a_match() {
        let slots = [slot(1, "u-1", "a@x.com", "org-1")];
        assert_eq!(
            identify_slot(&slots, &identity("u-2", "a@x.com", "org-1")),
            None
        );
    }

    // ---- store ----

    #[test]
    fn load_of_a_missing_index_is_empty_and_a_corrupt_one_is_an_error() {
        let store = Store::new(temp_dir("store-load"));
        assert_eq!(store.load().unwrap(), Accounts::default());
        std::fs::write(store.dir().join("accounts.json"), "{torn").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn save_current_creates_a_slot_and_makes_it_active() {
        let store = Store::new(temp_dir("store-save"));
        let saved = store
            .save_current(&live(Some("max"), "rt-1", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        assert_eq!(
            saved,
            Saved {
                id: 1,
                created: true
            }
        );
        let acc = store.load().unwrap();
        assert_eq!(acc.active_id, Some(1));
        let s = &acc.slots[0];
        assert_eq!((s.email.as_str(), s.plan.as_str()), ("a@x.com", "max"));
        assert_eq!(s.login_expires_at, Some(NOW + 30 * DAY));
        assert_eq!(s.saved_at, NOW);
        assert_eq!(
            store.read_credential(1).unwrap().expose(),
            creds(Some("max"), "rt-1")
        );
        assert_eq!(store.read_oauth(1).unwrap()["emailAddress"], "a@x.com");
    }

    #[test]
    fn saving_again_refreshes_the_slot_and_keeps_prev() {
        let store = Store::new(temp_dir("store-resave"));
        store
            .save_current(&live(Some("pro"), "rt-1", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        let again = store
            .save_current(
                &live(Some("max"), "rt-2", "u-1", "a@x.com"),
                false,
                NOW + 10,
            )
            .unwrap();
        assert_eq!(
            again,
            Saved {
                id: 1,
                created: false
            }
        );
        assert_eq!(store.load().unwrap().slots.len(), 1);
        assert_eq!(store.load().unwrap().slots[0].plan, "max");
        assert_eq!(
            store.read_credential(1).unwrap().expose(),
            creds(Some("max"), "rt-2")
        );
        let prev = vault::unprotect(&std::fs::read(store.dir().join("1.cred.prev")).unwrap());
        assert_eq!(prev.unwrap(), creds(Some("pro"), "rt-1").into_bytes());
    }

    #[test]
    fn a_second_account_gets_the_next_id() {
        let store = Store::new(temp_dir("store-two"));
        store
            .save_current(&live(Some("pro"), "a", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        let b = store
            .save_current(&live(Some("max"), "b", "u-2", "b@x.com"), false, NOW)
            .unwrap();
        assert_eq!(
            b,
            Saved {
                id: 2,
                created: true
            }
        );
        assert_eq!(store.load().unwrap().active_id, Some(2));
    }

    #[test]
    fn save_current_refuses_ineligible_logins_and_writes_nothing() {
        let store = Store::new(temp_dir("store-refuse"));
        let free = live(Some("free"), "rt", "u-1", "a@x.com");
        assert_eq!(
            store.save_current(&free, false, NOW),
            Err(SaveError::Ineligible(Ineligible::NotPaid(Some(
                "free".into()
            ))))
        );
        let api_key = LiveLogin {
            credentials: Secret::new("sk-ant-api03-FAKE".into()),
            config_text: config("u-1", "a@x.com", "org-1"),
        };
        assert_eq!(
            store.save_current(&api_key, false, NOW),
            Err(SaveError::Ineligible(Ineligible::NotSubscription))
        );
        let blank = LiveLogin {
            credentials: Secret::new(blank_shell("max", "", "")),
            config_text: config("u-1", "a@x.com", "org-1"),
        };
        assert_eq!(
            store.save_current(&blank, false, NOW),
            Err(SaveError::Ineligible(Ineligible::NotSubscription))
        );
        let no_identity = LiveLogin {
            credentials: Secret::new(creds(Some("pro"), "rt")),
            config_text: "{}".into(),
        };
        assert_eq!(
            store.save_current(&no_identity, false, NOW),
            Err(SaveError::Ineligible(Ineligible::NoIdentity))
        );
        let ok = live(Some("pro"), "rt", "u-1", "a@x.com");
        assert_eq!(
            store.save_current(&ok, true, NOW),
            Err(SaveError::ConfigDirOverride)
        );
        assert_eq!(std::fs::read_dir(store.dir()).unwrap().count(), 0);
    }

    #[test]
    fn the_index_and_oauth_files_never_contain_tokens() {
        let store = Store::new(temp_dir("store-notokens"));
        store
            .save_current(&live(Some("pro"), "RT-FAKE", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        for name in ["accounts.json", "1.oauth.json", "1.cred"] {
            let bytes = std::fs::read(store.dir().join(name)).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains("RT-FAKE") && !text.contains("sk-ant"),
                "{name}"
            );
        }
    }

    #[test]
    fn an_index_from_before_labels_loads_without_them() {
        let store = Store::new(temp_dir("store-label-old"));
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(
            store.dir().join("accounts.json"),
            r#"{"slots":[{"id":1,"email":"a@x.com","plan":"pro"}],"activeId":1,"nextId":2}"#,
        )
        .unwrap();
        assert_eq!(store.load().unwrap().slots[0].label, None);
    }

    #[test]
    fn set_label_is_stored_and_survives_saving_the_account_again() {
        let store = Store::new(temp_dir("store-label"));
        store
            .save_current(&live(Some("pro"), "a", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        store.set_label(1, "  Work ").unwrap();
        assert_eq!(
            store.load().unwrap().slots[0].label.as_deref(),
            Some("Work")
        );
        // Saving again (a renewed login) must not drop the label.
        store
            .save_current(&live(Some("pro"), "a2", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        assert_eq!(
            store.load().unwrap().slots[0].label.as_deref(),
            Some("Work")
        );
        assert!(store.set_label(9, "X").is_err(), "unknown slot");
    }

    #[test]
    fn claude_label_prefers_the_live_accounts_own_label() {
        let mut work = slot(2, "u-2", "w@x.com", "o");
        work.label = Some("W".into());
        let slots = vec![slot(1, "u-1", "p@x.com", "o"), work];
        assert_eq!(claude_label("P", &slots, Some(2)), "W");
        assert_eq!(
            claude_label("P", &slots, Some(1)),
            "P",
            "no own label: default"
        );
        assert_eq!(
            claude_label("Pess", &slots, None),
            "Pess",
            "unsaved login: default"
        );
    }

    #[test]
    fn remove_slot_deletes_its_files_and_clears_active() {
        let store = Store::new(temp_dir("store-remove"));
        store
            .save_current(&live(Some("pro"), "a", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        store
            .save_current(&live(Some("pro"), "a2", "u-1", "a@x.com"), false, NOW)
            .unwrap();
        store.remove_slot(1).unwrap();
        let acc = store.load().unwrap();
        assert!(acc.slots.is_empty());
        assert_eq!(acc.active_id, None);
        let left: Vec<_> = std::fs::read_dir(store.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left, ["accounts.json"]);
    }
}
