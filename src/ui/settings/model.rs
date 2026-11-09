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
}

/// The config to apply, and the autostart switch when the window has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub cfg: Config,
    pub autostart: Option<bool>,
}

pub fn config_from(
    base: &Config,
    toggles: &[(Toggle, bool)],
    hw_text: &str,
    limits_text: &str,
) -> Applied {
    let mut cfg = base.clone();
    let mut autostart = None;
    for (toggle, on) in toggles {
        let on = *on;
        match toggle {
            Toggle::Claude => cfg.claude_enabled = on,
            Toggle::Codex => cfg.codex_enabled = on,
            Toggle::Item(id) => cfg.set_visible(id, on),
            Toggle::Autostart => autostart = Some(on),
            Toggle::KeepAliveClaude => {
                cfg.keep_claude_login = on;
                cfg.keepalive_asked = true;
            }
            Toggle::KeepAliveCodex => {
                cfg.keep_codex_login = on;
                cfg.codex_keepalive_asked = true;
            }
        }
    }
    let number = |text: &str, fallback: u32| text.trim().parse().unwrap_or(fallback);
    cfg.hw_interval_secs = number(hw_text, cfg.hw_interval_secs);
    cfg.limits_interval_secs = number(limits_text, cfg.limits_interval_secs);
    Applied {
        cfg: cfg.clamped(),
        autostart,
    }
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
    fn toggles_map_to_config_like_the_old_apply() {
        let base = Config::default();
        let a = config_from(
            &base,
            &[
                (Toggle::Claude, false),
                (Toggle::KeepAliveCodex, true),
                (Toggle::Item(ItemId::Gpu), false),
                (Toggle::Autostart, true),
            ],
            "2",
            "300",
        );
        assert!(!a.cfg.claude_enabled);
        assert!(a.cfg.keep_codex_login && a.cfg.codex_keepalive_asked);
        assert!(!a.cfg.is_visible(&ItemId::Gpu));
        assert_eq!(
            (
                a.cfg.hw_interval_secs,
                a.cfg.limits_interval_secs,
                a.autostart
            ),
            (2, 300, Some(true))
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
            let c = config_from(&base, &[], hw, lim).cfg;
            (c.hw_interval_secs, c.limits_interval_secs)
        };
        assert_eq!(f("", ""), (3, 120));
        assert_eq!(f("0", "0"), (1, 60));
        assert_eq!(f("99999", "99999"), (5, 3600));
        assert_eq!(f(" 4 ", "abc"), (4, 120));
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
