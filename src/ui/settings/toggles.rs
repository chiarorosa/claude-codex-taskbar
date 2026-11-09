//! Checkbox-to-config mapping shared by the native Settings window and unit tests.

use crate::config::Config;
use crate::model::ItemId;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Toggle {
    Claude,
    Codex,
    Item(ItemId),
    Autostart,
    KeepAliveClaude,
    KeepAliveCodex,
    NotifyNearLimit,
}

pub(super) fn apply_toggle(cfg: &mut Config, toggle: &Toggle, on: bool) -> Option<bool> {
    match toggle {
        Toggle::Claude => cfg.claude_enabled = on,
        Toggle::Codex => cfg.codex_enabled = on,
        Toggle::Item(id) => cfg.set_visible(id, on),
        Toggle::Autostart => return Some(on),
        Toggle::KeepAliveClaude => {
            cfg.keep_claude_login = on;
            cfg.keepalive_asked = true;
        }
        Toggle::KeepAliveCodex => {
            cfg.keep_codex_login = on;
            cfg.codex_keepalive_asked = true;
        }
        Toggle::NotifyNearLimit => cfg.notify_near_limit = on,
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_toggle_changes_only_the_opt_in_and_preserves_thresholds() {
        let mut cfg = Config {
            notify_thresholds: vec![75, 90],
            hw_interval_secs: 3,
            ..Default::default()
        };
        assert!(!cfg.notify_near_limit);
        let expected = Config {
            notify_near_limit: true,
            ..cfg.clone()
        };
        assert_eq!(apply_toggle(&mut cfg, &Toggle::NotifyNearLimit, true), None);
        assert_eq!(cfg, expected);
        let saved = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&saved).unwrap(), cfg);
        assert_eq!(
            apply_toggle(&mut cfg, &Toggle::NotifyNearLimit, false),
            None
        );
        assert_eq!(
            cfg,
            Config {
                notify_near_limit: false,
                ..expected
            }
        );
    }

    #[test]
    fn provider_and_item_checkboxes_keep_their_existing_mapping() {
        let mut cfg = Config::default();
        assert_eq!(apply_toggle(&mut cfg, &Toggle::Claude, true), None);
        assert_eq!(apply_toggle(&mut cfg, &Toggle::Codex, true), None);
        assert!(cfg.claude_enabled && cfg.codex_enabled);
        assert_eq!(
            apply_toggle(&mut cfg, &Toggle::Item(ItemId::Cpu), false),
            None
        );
        assert!(!cfg.is_visible(&ItemId::Cpu));
        apply_toggle(&mut cfg, &Toggle::Item(ItemId::Cpu), true);
        assert!(cfg.is_visible(&ItemId::Cpu));
    }

    #[test]
    fn autostart_returns_an_action_without_mutating_configuration() {
        for on in [true, false] {
            let mut cfg = Config::default();
            assert_eq!(apply_toggle(&mut cfg, &Toggle::Autostart, on), Some(on));
            assert_eq!(cfg, Config::default());
        }
    }

    #[test]
    fn keepalive_checkboxes_still_record_that_the_user_was_asked() {
        let mut cfg = Config::default();
        for on in [true, false] {
            apply_toggle(&mut cfg, &Toggle::KeepAliveClaude, on);
            apply_toggle(&mut cfg, &Toggle::KeepAliveCodex, on);
            assert_eq!((cfg.keep_claude_login, cfg.keep_codex_login), (on, on));
            assert!(cfg.keepalive_asked && cfg.codex_keepalive_asked);
            assert!(!cfg.notify_near_limit);
        }
    }
}
