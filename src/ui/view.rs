//! Turns app state into the list of strip cells and the tooltip text.

use crate::config::Config;
use crate::hw::HwSnapshot;
use crate::i18n::{Key, Lang, fmt, fmt2, t};
use crate::limits::{FetchError, ModelKind, ProviderStatus, ProviderUsage, WindowUsage};
use crate::model::{ItemId, Kind};
use crate::timeutil::{format_local_day_hhmm, format_local_hhmm};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
    pub id: ItemId,
    pub kind: Kind,
    pub label: String,
    /// `None` renders as `—` with a grey bar.
    pub value: Option<f32>,
    /// Data is old (stale or from session logs): bar drawn at half opacity.
    pub dim: bool,
}

/// Taskbar facts that decide which setup hints apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskbarHints {
    pub left_aligned: bool,
    pub widgets_button_on: bool,
    pub windows_10: bool,
}

pub fn value_text(value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{:.0}%", v),
        None => "—".into(),
    }
}

fn usage_of(status: &ProviderStatus) -> Option<(&ProviderUsage, bool)> {
    match status {
        ProviderStatus::Ok { usage, .. } => Some((usage, false)),
        ProviderStatus::Stale { usage, .. } | ProviderStatus::FromSession { usage, .. } => {
            Some((usage, true))
        }
        _ => None,
    }
}

pub fn build_items(
    cfg: &Config,
    hw: &HwSnapshot,
    claude: &ProviderStatus,
    codex: &ProviderStatus,
    lang: Lang,
) -> Vec<ItemView> {
    let mut items = Vec::new();
    let mut push = |id: ItemId, kind: Kind, label: String, value: Option<f32>, dim: bool| {
        if cfg.is_visible(&id) {
            items.push(ItemView {
                id,
                kind,
                label,
                value,
                dim,
            });
        }
    };
    push(ItemId::Cpu, Kind::Hardware, "CPU".into(), hw.cpu, false);
    push(ItemId::Ram, Kind::Hardware, "RAM".into(), hw.ram, false);
    push(ItemId::Gpu, Kind::Hardware, "GPU".into(), hw.gpu, false);
    for d in &hw.disks {
        let id = ItemId::Disk(d.instance.clone());
        push(
            id,
            Kind::Hardware,
            d.label.clone(),
            Some(d.active_pct),
            false,
        );
    }
    let providers = [
        (
            claude,
            Kind::Claude,
            "Claude 5h",
            ItemId::ClaudeFive,
            ItemId::ClaudeWeek,
        ),
        (
            codex,
            Kind::Codex,
            "Codex 5h",
            ItemId::CodexFive,
            ItemId::CodexWeek,
        ),
    ];
    for (status, kind, five_label, five_id, week_id) in providers {
        if *status == ProviderStatus::Disabled {
            continue;
        }
        let usage = usage_of(status);
        let dim = usage.is_some_and(|(_, dim)| dim);
        let pct = |w: fn(&ProviderUsage) -> &Option<WindowUsage>| {
            usage.and_then(|(u, _)| w(u).as_ref()).map(|w| w.used_pct)
        };
        push(five_id, kind, five_label.into(), pct(|u| &u.five_hour), dim);
        push(
            week_id,
            kind,
            t(lang, Key::LabelWeek).into(),
            pct(|u| &u.weekly),
            dim,
        );
    }
    items
}

fn error_message(lang: Lang, kind: Kind, err: &FetchError) -> String {
    let key = match (kind, err) {
        (Kind::Claude, FetchError::Expired) => Key::ClaudeExpired,
        (_, FetchError::Expired) => Key::CodexExpired,
        (Kind::Claude, FetchError::NoLogin) => Key::ClaudeNoLogin,
        (_, FetchError::NoLogin) => Key::CodexNoLogin,
        (_, FetchError::NoSubscription) => Key::NoSubscription,
        (_, FetchError::Network(msg) | FetchError::BadResponse(msg)) if msg.is_empty() => {
            Key::NoRecentData
        }
        (_, FetchError::Network(msg) | FetchError::BadResponse(msg)) => {
            return format!("{} ({msg})", t(lang, Key::NoRecentData));
        }
    };
    t(lang, key).into()
}

// Round up so a reset less than a minute away never looks already due (#14).
fn reset_countdown(lang: Lang, reset: i64, now: i64) -> Option<String> {
    let seconds = reset.checked_sub(now)?;
    if seconds <= 0 {
        return None;
    }
    let minutes = seconds / 60 + i64::from(seconds % 60 != 0);
    let (hours, minutes) = (minutes / 60, minutes % 60);
    let (key, first, second) = match hours {
        0 => return Some(fmt(t(lang, Key::ResetsInMinutesFmt), &minutes.to_string())),
        1..=23 if minutes == 0 => (Key::ResetsInHoursOnlyFmt, hours, 0),
        1..=23 => (Key::ResetsInHoursFmt, hours, minutes),
        _ if hours % 24 == 0 => (Key::ResetsInDaysOnlyFmt, hours / 24, 0),
        _ => (Key::ResetsInDaysFmt, hours / 24, hours % 24),
    };
    let text = fmt(t(lang, key), &first.to_string());
    Some(fmt(&text, &second.to_string()))
}

fn window_line(
    out: &mut String,
    lang: Lang,
    label: &str,
    w: Option<&WindowUsage>,
    weekly: bool,
    now: i64,
) {
    let Some(w) = w else { return };
    let _ = write!(out, "\n  {label}: {}", value_text(Some(w.used_pct)));
    if let Some(reset) = w.resets_at {
        let when = if weekly {
            format_local_day_hhmm(reset)
        } else {
            format_local_hhmm(reset)
        };
        let _ = write!(out, " · {}", fmt(t(lang, Key::ResetsFmt), &when));
        if let Some(countdown) = reset_countdown(lang, reset, now) {
            let _ = write!(out, " ({countdown})");
        }
    }
}

fn model_usage_lines(out: &mut String, lang: Lang, usage: &ProviderUsage, now: i64) {
    for (model, window) in &usage.weekly_by_model {
        let key = match model {
            ModelKind::Opus => Key::LabelOpusWeek,
            ModelKind::Sonnet => Key::LabelSonnetWeek,
        };
        window_line(out, lang, t(lang, key), Some(window), true, now);
    }
    if usage.extra_usage_enabled {
        let _ = write!(out, "\n  {}", t(lang, Key::ExtraUsageEnabled));
    }
}

fn provider_block(
    out: &mut String,
    lang: Lang,
    name: &str,
    kind: Kind,
    status: &ProviderStatus,
    now: i64,
) {
    let week = t(lang, Key::LabelWeek);
    let usage_lines = |out: &mut String, u: &ProviderUsage| {
        window_line(out, lang, "5h", u.five_hour.as_ref(), false, now);
        window_line(out, lang, week, u.weekly.as_ref(), true, now);
        model_usage_lines(out, lang, u, now);
    };
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(name);
    match status {
        ProviderStatus::Disabled => {}
        ProviderStatus::Loading => {
            let _ = write!(out, "\n  {}", t(lang, Key::Loading));
        }
        ProviderStatus::Ok { usage, at } => {
            usage_lines(out, usage);
            let _ = write!(
                out,
                "\n  {}",
                fmt(t(lang, Key::UpdatedFmt), &format_local_hhmm(*at))
            );
        }
        ProviderStatus::Stale { usage, at, err } => {
            usage_lines(out, usage);
            let at = format_local_hhmm(*at);
            let notice = match err {
                FetchError::Network(msg) | FetchError::BadResponse(msg) if !msg.is_empty() => {
                    fmt2(t(lang, Key::StaleReasonFmt), msg, &at)
                }
                _ => fmt(t(lang, Key::StaleFmt), &at),
            };
            let _ = write!(out, "\n  {notice}");
        }
        ProviderStatus::FromSession { usage, at } => {
            usage_lines(out, usage);
            let when = format_local_day_hhmm(*at);
            let _ = write!(out, "\n  {}", fmt(t(lang, Key::SessionDataFmt), &when));
        }
        ProviderStatus::Failed(err) => {
            let _ = write!(out, "\n  {}", error_message(lang, kind, err));
        }
    }
}

/// Multi-line tooltip: per-provider details, items cut for space, and setup hints.
pub fn tooltip_text(
    lang: Lang,
    claude: &ProviderStatus,
    codex: &ProviderStatus,
    hidden: &[ItemView],
    hints: TaskbarHints,
    now: i64,
) -> String {
    let mut out = String::new();
    for (name, kind, status) in [
        ("Claude", Kind::Claude, claude),
        ("Codex", Kind::Codex, codex),
    ] {
        if *status != ProviderStatus::Disabled {
            provider_block(&mut out, lang, name, kind, status, now);
        }
    }
    if !hidden.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(t(lang, Key::HiddenItems));
        for item in hidden {
            let _ = write!(out, "\n  {} {}", item.label, value_text(item.value));
        }
    }
    for key in setup_hints(hints) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(t(lang, key));
    }
    out
}

/// Taskbar settings the user should change for the strip to show up well.
/// Windows 10 has no centered taskbar and no Widgets button, so both hints are skipped.
pub fn setup_hints(hints: TaskbarHints) -> Vec<Key> {
    [
        (
            hints.left_aligned && !hints.windows_10,
            Key::HintCenterTaskbar,
        ),
        (hints.widgets_button_on, Key::HintWidgetsButton),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::hw::{DiskReading, HwSnapshot};
    use crate::i18n::Lang;
    use crate::limits::{FetchError, ProviderStatus, ProviderUsage, WindowUsage};
    use crate::model::{ItemId, Kind};

    fn hw() -> HwSnapshot {
        HwSnapshot {
            cpu: Some(23.0),
            ram: Some(61.0),
            gpu: None,
            disks: vec![DiskReading {
                instance: "0 C:".into(),
                label: "C:".into(),
                active_pct: 70.0,
            }],
        }
    }

    fn usage(five: f32, week: f32) -> ProviderUsage {
        ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: five,
                resets_at: None,
            }),
            weekly: Some(WindowUsage {
                used_pct: week,
                resets_at: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn items_in_order_with_labels() {
        let claude = ProviderStatus::Ok {
            usage: usage(62.0, 34.0),
            at: 0,
        };
        let items = build_items(
            &Config::default(),
            &hw(),
            &claude,
            &ProviderStatus::Disabled,
            Lang::PtBr,
        );
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["CPU", "RAM", "GPU", "C:", "Claude 5h", "sem"]);
        assert_eq!(items[0].kind, Kind::Hardware);
        assert_eq!(items[4].value, Some(62.0));
        assert_eq!(items[5].value, Some(34.0));
        assert_eq!(items[2].value, None); // GPU counter unavailable shows "—"
    }

    #[test]
    fn hidden_items_are_skipped() {
        let mut cfg = Config::default();
        cfg.set_visible(&ItemId::Gpu, false);
        cfg.set_visible(&ItemId::CodexWeek, false);
        let codex = ProviderStatus::Ok {
            usage: usage(1.0, 2.0),
            at: 0,
        };
        let items = build_items(&cfg, &hw(), &ProviderStatus::Disabled, &codex, Lang::En);
        let ids: Vec<_> = items.iter().map(|i| i.id.clone()).collect();
        assert_eq!(
            ids,
            [
                ItemId::Cpu,
                ItemId::Ram,
                ItemId::Disk("0 C:".into()),
                ItemId::CodexFive
            ]
        );
    }

    #[test]
    fn failed_status_has_no_value_and_stale_is_dimmed() {
        let claude = ProviderStatus::Failed(FetchError::Expired);
        let codex = ProviderStatus::Stale {
            usage: usage(5.0, 6.0),
            at: 0,
            err: FetchError::Network("x".into()),
        };
        let items = build_items(&Config::default(), &hw(), &claude, &codex, Lang::En);
        let claude5 = items.iter().find(|i| i.id == ItemId::ClaudeFive).unwrap();
        assert_eq!(claude5.value, None);
        assert!(!claude5.dim);
        let codex5 = items.iter().find(|i| i.id == ItemId::CodexFive).unwrap();
        assert_eq!(codex5.value, Some(5.0));
        assert!(codex5.dim);
    }

    fn stale_tip(lang: Lang, err: FetchError) -> String {
        let status = ProviderStatus::Stale {
            usage: usage(5.0, 6.0),
            at: 1000,
            err,
        };
        tooltip_text(
            lang,
            &status,
            &ProviderStatus::Disabled,
            &[],
            TaskbarHints::default(),
            1060,
        )
    }

    #[test]
    fn stale_tooltip_says_why_the_update_failed() {
        let at = format_local_hhmm(1000);
        let err = || FetchError::BadResponse("HTTP 429".into());
        assert!(
            stale_tip(Lang::En, err())
                .contains(&format!("Update failed (HTTP 429); last data at {at}"))
        );
        assert!(stale_tip(Lang::PtBr, err()).contains(&format!(
            "Falha ao atualizar (HTTP 429); último dado às {at}"
        )));
    }

    #[test]
    fn stale_tooltip_without_a_reason_keeps_the_plain_notice() {
        let tip = stale_tip(Lang::En, FetchError::Network(String::new()));
        assert!(tip.contains("Update failed; last data at"));
        assert!(!tip.contains("()"));
    }

    #[test]
    fn value_text_never_shows_zero_for_errors() {
        assert_eq!(value_text(None), "—");
        assert_eq!(value_text(Some(61.6)), "62%");
        assert_eq!(value_text(Some(0.0)), "0%");
    }

    #[test]
    fn tooltip_explains_auth_problems() {
        let tip = tooltip_text(
            Lang::En,
            &ProviderStatus::Failed(FetchError::Expired),
            &ProviderStatus::Failed(FetchError::NoLogin),
            &[],
            TaskbarHints::default(),
            0,
        );
        assert!(tip.contains("Claude login expired — open Claude Code to renew"));
        assert!(tip.contains("Log in by running `codex login`"));
    }

    #[test]
    fn tooltip_skips_disabled_and_lists_hidden_items_and_hints() {
        let hidden = vec![ItemView {
            id: ItemId::Gpu,
            kind: Kind::Hardware,
            label: "GPU".into(),
            value: Some(12.0),
            dim: false,
        }];
        let tip = tooltip_text(
            Lang::En,
            &ProviderStatus::Disabled,
            &ProviderStatus::Disabled,
            &hidden,
            TaskbarHints {
                left_aligned: true,
                widgets_button_on: true,
                ..Default::default()
            },
            0,
        );
        assert!(!tip.contains("Claude"));
        assert!(tip.contains("Not shown (no room):"));
        assert!(tip.contains("GPU 12%"));
        assert!(tip.contains("Center the taskbar icons"));
        assert!(tip.contains("Turn off the Windows Widgets button"));
    }

    #[test]
    fn tooltip_shows_percentages_for_ok() {
        let tip = tooltip_text(
            Lang::En,
            &ProviderStatus::Ok {
                usage: usage(62.0, 34.0),
                at: 0,
            },
            &ProviderStatus::Disabled,
            &[],
            TaskbarHints::default(),
            0,
        );
        assert!(tip.contains("Claude"));
        assert!(tip.contains("5h: 62%"));
        assert!(tip.contains("wk: 34%"));
    }
}

#[cfg(test)]
mod aging_view_tests {
    use super::*;
    use crate::limits::{FetchError, ProviderStatus};

    #[test]
    fn outdated_data_message_has_no_empty_parens() {
        let tip = tooltip_text(
            Lang::En,
            &ProviderStatus::Failed(FetchError::Network(String::new())),
            &ProviderStatus::Disabled,
            &[],
            TaskbarHints::default(),
            0,
        );
        assert!(tip.contains("No recent data"));
        assert!(!tip.contains("()"));
    }
}

#[cfg(test)]
mod hint_tests {
    use super::*;
    use crate::i18n::Key;

    #[test]
    fn hints_follow_taskbar_state() {
        assert_eq!(setup_hints(TaskbarHints::default()), Vec::<Key>::new());
        assert_eq!(
            setup_hints(TaskbarHints {
                left_aligned: true,
                ..Default::default()
            }),
            [Key::HintCenterTaskbar]
        );
        assert_eq!(
            setup_hints(TaskbarHints {
                left_aligned: true,
                widgets_button_on: true,
                ..Default::default()
            }),
            [Key::HintCenterTaskbar, Key::HintWidgetsButton]
        );
        assert_eq!(
            setup_hints(TaskbarHints {
                left_aligned: true,
                windows_10: true,
                ..Default::default()
            }),
            Vec::<Key>::new(),
            "Windows 10 has no centering option and no Widgets button"
        );
    }
}

#[cfg(test)]
mod claude_tests;

#[cfg(test)]
mod countdown_tests;
