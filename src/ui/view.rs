//! Turns app state into the list of strip cells and the tooltip text.

use crate::config::Config;
use crate::hw::HwSnapshot;
use crate::i18n::{Key, Lang, fmt, fmt2, t};
use crate::limits::{
    FIVE_HOUR_SECS, FetchError, ModelKind, ProviderStatus, ProviderUsage, WEEK_SECS, WindowUsage,
    elapsed_fraction,
};
use crate::model::{ItemId, Kind};
use crate::timeutil::{format_local_day_hhmm, format_local_hhmm};
use crate::ui::taskbar::Side;
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
    /// Text only: bar geometry and colors still use `value`.
    pub show_remaining: bool,
    /// Account label drawn before this item, in the provider's color (#74). Set on the
    /// first visible item of each provider so it hides together with that provider.
    pub badge: Option<String>,
    /// Elapsed fraction (`0.0..=1.0`) of the window for the pace tick on the bar (#14).
    /// `None` for hardware, error/stale/session states, or when `resets_at` is absent/expired.
    pub tick: Option<f32>,
}

/// Taskbar facts that decide which setup hints apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskbarHints {
    pub left_aligned: bool,
    pub widgets_button_on: bool,
    pub windows_10: bool,
    /// Where the strip sits; next to the tray it needs no setup hints.
    pub side: Side,
}

impl ItemView {
    pub fn text(&self) -> String {
        value_text(
            self.value,
            self.show_remaining && self.kind != Kind::Hardware,
        )
    }
}

pub fn value_text(value: Option<f32>, remaining: bool) -> String {
    match value {
        Some(v) => format!("{}%", fmt_round(if remaining { 100.0 - v } else { v })),
        None => "—".into(),
    }
}

/// `format!("{:.0}", v)` without the standard library's float formatter (#83): the same text
/// for every `f32`. Finite values fit a `u128` (`f32::MAX` < 2^128), the fraction test is
/// exact in `f64`, and a tie rounds to even, as `{:.0}` does.
pub fn fmt_round(v: f32) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    let sign = if v.is_sign_negative() { "-" } else { "" };
    if v.is_infinite() {
        return format!("{sign}inf");
    }
    let x = f64::from(v).abs();
    let whole = x.floor();
    let frac = x - whole;
    let mut n = whole as u128;
    if frac > 0.5 || (frac == 0.5 && n % 2 == 1) {
        n += 1;
    }
    format!("{sign}{n}")
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

fn window_tick(
    status: &ProviderStatus,
    w: Option<&WindowUsage>,
    window_len: i64,
    now: i64,
) -> Option<f32> {
    if !matches!(status, ProviderStatus::Ok { .. }) {
        return None;
    }
    let f = elapsed_fraction(w?.resets_at, window_len, now)?;
    Some(((f * 1000.0).round() / 1000.0) as f32)
}

/// `claude_label` is the live Claude account's label (it depends on the saved accounts);
/// the Codex label comes from `cfg`.
pub fn build_items(
    cfg: &Config,
    hw: &HwSnapshot,
    claude: &ProviderStatus,
    codex: &ProviderStatus,
    claude_label: &str,
    lang: Lang,
    now: i64,
) -> Vec<ItemView> {
    let mut items = Vec::new();
    let mut push = |id: ItemId,
                    kind: Kind,
                    label: String,
                    value: Option<f32>,
                    dim: bool,
                    tick: Option<f32>| {
        if cfg.is_visible(&id) {
            items.push(ItemView {
                id,
                kind,
                label,
                value,
                dim,
                show_remaining: cfg.show_remaining,
                badge: None,
                tick,
            });
        }
    };
    push(
        ItemId::Cpu,
        Kind::Hardware,
        "CPU".into(),
        hw.cpu,
        false,
        None,
    );
    push(
        ItemId::Ram,
        Kind::Hardware,
        "RAM".into(),
        hw.ram,
        false,
        None,
    );
    push(
        ItemId::Gpu,
        Kind::Hardware,
        "GPU".into(),
        hw.gpu,
        false,
        None,
    );
    for d in &hw.disks {
        let id = ItemId::Disk(d.instance.clone());
        push(
            id,
            Kind::Hardware,
            d.label.clone(),
            Some(d.active_pct),
            false,
            None,
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
        let win =
            |w: fn(&ProviderUsage) -> &Option<WindowUsage>| usage.and_then(|(u, _)| w(u).as_ref());
        let five = win(|u| &u.five_hour);
        let week = win(|u| &u.weekly);
        push(
            five_id,
            kind,
            five_label.into(),
            five.map(|w| w.used_pct),
            dim,
            window_tick(status, five, FIVE_HOUR_SECS, now),
        );
        push(
            week_id,
            kind,
            t(lang, Key::LabelWeek).into(),
            week.map(|w| w.used_pct),
            dim,
            window_tick(status, week, WEEK_SECS, now),
        );
    }
    for (kind, label) in [
        (Kind::Claude, claude_label),
        (Kind::Codex, cfg.codex_label.as_str()),
    ] {
        if let Some(first) = items.iter_mut().find(|i| i.kind == kind)
            && !label.is_empty()
        {
            first.badge = Some(label.to_string());
        }
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
        (_, _) => match error_reason(lang, err) {
            Some(reason) => return format!("{} ({reason})", t(lang, Key::NoRecentData)),
            None => Key::NoRecentData,
        },
    };
    t(lang, key).into()
}

/// Why a transient fetch failed, for the tooltip; `None` when there is nothing to add.
fn error_reason(lang: Lang, err: &FetchError) -> Option<String> {
    match err {
        FetchError::Network(msg) | FetchError::Offline(msg) | FetchError::BadResponse(msg)
            if !msg.is_empty() =>
        {
            Some(msg.clone())
        }
        FetchError::RateLimited(Some(at)) => Some(fmt(
            t(lang, Key::RateLimitedNextFmt),
            &format_local_hhmm(*at),
        )),
        FetchError::RateLimited(None) => Some(t(lang, Key::RateLimited).into()),
        _ => None,
    }
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
    remaining: bool,
) {
    let Some(w) = w else { return };
    let _ = write!(
        out,
        "\n  {label}: {}",
        value_text(Some(w.used_pct), remaining)
    );
    if remaining {
        let _ = write!(out, " {}", t(lang, Key::Remaining));
    }
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

fn model_usage_lines(
    out: &mut String,
    lang: Lang,
    usage: &ProviderUsage,
    now: i64,
    remaining: bool,
) {
    for (model, window) in &usage.weekly_by_model {
        let key = match model {
            ModelKind::Opus => Key::LabelOpusWeek,
            ModelKind::Sonnet => Key::LabelSonnetWeek,
        };
        window_line(out, lang, t(lang, key), Some(window), true, now, remaining);
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
    remaining: bool,
) {
    let week = t(lang, Key::LabelWeek);
    let usage_lines = |out: &mut String, u: &ProviderUsage| {
        window_line(out, lang, "5h", u.five_hour.as_ref(), false, now, remaining);
        window_line(out, lang, week, u.weekly.as_ref(), true, now, remaining);
        model_usage_lines(out, lang, u, now, remaining);
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
            let notice = match error_reason(lang, err) {
                Some(reason) => fmt2(t(lang, Key::StaleReasonFmt), &reason, &at),
                None => fmt(t(lang, Key::StaleFmt), &at),
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
    remaining: bool,
) -> String {
    let mut out = String::new();
    for (name, kind, status) in [
        ("Claude", Kind::Claude, claude),
        ("Codex", Kind::Codex, codex),
    ] {
        if *status != ProviderStatus::Disabled {
            provider_block(&mut out, lang, name, kind, status, now, remaining);
        }
    }
    if !hidden.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(t(lang, Key::HiddenItems));
        for item in hidden {
            let _ = write!(out, "\n  {} {}", item.label, item.text());
            if item.show_remaining && item.kind != Kind::Hardware && item.value.is_some() {
                let _ = write!(out, " {}", t(lang, Key::Remaining));
            }
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
/// Windows 10 has no centered taskbar and no Widgets button, so it gets no hints.
pub fn setup_hints(hints: TaskbarHints) -> Vec<Key> {
    if hints.side == Side::Right {
        return Vec::new();
    }
    if hints.windows_10 {
        return Vec::new();
    }
    [
        (hints.left_aligned, Key::HintCenterTaskbar),
        (hints.widgets_button_on, Key::HintWidgetsButton),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_limits_stay_missing_in_both_modes() {
        for remaining in [false, true] {
            let cfg = Config {
                show_remaining: remaining,
                ..Config::default()
            };
            for status in [
                ProviderStatus::Loading,
                ProviderStatus::Failed(FetchError::Expired),
                ProviderStatus::Failed(FetchError::Network(String::new())),
                ProviderStatus::Ok {
                    usage: ProviderUsage::default(),
                    at: 0,
                },
            ] {
                let items = build_items(&cfg, &hw(), &status, &status, "", Lang::En, 0);
                assert!(
                    items
                        .iter()
                        .filter(|i| i.kind != Kind::Hardware)
                        .all(|i| i.text() == "—")
                );
                let tip = tooltip_text(
                    Lang::En,
                    &status,
                    &status,
                    &items,
                    TaskbarHints::default(),
                    0,
                    remaining,
                );
                assert!(
                    !tip.contains("5h 0%") && !tip.contains("wk 0%") && !tip.contains("0% left"),
                    "{tip}"
                );
            }
        }
    }

    #[test]
    fn fmt_round_matches_the_standard_formatter_for_every_kind_of_f32() {
        let check = |v: f32| {
            assert_eq!(
                fmt_round(v),
                format!("{v:.0}"),
                "{v:?} ({:#x})",
                v.to_bits()
            )
        };
        // Ties round to even, both signs; -0 keeps its sign like `{:.0}`.
        for k in -2000..=2000 {
            check(k as f32 / 2.0);
        }
        for v in [
            0.0,
            -0.0,
            0.49999997,
            0.5,
            0.50000006,
            99.5,
            100.5,
            62.4,
            37.6,
            1e-45,
            16_777_215.5,
            16_777_216.0,
            3.0e38,
            f32::MAX,
            f32::MIN,
            f32::MIN_POSITIVE,
            f32::EPSILON,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            check(v);
        }
        // Every percentage a provider can send, at 1/1024 steps, and a spread of bit patterns.
        for k in 0..=110 * 1024 {
            check(k as f32 / 1024.0);
        }
        for bits in (0..=u32::MAX).step_by(4099) {
            check(f32::from_bits(bits));
        }
    }

    #[test]
    fn remaining_formatting() {
        for (used, expected) in [(62.0, "38%"), (0.0, "100%"), (100.0, "0%")] {
            assert_eq!(value_text(Some(used), true), expected);
            assert_eq!(value_text(Some(used), false), format!("{used:.0}%"));
        }
        for remaining in [false, true] {
            assert_eq!(value_text(None, remaining), "—");
        }
    }

    #[test]
    fn remaining_tooltips_and_strip_leave_hardware_and_usage_intact() {
        for (lang, expected) in [(Lang::En, "38% left"), (Lang::PtBr, "38% restante")] {
            for remaining in [false, true] {
                let cfg = Config {
                    show_remaining: remaining,
                    ..Config::default()
                };
                let items = build_items(&cfg, &hw(), &ok(62.0, 100.0), &ok(0.0, 62.0), "", lang, 0);
                assert_eq!(items[0].text(), "23%");
                assert_eq!(items[4].value, Some(62.0));
                assert_eq!(items[4].text(), if remaining { "38%" } else { "62%" });
                let tip = tooltip_text(
                    lang,
                    &ok(62.0, 100.0),
                    &ok(0.0, 62.0),
                    &items,
                    TaskbarHints::default(),
                    0,
                    remaining,
                );
                assert!(
                    tip.contains(if remaining { expected } else { "62%" }),
                    "{tip}"
                );
                assert!(tip.contains("CPU 23%"), "{tip}");
                if remaining {
                    assert!(tip.contains(&format!("Claude 5h {expected}")), "{tip}");
                }
            }
        }
    }
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

    fn ok(five: f32, week: f32) -> ProviderStatus {
        ProviderStatus::Ok {
            usage: usage(five, week),
            at: 0,
        }
    }

    fn badges(items: &[ItemView]) -> Vec<(ItemId, Option<&str>)> {
        items
            .iter()
            .map(|i| (i.id.clone(), i.badge.as_deref()))
            .filter(|(id, _)| {
                !matches!(
                    id,
                    ItemId::Cpu | ItemId::Ram | ItemId::Gpu | ItemId::Disk(_)
                )
            })
            .collect()
    }

    #[test]
    fn each_provider_label_sits_on_its_first_item() {
        let cfg = Config {
            codex_label: "W".into(),
            ..Config::default()
        };
        let items = build_items(
            &cfg,
            &hw(),
            &ok(1.0, 2.0),
            &ok(3.0, 4.0),
            "Pess",
            Lang::En,
            0,
        );
        assert_eq!(
            badges(&items),
            [
                (ItemId::ClaudeFive, Some("Pess")),
                (ItemId::ClaudeWeek, None),
                (ItemId::CodexFive, Some("W")),
                (ItemId::CodexWeek, None),
            ]
        );
        assert!(
            items
                .iter()
                .filter(|i| i.kind == Kind::Hardware)
                .all(|i| i.badge.is_none())
        );
    }

    #[test]
    fn the_label_moves_to_the_weekly_item_when_5h_is_hidden() {
        let mut cfg = Config::default();
        cfg.set_visible(&ItemId::ClaudeFive, false);
        let items = build_items(
            &cfg,
            &hw(),
            &ok(1.0, 2.0),
            &ProviderStatus::Disabled,
            "P",
            Lang::En,
            0,
        );
        assert_eq!(badges(&items), [(ItemId::ClaudeWeek, Some("P"))]);
    }

    #[test]
    fn an_empty_label_shows_no_badge() {
        let cfg = Config {
            codex_label: String::new(),
            ..Config::default()
        };
        let items = build_items(&cfg, &hw(), &ok(1.0, 2.0), &ok(3.0, 4.0), "", Lang::En, 0);
        assert!(items.iter().all(|i| i.badge.is_none()));
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
            "P",
            Lang::PtBr,
            0,
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
        let items = build_items(
            &cfg,
            &hw(),
            &ProviderStatus::Disabled,
            &codex,
            "P",
            Lang::En,
            0,
        );
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
        let items = build_items(&Config::default(), &hw(), &claude, &codex, "P", Lang::En, 0);
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
            false,
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
    fn offline_errors_read_exactly_like_network_errors() {
        let offline = FetchError::Offline("host not found".into());
        let network = FetchError::Network("host not found".into());
        for lang in Lang::ALL {
            assert_eq!(
                stale_tip(lang, offline.clone()),
                stale_tip(lang, network.clone())
            );
            for kind in [Kind::Claude, Kind::Codex] {
                assert_eq!(
                    error_message(lang, kind, &offline),
                    error_message(lang, kind, &network)
                );
            }
        }
    }

    #[test]
    fn rate_limit_shows_when_the_next_try_is() {
        let next = format_local_hhmm(1900);
        let at = format_local_hhmm(1000);
        let limited = || FetchError::RateLimited(Some(1900));
        assert!(stale_tip(Lang::En, limited()).contains(&format!(
            "Update failed (rate limited, next try {next}); last data at {at}"
        )));
        assert!(stale_tip(Lang::PtBr, limited()).contains(&format!(
            "Falha ao atualizar (limite de requisições, próxima tentativa às {next}); último dado às {at}"
        )));
        assert_eq!(
            error_message(Lang::En, Kind::Claude, &limited()),
            format!("No recent data (rate limited, next try {next})")
        );
        assert_eq!(
            error_message(Lang::PtBr, Kind::Codex, &FetchError::RateLimited(None)),
            "Sem dado recente (limite de requisições)"
        );
    }

    #[test]
    fn stale_tooltip_without_a_reason_keeps_the_plain_notice() {
        let tip = stale_tip(Lang::En, FetchError::Network(String::new()));
        assert!(tip.contains("Update failed; last data at"));
        assert!(!tip.contains("()"));
    }

    #[test]
    fn value_text_never_shows_zero_for_errors() {
        assert_eq!(value_text(None, false), "—");
        assert_eq!(value_text(Some(61.6), false), "62%");
        assert_eq!(value_text(Some(0.0), false), "0%");
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
            false,
        );
        assert!(tip.contains("Claude login expired — open Claude Code to renew"));
        assert!(tip.contains("Log in by running `codex login`"));
    }

    #[test]
    fn tooltip_explains_claude_cli_login_missing() {
        for (lang, expected) in [
            (
                Lang::En,
                "Claude Code CLI login not found — desktop app logins aren't supported; run `claude` once to fix it",
            ),
            (
                Lang::PtBr,
                "Login do Claude Code CLI não encontrado — logins do app desktop não são suportados; rode `claude` uma vez para resolver",
            ),
            (
                Lang::Es,
                "No se encontró el inicio de sesión de la CLI de Claude Code — los inicios de sesión de la app de escritorio no son compatibles; ejecuta `claude` una vez para solucionarlo",
            ),
        ] {
            let tip = tooltip_text(
                lang,
                &ProviderStatus::Failed(FetchError::NoLogin),
                &ProviderStatus::Disabled,
                &[],
                TaskbarHints::default(),
                0,
                false,
            );
            assert!(tip.contains(expected));
        }
    }

    #[test]
    fn tooltip_skips_disabled_and_lists_hidden_items_and_hints() {
        let hidden = vec![ItemView {
            id: ItemId::Gpu,
            kind: Kind::Hardware,
            label: "GPU".into(),
            value: Some(12.0),
            dim: false,
            show_remaining: false,
            badge: None,
            tick: None,
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
            false,
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
            false,
        );
        assert!(tip.contains("Claude"));
        assert!(tip.contains("5h: 62%"));
        assert!(tip.contains("wk: 34%"));
    }

    #[test]
    fn tick_marks_elapsed_window_fraction_only_on_fresh_ok_with_future_reset() {
        let now = 1_800_000_000;
        let timed = ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: 60.0,
                resets_at: Some(now + FIVE_HOUR_SECS / 2),
            }),
            weekly: Some(WindowUsage {
                used_pct: 93.0,
                resets_at: Some(now + WEEK_SECS * 3 / 4),
            }),
            ..Default::default()
        };
        let ok_status = ProviderStatus::Ok {
            usage: timed.clone(),
            at: now,
        };
        let items = build_items(
            &Config::default(),
            &hw(),
            &ok_status,
            &ProviderStatus::Disabled,
            "P",
            Lang::En,
            now,
        );
        assert!(
            items
                .iter()
                .filter(|i| i.kind == Kind::Hardware)
                .all(|i| i.tick.is_none())
        );
        let five = items.iter().find(|i| i.id == ItemId::ClaudeFive).unwrap();
        let week = items.iter().find(|i| i.id == ItemId::ClaudeWeek).unwrap();
        assert_eq!(five.tick, Some(0.5));
        assert_eq!(week.tick, Some(0.25));

        // Missing resets_at, past resets_at, Stale, FromSession, Loading, Failed -> no tick.
        for non_tick_status in [
            ok(60.0, 93.0),
            ProviderStatus::Ok {
                usage: ProviderUsage {
                    five_hour: Some(WindowUsage {
                        used_pct: 60.0,
                        resets_at: Some(now),
                    }),
                    weekly: Some(WindowUsage {
                        used_pct: 93.0,
                        resets_at: Some(now - 10),
                    }),
                    ..Default::default()
                },
                at: now,
            },
            ProviderStatus::Stale {
                usage: timed.clone(),
                at: now,
                err: FetchError::Network("timeout".into()),
            },
            ProviderStatus::FromSession {
                usage: timed,
                at: now,
            },
            ProviderStatus::Loading,
            ProviderStatus::Failed(FetchError::Expired),
        ] {
            let items = build_items(
                &Config::default(),
                &hw(),
                &non_tick_status,
                &ProviderStatus::Disabled,
                "P",
                Lang::En,
                now,
            );
            assert!(
                items.iter().all(|i| i.tick.is_none()),
                "unexpected tick for {non_tick_status:?}: {items:?}"
            );
        }
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
            false,
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
    fn right_side_has_no_setup_hints() {
        assert_eq!(
            setup_hints(TaskbarHints {
                left_aligned: true,
                windows_10: true,
                widgets_button_on: true,
                side: Side::Right,
            }),
            Vec::<Key>::new()
        );
    }

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
            "Windows 10 gets no centering or Widgets hints"
        );
        assert_eq!(
            setup_hints(TaskbarHints {
                windows_10: true,
                widgets_button_on: true,
                ..Default::default()
            }),
            Vec::<Key>::new(),
            "Windows 10 has no Widgets button to turn off"
        );
    }
}

#[cfg(test)]
mod claude_tests;

#[cfg(test)]
mod countdown_tests;
