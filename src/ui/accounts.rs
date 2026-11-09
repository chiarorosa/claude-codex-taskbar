//! View model for the "Claude account" submenu (#57). Pure: the strip window reads the live
//! login and the store, and this decides what the menu shows.

use crate::i18n::{Key, Lang, fmt, t};
use crate::limits::ProviderStatus;
use crate::limits::switcher::lock::LockError;
use crate::limits::switcher::store::{
    Accounts, Expiry, Ineligible, LastSeen, LiveLogin, SaveError, identify_slot,
    login_expiry_state, parse_identity, parse_login, plan_eligible,
};
use crate::limits::switcher::switch::SwitchError;

/// What the live login is, relative to the saved slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveState {
    /// The saved slot holding the live login, if any.
    pub slot: Option<u32>,
    /// The live login passes the paid-plan gate, so it may be saved.
    pub eligible: bool,
}

pub fn live_state(accounts: &Accounts, live: Option<&LiveLogin>, now: i64) -> LiveState {
    let Some(live) = live else {
        return LiveState::default();
    };
    let identity = parse_identity(&live.config_text);
    let login = parse_login(live.credentials.expose());
    LiveState {
        slot: identity
            .as_ref()
            .and_then(|id| identify_slot(&accounts.slots, id)),
        eligible: match (&login, &identity) {
            (Some(login), Some(id)) => plan_eligible(login, id, now).is_ok(),
            _ => false,
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub label: String,
    /// The live account; shown checked.
    pub active: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountMenu {
    /// Switch targets, in slot order. Empty until there is another account to switch to.
    pub entries: Vec<Entry>,
    /// Shown, disabled, when there is nothing to switch to yet.
    pub hint: Option<String>,
    /// Shown, disabled, when the live login is not saved.
    pub unsaved_note: Option<String>,
    pub can_save: bool,
    /// (slot id, email) for the "Remove saved account" submenu.
    pub removable: Vec<(u32, String)>,
}

/// The submenu, or `None` to hide it (nothing saved and nothing that could be saved).
pub fn account_menu(
    accounts: &Accounts,
    live: LiveState,
    switching: bool,
    now: i64,
    lang: Lang,
) -> Option<AccountMenu> {
    if accounts.slots.is_empty() && !live.eligible {
        return None;
    }
    let has_target = accounts.slots.iter().any(|s| Some(s.id) != live.slot);
    let entries = if has_target {
        accounts
            .slots
            .iter()
            .map(|s| {
                let active = Some(s.id) == live.slot;
                let expiry = login_expiry_state(s.login_expires_at, now);
                Entry {
                    id: s.id,
                    label: entry_label(
                        &s.email,
                        &s.plan,
                        s.last_seen.as_ref(),
                        expiry,
                        active,
                        now,
                        lang,
                    ),
                    active,
                    enabled: !switching && !active && expiry != Expiry::Expired,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(AccountMenu {
        entries,
        hint: (!has_target).then(|| t(lang, Key::AccountAddHint).to_string()),
        unsaved_note: (live.slot.is_none() && live.eligible)
            .then(|| t(lang, Key::AccountNotSaved).to_string()),
        can_save: live.eligible && !switching,
        removable: accounts
            .slots
            .iter()
            .map(|s| (s.id, s.email.clone()))
            .collect(),
    })
}

/// The strip's current Claude usage, stored on the slot being left. Errors store nothing.
pub fn last_seen_from(status: &ProviderStatus) -> Option<LastSeen> {
    match status {
        ProviderStatus::Ok { usage, at } | ProviderStatus::Stale { usage, at, .. } => {
            Some(LastSeen {
                five_hour_pct: usage.five_hour.as_ref().map(|w| w.used_pct),
                weekly_pct: usage.weekly.as_ref().map(|w| w.used_pct),
                at: *at,
            })
        }
        _ => None,
    }
}

fn plan_name(plan: &str) -> String {
    let mut chars = plan.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn ago(secs: i64, lang: Lang) -> String {
    let secs = secs.max(0);
    let (key, n) = if secs < 3600 {
        (Key::AgoMinutesFmt, (secs / 60).max(1))
    } else if secs < 24 * 3600 {
        (Key::AgoHoursFmt, secs / 3600)
    } else {
        (Key::AgoDaysFmt, secs / (24 * 3600))
    };
    fmt(t(lang, key), &n.to_string())
}

/// One switch target: `email (Plan)`, then its last-seen usage and any login warning.
pub fn entry_label(
    slot_email: &str,
    plan: &str,
    seen: Option<&LastSeen>,
    expiry: Expiry,
    active: bool,
    now: i64,
    lang: Lang,
) -> String {
    let mut parts = vec![format!("{slot_email} ({})", plan_name(plan))];
    if !active && let Some(seen) = seen {
        if let Some(p) = seen.five_hour_pct {
            parts.push(format!("5h {p:.0}%"));
        }
        if let Some(p) = seen.weekly_pct {
            parts.push(format!("7d {p:.0}%"));
        }
        parts.push(ago(now - seen.at, lang));
    }
    match expiry {
        Expiry::Soon { days } => {
            parts.push(fmt(t(lang, Key::AccountExpiresFmt), &days.to_string()))
        }
        Expiry::Expired => parts.push(t(lang, Key::AccountNeedsLogin).to_string()),
        Expiry::Ok | Expiry::Unknown => {}
    }
    parts.join(" · ")
}

fn ineligible_text(lang: Lang, why: &Ineligible) -> String {
    let key = match why {
        Ineligible::NotPaid(_) => Key::ErrNotPaid,
        Ineligible::Trial => Key::ErrTrial,
        Ineligible::NotSubscription => Key::ErrNotSubscription,
        Ineligible::NoIdentity => Key::ErrNoIdentity,
    };
    t(lang, key).to_string()
}

pub fn switch_error_text(lang: Lang, err: &SwitchError) -> String {
    let reason = match err {
        SwitchError::ConfigDirOverride => t(lang, Key::ErrConfigDir).to_string(),
        SwitchError::NoSuchSlot => t(lang, Key::ErrNoSuchSlot).to_string(),
        SwitchError::Ineligible(why) => ineligible_text(lang, why),
        SwitchError::Expired => t(lang, Key::ErrExpired).to_string(),
        SwitchError::UnreadableConfig => t(lang, Key::ErrUnreadableConfig).to_string(),
        SwitchError::Lock(LockError::Timeout(_)) => t(lang, Key::ErrLocked).to_string(),
        SwitchError::Lock(LockError::Io(e))
        | SwitchError::StashFailed(e)
        | SwitchError::RolledBack(e)
        | SwitchError::Io(e) => fmt(t(lang, Key::ErrNothingChangedFmt), e),
    };
    fmt(t(lang, Key::SwitchFailedFmt), &reason)
}

pub fn save_error_text(lang: Lang, err: &SaveError) -> String {
    let reason = match err {
        SaveError::Ineligible(why) => ineligible_text(lang, why),
        SaveError::ConfigDirOverride => t(lang, Key::ErrConfigDir).to_string(),
        SaveError::Io(e) => fmt(t(lang, Key::ErrNothingChangedFmt), e),
    };
    fmt(t(lang, Key::SaveFailedFmt), &reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::FetchError;
    use crate::limits::switcher::Secret;
    use crate::limits::switcher::store::Slot;

    const NOW: i64 = 1_800_000_000;
    const L: Lang = Lang::En;

    fn slot(id: u32, email: &str) -> Slot {
        Slot {
            id,
            email: email.into(),
            account_uuid: format!("u-{id}"),
            organization_uuid: "org".into(),
            organization_name: String::new(),
            plan: "max".into(),
            login_expires_at: Some(NOW + 30 * 86400),
            last_seen: None,
            saved_at: NOW,
            label: None,
        }
    }

    fn accounts(slots: Vec<Slot>) -> Accounts {
        Accounts {
            slots,
            active_id: None,
            next_id: 0,
        }
    }

    fn live(uuid: &str, email: &str, plan: &str) -> LiveLogin {
        LiveLogin {
            credentials: Secret::new(format!(
                r#"{{"claudeAiOauth":{{"accessToken":"a","refreshToken":"r","subscriptionType":"{plan}"}}}}"#
            )),
            config_text: format!(
                r#"{{"oauthAccount":{{"accountUuid":"{uuid}","emailAddress":"{email}","organizationUuid":"org"}}}}"#
            ),
        }
    }

    #[test]
    fn live_state_finds_the_slot_and_the_gate() {
        let acc = accounts(vec![slot(1, "a@x"), slot(2, "b@x")]);
        assert_eq!(
            live_state(&acc, Some(&live("u-2", "b@x", "max")), NOW),
            LiveState {
                slot: Some(2),
                eligible: true
            }
        );
        assert_eq!(
            live_state(&acc, Some(&live("u-9", "c@x", "free")), NOW),
            LiveState {
                slot: None,
                eligible: false
            }
        );
        assert_eq!(live_state(&acc, None, NOW), LiveState::default());
    }

    #[test]
    fn hidden_when_nothing_is_saved_or_saveable() {
        assert_eq!(
            account_menu(&accounts(vec![]), LiveState::default(), false, NOW, L),
            None
        );
    }

    #[test]
    fn first_account_offers_save_and_a_hint() {
        let menu = account_menu(
            &accounts(vec![]),
            LiveState {
                slot: None,
                eligible: true,
            },
            false,
            NOW,
            L,
        )
        .unwrap();
        assert!(menu.can_save && menu.entries.is_empty());
        assert!(menu.hint.is_some());
        assert!(menu.unsaved_note.is_some());
    }

    #[test]
    fn one_saved_live_account_still_has_nothing_to_switch_to() {
        let acc = accounts(vec![slot(1, "a@x")]);
        let menu = account_menu(
            &acc,
            LiveState {
                slot: Some(1),
                eligible: true,
            },
            false,
            NOW,
            L,
        )
        .unwrap();
        assert!(menu.entries.is_empty());
        assert!(menu.hint.is_some() && menu.unsaved_note.is_none());
        assert_eq!(menu.removable, vec![(1, "a@x".to_string())]);
    }

    #[test]
    fn two_accounts_list_both_with_the_live_one_checked() {
        let acc = accounts(vec![slot(1, "a@x"), slot(2, "b@x")]);
        let menu = account_menu(
            &acc,
            LiveState {
                slot: Some(2),
                eligible: true,
            },
            false,
            NOW,
            L,
        )
        .unwrap();
        let summary: Vec<_> = menu
            .entries
            .iter()
            .map(|e| (e.id, e.active, e.enabled))
            .collect();
        assert_eq!(summary, vec![(1, false, true), (2, true, false)]);
        assert!(menu.hint.is_none());
    }

    #[test]
    fn an_unsaved_live_login_can_switch_to_a_single_slot() {
        let acc = accounts(vec![slot(1, "a@x")]);
        let menu = account_menu(
            &acc,
            LiveState {
                slot: None,
                eligible: true,
            },
            false,
            NOW,
            L,
        )
        .unwrap();
        assert_eq!(menu.entries.len(), 1);
        assert!(menu.entries[0].enabled);
        assert!(menu.unsaved_note.is_some());
    }

    #[test]
    fn expired_slots_are_disabled_and_everything_is_disabled_while_switching() {
        let mut expired = slot(1, "a@x");
        expired.login_expires_at = Some(NOW - 1);
        let acc = accounts(vec![expired, slot(2, "b@x"), slot(3, "c@x")]);
        let state = LiveState {
            slot: Some(3),
            eligible: true,
        };
        let menu = account_menu(&acc, state, false, NOW, L).unwrap();
        assert!(!menu.entries[0].enabled && menu.entries[1].enabled);
        let busy = account_menu(&acc, state, true, NOW, L).unwrap();
        assert!(busy.entries.iter().all(|e| !e.enabled) && !busy.can_save);
    }

    #[test]
    fn labels_show_plan_last_seen_and_warnings() {
        let seen = LastSeen {
            five_hour_pct: Some(92.4),
            weekly_pct: Some(40.0),
            at: NOW - 3 * 3600,
        };
        assert_eq!(
            entry_label("a@x", "max", Some(&seen), Expiry::Ok, false, NOW, L),
            "a@x (Max) · 5h 92% · 7d 40% · 3h ago"
        );
        assert_eq!(
            entry_label("a@x", "pro", None, Expiry::Soon { days: 2 }, false, NOW, L),
            "a@x (Pro) · ⚠ login expires in 2d"
        );
        assert_eq!(
            entry_label("a@x", "team", None, Expiry::Expired, false, NOW, L),
            "a@x (Team) · ⚠ needs /login"
        );
        // The live account shows no stale snapshot; the strip already shows its usage.
        assert_eq!(
            entry_label("a@x", "max", Some(&seen), Expiry::Ok, true, NOW, L),
            "a@x (Max)"
        );
        let partial = LastSeen {
            five_hour_pct: None,
            weekly_pct: Some(5.0),
            at: NOW - 90,
        };
        assert_eq!(
            entry_label(
                "a@x",
                "enterprise",
                Some(&partial),
                Expiry::Unknown,
                false,
                NOW,
                Lang::PtBr
            ),
            "a@x (Enterprise) · 7d 5% · há 1min"
        );
    }

    #[test]
    fn last_seen_comes_from_ok_or_stale_data_only() {
        use crate::limits::{ProviderUsage, WindowUsage};
        let usage = ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: 92.0,
                resets_at: None,
            }),
            weekly: None,
            ..Default::default()
        };
        let ok = ProviderStatus::Ok {
            usage: usage.clone(),
            at: NOW,
        };
        assert_eq!(
            last_seen_from(&ok),
            Some(LastSeen {
                five_hour_pct: Some(92.0),
                weekly_pct: None,
                at: NOW
            })
        );
        let stale = ProviderStatus::Stale {
            usage,
            at: NOW - 60,
            err: FetchError::Expired,
        };
        assert_eq!(last_seen_from(&stale).unwrap().at, NOW - 60);
        assert_eq!(
            last_seen_from(&ProviderStatus::Failed(FetchError::Expired)),
            None
        );
        assert_eq!(last_seen_from(&ProviderStatus::Loading), None);
    }

    #[test]
    fn every_error_has_a_message_in_both_languages() {
        let switch_errors = [
            SwitchError::ConfigDirOverride,
            SwitchError::NoSuchSlot,
            SwitchError::Ineligible(Ineligible::Trial),
            SwitchError::Expired,
            SwitchError::UnreadableConfig,
            SwitchError::Lock(LockError::Timeout("x".into())),
            SwitchError::StashFailed("e".into()),
            SwitchError::RolledBack("e".into()),
            SwitchError::Io("e".into()),
        ];
        let save_errors = [
            SaveError::Ineligible(Ineligible::NotPaid(Some("free".into()))),
            SaveError::Ineligible(Ineligible::NotSubscription),
            SaveError::Ineligible(Ineligible::Trial),
            SaveError::Ineligible(Ineligible::NoIdentity),
            SaveError::ConfigDirOverride,
            SaveError::Io("e".into()),
        ];
        for lang in [Lang::En, Lang::PtBr] {
            for e in &switch_errors {
                let text = switch_error_text(lang, e);
                assert!(!text.is_empty() && !text.contains("{}"), "{e:?}");
            }
            for e in &save_errors {
                let text = save_error_text(lang, e);
                assert!(!text.is_empty() && !text.contains("{}"), "{e:?}");
            }
        }
        assert!(
            save_error_text(L, &SaveError::Ineligible(Ineligible::NotPaid(None))).contains("paid")
        );
    }
}
