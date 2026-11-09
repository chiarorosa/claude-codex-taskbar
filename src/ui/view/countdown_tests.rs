use super::*;

const NOW: i64 = 1_800_000_000;
const DURATIONS: &[(i64, &str, &str)] = &[
    (1, "1m", "1min"),
    (59, "1m", "1min"),
    (60, "1m", "1min"),
    (61, "2m", "2min"),
    (2700, "45m", "45min"),
    (3599, "1h", "1h"),
    (3600, "1h", "1h"),
    (3601, "1h 1m", "1h 1min"),
    (4320, "1h 12m", "1h 12min"),
    (18_000, "5h", "5h"),
    (86_340, "23h 59m", "23h 59min"),
    (86_399, "1d", "1d"),
    (86_400, "1d", "1d"),
    (86_401, "1d", "1d"),
    (183_600, "2d 3h", "2d 3h"),
    (183_660, "2d 3h", "2d 3h"),
    (604_800, "7d", "7d"),
];

fn timed_usage() -> ProviderUsage {
    ProviderUsage {
        five_hour: Some(WindowUsage {
            used_pct: 0.0,
            resets_at: Some(NOW + 4320),
        }),
        weekly: Some(WindowUsage {
            used_pct: 100.0,
            resets_at: Some(NOW + 604_800),
        }),
        ..Default::default()
    }
}

fn countdown_tip(lang: Lang, status: &ProviderStatus, now: i64) -> String {
    tooltip_text(lang, status, status, &[], TaskbarHints::default(), now)
}

fn model_window(model: ModelKind, minutes: i64) -> (ModelKind, WindowUsage) {
    (
        model,
        WindowUsage {
            used_pct: 30.0,
            resets_at: Some(NOW + minutes * 60),
        },
    )
}

fn retained_tip_expected(lang: Lang, notice: Key) -> String {
    let at = if notice == Key::SessionDataFmt {
        format_local_day_hhmm(NOW - 60)
    } else {
        format_local_hhmm(NOW - 60)
    };
    let five = fmt(t(lang, Key::ResetsFmt), &format_local_hhmm(NOW + 4320));
    let week = fmt(
        t(lang, Key::ResetsFmt),
        &format_local_day_hhmm(NOW + 604_800),
    );
    let (label, five_countdown, week_countdown) = match lang {
        Lang::En => ("wk", "in 1h 12m", "in 7d"),
        Lang::PtBr => ("sem", "em 1h 12min", "em 7d"),
    };
    let notice = if notice == Key::StaleReasonFmt {
        fmt2(t(lang, notice), "offline", &at)
    } else {
        fmt(t(lang, notice), &at)
    };
    let block = format!(
        "\n  5h: 0% · {five} ({five_countdown})\n  {label}: 100% · {week} ({week_countdown})\n  {notice}"
    );
    format!("Claude{block}\n\nCodex{block}")
}

#[test]
fn countdown_rounds_up_minutes_in_both_languages() {
    for &(seconds, en, pt) in DURATIONS {
        assert_eq!(
            reset_countdown(Lang::En, NOW + seconds, NOW),
            Some(format!("in {en}")),
            "{seconds} seconds"
        );
        assert_eq!(
            reset_countdown(Lang::PtBr, NOW + seconds, NOW),
            Some(format!("em {pt}")),
            "{seconds} seconds"
        );
    }
}

#[test]
fn countdown_omits_elapsed_or_unrepresentable_intervals() {
    for (reset, now) in [(NOW, NOW), (NOW - 1, NOW), (i64::MAX, -1), (i64::MIN, 1)] {
        assert_eq!(reset_countdown(Lang::En, reset, now), None);
        assert_eq!(reset_countdown(Lang::PtBr, reset, now), None);
    }
}

#[test]
fn tooltip_keeps_absolute_times_and_adds_both_provider_countdowns() {
    let status = ProviderStatus::Ok {
        usage: timed_usage(),
        at: NOW - 60,
    };
    for (lang, week, five, weekly) in [
        (Lang::En, "wk", "in 1h 12m", "in 7d"),
        (Lang::PtBr, "sem", "em 1h 12min", "em 7d"),
    ] {
        let tip = countdown_tip(lang, &status, NOW);
        let absolute5 = fmt(t(lang, Key::ResetsFmt), &format_local_hhmm(NOW + 4320));
        let absolute_week = fmt(
            t(lang, Key::ResetsFmt),
            &format_local_day_hhmm(NOW + 604_800),
        );
        let five_line = format!("\n  5h: 0% · {absolute5} ({five})\n");
        let week_line = format!("\n  {week}: 100% · {absolute_week} ({weekly})\n");
        assert_eq!(tip.matches(&five_line).count(), 2, "{tip}");
        assert_eq!(tip.matches(&week_line).count(), 2, "{tip}");
    }
}

#[test]
fn tooltip_recomputes_countdown_at_display_time() {
    let status = ProviderStatus::Ok {
        usage: timed_usage(),
        at: NOW - 300,
    };
    assert!(countdown_tip(Lang::En, &status, NOW).contains("(in 1h 12m)"));
    assert!(countdown_tip(Lang::En, &status, NOW + 60).contains("(in 1h 11m)"));
}

#[test]
fn model_countdowns_use_their_own_reset_times() {
    let status = ProviderStatus::Ok {
        usage: ProviderUsage {
            weekly_by_model: vec![
                model_window(ModelKind::Opus, 72),
                model_window(ModelKind::Sonnet, 5),
            ],
            ..Default::default()
        },
        at: NOW,
    };
    for (lang, opus, sonnet, first, second) in [
        (Lang::En, "Opus wk", "Sonnet wk", "in 1h 12m", "in 5m"),
        (
            Lang::PtBr,
            "Opus sem",
            "Sonnet sem",
            "em 1h 12min",
            "em 5min",
        ),
    ] {
        let tip = countdown_tip(lang, &status, NOW);
        assert!(
            tip.lines()
                .any(|line| line.starts_with(&format!("  {opus}: 30%"))
                    && line.ends_with(&format!("({first})"))),
            "{tip}"
        );
        assert!(
            tip.lines()
                .any(|line| line.starts_with(&format!("  {sonnet}: 30%"))
                    && line.ends_with(&format!("({second})"))),
            "{tip}"
        );
    }
}

#[test]
fn tooltip_without_reset_keeps_percentage_without_countdown() {
    let status = ProviderStatus::Ok {
        usage: ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: 0.0,
                resets_at: None,
            }),
            ..Default::default()
        },
        at: NOW,
    };
    let tip = countdown_tip(Lang::En, &status, NOW);
    assert_eq!(tip.matches("\n  5h: 0%\n").count(), 2, "{tip}");
    assert!(!tip.contains("resets"), "{tip}");
}

#[test]
fn tooltip_omits_countdown_for_elapsed_resets() {
    for reset in [NOW, NOW - 1] {
        let window = WindowUsage {
            used_pct: 30.0,
            resets_at: Some(reset),
        };
        let mut line = String::new();
        window_line(&mut line, Lang::En, "5h", Some(&window), false, NOW);
        let absolute = fmt(t(Lang::En, Key::ResetsFmt), &format_local_hhmm(reset));
        assert_eq!(line, format!("\n  5h: 30% · {absolute}"));
    }
}

#[test]
fn retained_statuses_keep_notices_and_known_future_reset_countdowns() {
    for (status, en, pt, notice) in [
        (
            ProviderStatus::Stale {
                usage: timed_usage(),
                at: NOW - 60,
                err: FetchError::Network("offline".into()),
            },
            "Update failed (offline); last data at",
            "Falha ao atualizar (offline); último dado às",
            Key::StaleReasonFmt,
        ),
        (
            ProviderStatus::FromSession {
                usage: timed_usage(),
                at: NOW - 60,
            },
            "(last session)",
            "(última sessão)",
            Key::SessionDataFmt,
        ),
    ] {
        let tip_en = countdown_tip(Lang::En, &status, NOW);
        let tip_pt = countdown_tip(Lang::PtBr, &status, NOW);
        assert!(
            tip_en.contains(en) && tip_en.contains("(in 1h 12m)"),
            "{tip_en}"
        );
        assert!(
            tip_pt.contains(pt) && tip_pt.contains("(em 1h 12min)"),
            "{tip_pt}"
        );
        assert_eq!(tip_en, retained_tip_expected(Lang::En, notice));
        assert_eq!(tip_pt, retained_tip_expected(Lang::PtBr, notice));
    }
}

#[test]
fn tooltip_never_invents_countdown_without_usage() {
    for status in [
        ProviderStatus::Loading,
        ProviderStatus::Disabled,
        ProviderStatus::Failed(FetchError::Expired),
        ProviderStatus::Ok {
            usage: ProviderUsage::default(),
            at: NOW,
        },
    ] {
        for lang in [Lang::En, Lang::PtBr] {
            let tip = countdown_tip(lang, &status, NOW);
            assert!(!tip.contains("(in ") && !tip.contains("(em "), "{tip}");
            assert!(!tip.contains('%'), "{tip}");
        }
    }
}
