//! What the Settings window edits: its toggles, turning them into a `Config`, and the login
//! status lines. Pure; the window owns the controls.

use crate::config::Config;
use crate::i18n::Key;
use crate::model::ItemId;

#[derive(Clone, Debug, PartialEq)]
pub enum Toggle {
    Claude,
    Codex,
    Item(ItemId),
    Autostart,
    KeepAliveClaude,
    KeepAliveCodex,
    /// Windows balloon when a limit crosses a threshold (#15).
    NotifyNearLimit,
}

/// The config to apply, and the autostart switch when the window has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub cfg: Config,
    pub autostart: Option<bool>,
}

/// The config after one switch or chip changed: only that field moves, so changes made
/// elsewhere while the window is open (the keep-alive prompt, the strip menu) are kept.
pub fn apply_toggle(base: &Config, toggle: &Toggle, on: bool) -> Applied {
    let mut cfg = base.clone();
    let mut autostart = None;
    match toggle {
        Toggle::Claude => cfg.claude_enabled = on,
        Toggle::Codex => cfg.codex_enabled = on,
        Toggle::Item(id) => cfg.set_visible(id, on),
        Toggle::Autostart => autostart = Some(on),
        Toggle::NotifyNearLimit => cfg.notify_near_limit = on,
        Toggle::KeepAliveClaude => {
            cfg.keep_claude_login = on;
            cfg.keepalive_asked = true;
        }
        Toggle::KeepAliveCodex => {
            cfg.keep_codex_login = on;
            cfg.codex_keepalive_asked = true;
        }
    }
    Applied { cfg, autostart }
}

/// The config after the number fields were edited: only the intervals move, clamped; text
/// that is not a number keeps the current value.
pub fn apply_numbers(base: &Config, hw_text: &str, limits_text: &str) -> Config {
    let mut cfg = base.clone();
    let number = |text: &str, fallback: u32| text.trim().parse().unwrap_or(fallback);
    cfg.hw_interval_secs = number(hw_text, cfg.hw_interval_secs);
    cfg.limits_interval_secs = number(limits_text, cfg.limits_interval_secs);
    cfg.clamped()
}

/// The config after the label fields were edited. With `claude_saved`, the Claude label
/// belongs to the saved account and is written to the account index instead (#74).
pub fn apply_labels(base: &Config, claude: &str, codex: &str, claude_saved: bool) -> Config {
    let mut cfg = base.clone();
    if !claude_saved {
        cfg.claude_label = claude.to_string();
    }
    cfg.codex_label = codex.to_string();
    cfg.clamped()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Success,
    Warning,
}

/// The login line under a provider: found, or how to log in.
pub fn status_line(found: bool, missing: Key) -> (Tone, Key) {
    if found {
        (Tone::Success, Key::LoginFoundStatus)
    } else {
        (Tone::Warning, missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_one_toggle_changes_only_that_field() {
        // Changes made elsewhere while Settings is open (keep-alive prompt, strip menu) stay.
        let base = Config {
            keep_claude_login: true,
            ..Config::default()
        };
        let a = apply_toggle(&base, &Toggle::Item(ItemId::Gpu), false);
        assert!(!a.cfg.is_visible(&ItemId::Gpu));
        assert!(a.cfg.keep_claude_login);
        assert_eq!(a.autostart, None);
        let a = apply_toggle(&base, &Toggle::Autostart, true);
        assert_eq!((a.cfg, a.autostart), (base.clone(), Some(true)));
    }

    #[test]
    fn notify_toggle_changes_only_the_opt_in_and_keeps_thresholds() {
        let base = Config {
            notify_thresholds: vec![75, 90],
            hw_interval_secs: 3,
            ..Config::default()
        };
        let a = apply_toggle(&base, &Toggle::NotifyNearLimit, true);
        assert_eq!(
            (a.cfg, a.autostart),
            (
                Config {
                    notify_near_limit: true,
                    ..base.clone()
                },
                None
            )
        );
        assert!(
            !apply_toggle(&base, &Toggle::NotifyNearLimit, false)
                .cfg
                .notify_near_limit
        );
    }

    #[test]
    fn keep_alive_toggle_marks_only_its_own_prompt_as_answered() {
        let a = apply_toggle(&Config::default(), &Toggle::KeepAliveCodex, true);
        assert!(a.cfg.keep_codex_login && a.cfg.codex_keepalive_asked);
        assert!(!a.cfg.keepalive_asked);
    }

    #[test]
    fn numbers_apply_without_touching_toggles() {
        let base = Config {
            claude_enabled: false,
            ..Config::default()
        };
        let cfg = apply_numbers(&base, "2", "300");
        assert_eq!((cfg.hw_interval_secs, cfg.limits_interval_secs), (2, 300));
        assert!(!cfg.claude_enabled && !cfg.keepalive_asked);
    }

    #[test]
    fn toggles_map_to_config_like_the_old_apply() {
        let base = Config::default();
        assert!(
            !apply_toggle(&base, &Toggle::Claude, false)
                .cfg
                .claude_enabled
        );
        let keep = apply_toggle(&base, &Toggle::KeepAliveCodex, true).cfg;
        assert!(keep.keep_codex_login && keep.codex_keepalive_asked);
        let gpu = apply_toggle(&base, &Toggle::Item(ItemId::Gpu), false).cfg;
        assert!(!gpu.is_visible(&ItemId::Gpu));
        assert_eq!(
            apply_toggle(&base, &Toggle::Autostart, true).autostart,
            Some(true)
        );
    }

    #[test]
    fn number_fields_clamp_and_fall_back() {
        let base = Config {
            hw_interval_secs: 3,
            limits_interval_secs: 120,
            ..Config::default()
        };
        let f = |hw: &str, lim: &str| {
            let c = apply_numbers(&base, hw, lim);
            (c.hw_interval_secs, c.limits_interval_secs)
        };
        assert_eq!(f("", ""), (3, 120));
        assert_eq!(f("0", "0"), (1, 60));
        assert_eq!(f("99999", "99999"), (5, 3600));
        assert_eq!(f(" 4 ", "abc"), (4, 120));
    }

    #[test]
    fn labels_go_to_the_config_unless_the_claude_account_is_saved() {
        let base = Config::default();
        let cfg = apply_labels(&base, " Work ", "Business", false);
        assert_eq!(
            (cfg.claude_label.as_str(), cfg.codex_label.as_str()),
            ("Work", "Busin")
        );
        // A saved live account keeps its label in the account index, not in the config.
        let cfg = apply_labels(&base, "W", "C", true);
        assert_eq!(
            (cfg.claude_label.as_str(), cfg.codex_label.as_str()),
            ("P", "C")
        );
    }

    #[test]
    fn status_line_tones() {
        assert_eq!(
            status_line(true, Key::ClaudeNoLogin),
            (Tone::Success, Key::LoginFoundStatus)
        );
        assert_eq!(
            status_line(false, Key::CodexNoLogin),
            (Tone::Warning, Key::CodexNoLogin)
        );
    }
}
