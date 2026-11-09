use super::*;
use crate::limits::{FetchError, ProviderStatus, ProviderUsage, WindowUsage};

const THRESHOLDS: [u32; 2] = [80, 95];

fn window(used_pct: f32, resets_at: Option<i64>) -> Option<WindowUsage> {
    Some(WindowUsage {
        used_pct,
        resets_at,
    })
}

fn ok(five_hour: Option<WindowUsage>, weekly: Option<WindowUsage>) -> ProviderStatus {
    ProviderStatus::Ok {
        usage: ProviderUsage {
            five_hour,
            weekly,
            ..Default::default()
        },
        at: 1,
    }
}

fn sample(five: f32, weekly: f32, reset: i64) -> ProviderStatus {
    ok(
        window(five, Some(reset)),
        window(weekly, Some(reset + 1000)),
    )
}

fn baseline(five: f32, weekly: f32, reset: i64) -> CrossingState {
    crossings(
        &CrossingState::default(),
        &sample(five, weekly, reset),
        &THRESHOLDS,
    )
    .0
}

fn event(window: LimitWindow, threshold: u32) -> Event {
    Event { window, threshold }
}

#[test]
fn first_reading_seeds_each_window_without_a_crossing() {
    let (next, events) = crossings(
        &CrossingState::default(),
        &sample(97.0, 99.0, 100),
        &THRESHOLDS,
    );
    assert_eq!(events, vec![]);
    assert_eq!(next.five_hour, window(97.0, Some(100)));
    assert_eq!(next.weekly, window(99.0, Some(1100)));
    assert_eq!(
        crossings(&next, &sample(100.0, 100.0, 100), &THRESHOLDS).1,
        vec![]
    );
}

#[test]
fn crossing_boundaries_and_large_jumps_report_every_threshold() {
    for (pct, expected) in [
        (70.0, vec![]),
        (79.9, vec![]),
        (80.0, vec![80]),
        (80.1, vec![80]),
        (94.9, vec![80]),
        (95.0, vec![80, 95]),
        (97.0, vec![80, 95]),
    ] {
        let prev = baseline(70.0, 10.0, 100);
        let (_, events) = crossings(&prev, &sample(pct, 10.0, 100), &THRESHOLDS);
        let expected: Vec<_> = expected
            .into_iter()
            .map(|t| event(LimitWindow::FiveHour, t))
            .collect();
        assert_eq!(events, expected, "{pct}%");
        assert_eq!(
            prev,
            baseline(70.0, 10.0, 100),
            "input history is unchanged"
        );
    }
}

#[test]
fn thresholds_do_not_repeat_after_refreshes_or_percentage_dips() {
    let mut prev = baseline(70.0, 10.0, 100);
    for (pct, expected) in [
        (80.0, vec![event(LimitWindow::FiveHour, 80)]),
        (81.0, vec![]),
        (70.0, vec![]),
        (85.0, vec![]),
        (95.0, vec![event(LimitWindow::FiveHour, 95)]),
        (90.0, vec![]),
        (97.0, vec![]),
    ] {
        let (next, events) = crossings(&prev, &sample(pct, 10.0, 100), &THRESHOLDS);
        assert_eq!(events, expected, "{pct}%");
        prev = next;
    }
    assert_eq!(prev.five_hour, window(97.0, Some(100)));
}

#[test]
fn non_ok_statuses_never_emit_or_change_history() {
    let ProviderStatus::Ok { usage, .. } = sample(97.0, 97.0, 200) else {
        unreachable!()
    };
    let statuses = [
        ProviderStatus::Disabled,
        ProviderStatus::Loading,
        ProviderStatus::Failed(FetchError::Network("offline".into())),
        ProviderStatus::Stale {
            usage: usage.clone(),
            at: 1,
            err: FetchError::Network("offline".into()),
        },
        ProviderStatus::FromSession { usage, at: 1 },
    ];
    for status in statuses {
        let prev = baseline(70.0, 70.0, 100);
        assert_eq!(
            crossings(&prev, &status, &THRESHOLDS),
            (prev.clone(), vec![])
        );
        assert_eq!(
            crossings(&prev, &sample(80.0, 70.0, 100), &THRESHOLDS).1,
            vec![event(LimitWindow::FiveHour, 80)]
        );
    }
}

#[test]
fn a_known_reset_rearms_only_its_window() {
    let prev = baseline(97.0, 97.0, 100);
    let (next, events) = crossings(
        &prev,
        &ok(window(97.0, Some(200)), window(97.0, Some(1100))),
        &THRESHOLDS,
    );
    assert_eq!(
        events,
        vec![
            event(LimitWindow::FiveHour, 80),
            event(LimitWindow::FiveHour, 95)
        ]
    );
    assert_eq!(next.weekly, prev.weekly);
    assert_eq!(
        crossings(
            &next,
            &ok(window(97.0, Some(200)), window(97.0, Some(1100))),
            &THRESHOLDS
        )
        .1,
        vec![]
    );
    let (_, weekly) = crossings(
        &next,
        &ok(window(97.0, Some(200)), window(80.0, Some(1200))),
        &THRESHOLDS,
    );
    assert_eq!(weekly, vec![event(LimitWindow::Weekly, 80)]);
}

#[test]
fn reset_below_threshold_rearms_later_crossings() {
    let prev = baseline(97.0, 97.0, 100);
    let (reset, events) = crossings(&prev, &sample(5.0, 10.0, 200), &THRESHOLDS);
    assert_eq!(events, vec![]);
    assert_eq!(reset.five_hour, window(5.0, Some(200)));
    assert_eq!(
        crossings(&reset, &sample(80.0, 10.0, 200), &THRESHOLDS).1,
        vec![event(LimitWindow::FiveHour, 80)]
    );
}

#[test]
fn missing_windows_preserve_history_and_do_not_block_the_other_window() {
    let prev = baseline(97.0, 70.0, 100);
    let (missing, events) = crossings(&prev, &ok(None, window(95.0, Some(1100))), &THRESHOLDS);
    assert_eq!(
        events,
        vec![
            event(LimitWindow::Weekly, 80),
            event(LimitWindow::Weekly, 95)
        ]
    );
    assert_eq!(missing.five_hour, prev.five_hour);
    assert_eq!(
        crossings(&missing, &sample(97.0, 95.0, 100), &THRESHOLDS).1,
        vec![]
    );
    let (empty, events) = crossings(&missing, &ok(None, None), &THRESHOLDS);
    assert_eq!((empty, events), (missing, vec![]));
}

#[test]
fn a_window_first_seen_later_seeds_its_own_baseline() {
    let (prev, _) = crossings(
        &CrossingState::default(),
        &ok(None, window(70.0, Some(100))),
        &THRESHOLDS,
    );
    let (next, events) = crossings(
        &prev,
        &ok(window(97.0, Some(200)), window(80.0, Some(100))),
        &THRESHOLDS,
    );
    assert_eq!(events, vec![event(LimitWindow::Weekly, 80)]);
    assert_eq!(next.five_hour, window(97.0, Some(200)));
}

#[test]
fn unknown_reset_times_do_not_invent_a_new_window() {
    let (prev, _) = crossings(
        &CrossingState::default(),
        &ok(window(70.0, None), None),
        &THRESHOLDS,
    );
    let (high, events) = crossings(&prev, &ok(window(80.0, None), None), &THRESHOLDS);
    assert_eq!(events, vec![event(LimitWindow::FiveHour, 80)]);
    let (known, events) = crossings(&high, &ok(window(81.0, Some(100)), None), &THRESHOLDS);
    assert_eq!(events, vec![]);
    assert_eq!(known.five_hour, window(81.0, Some(100)));
    let (unknown, events) = crossings(&known, &ok(window(70.0, None), None), &THRESHOLDS);
    assert_eq!(events, vec![]);
    assert_eq!(unknown.five_hour, window(81.0, Some(100)));
    assert_eq!(
        crossings(&unknown, &ok(window(90.0, Some(100)), None), &THRESHOLDS).1,
        vec![]
    );
    assert_eq!(
        crossings(&unknown, &ok(window(80.0, Some(200)), None), &THRESHOLDS).1,
        vec![event(LimitWindow::FiveHour, 80)]
    );
}

#[test]
fn independent_providers_and_windows_keep_deterministic_event_order() {
    let claude = baseline(70.0, 70.0, 100);
    let codex = baseline(97.0, 97.0, 100);
    let current = sample(97.0, 97.0, 100);
    assert_eq!(
        crossings(&claude, &current, &THRESHOLDS).1,
        vec![
            event(LimitWindow::FiveHour, 80),
            event(LimitWindow::FiveHour, 95),
            event(LimitWindow::Weekly, 80),
            event(LimitWindow::Weekly, 95),
        ]
    );
    assert_eq!(crossings(&codex, &current, &THRESHOLDS).1, vec![]);
}

#[test]
fn configured_thresholds_replace_the_defaults() {
    let prev = baseline(70.0, 10.0, 100);
    assert_eq!(
        crossings(&prev, &sample(90.0, 10.0, 100), &[75, 90]).1,
        vec![
            event(LimitWindow::FiveHour, 75),
            event(LimitWindow::FiveHour, 90)
        ]
    );
    assert_eq!(crossings(&prev, &sample(100.0, 10.0, 100), &[]).1, vec![]);
}

#[test]
fn non_finite_percentages_do_not_poison_history() {
    for pct in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let prev = baseline(70.0, 70.0, 100);
        let (next, events) = crossings(
            &prev,
            &ok(window(pct, Some(200)), window(80.0, Some(1100))),
            &THRESHOLDS,
        );
        assert_eq!(events, vec![event(LimitWindow::Weekly, 80)]);
        assert_eq!(next.five_hour, prev.five_hour);
        assert_eq!(
            crossings(&next, &sample(80.0, 80.0, 100), &THRESHOLDS).1,
            vec![event(LimitWindow::FiveHour, 80)]
        );
    }
}

#[test]
fn reset_jitter_preserves_real_crossings_without_repeating_alerts() {
    let mut prev = baseline(70.0, 70.0, 1000);
    for (shift, pct, peak, thresholds) in [
        (1, 80.0, 80.0, vec![80]),
        (-1, 95.0, 95.0, vec![95]),
        (0, 97.0, 97.0, vec![]),
        (1, 70.0, 97.0, vec![]),
        (-1, 97.0, 97.0, vec![]),
    ] {
        let (next, events) = crossings(&prev, &sample(pct, pct, 1000 + shift), &THRESHOLDS);
        let expected: Vec<_> = [LimitWindow::FiveHour, LimitWindow::Weekly]
            .into_iter()
            .flat_map(|w| thresholds.iter().map(move |&t| event(w, t)))
            .collect();
        assert_eq!(events, expected, "{shift:+}s at {pct}%");
        assert_eq!(next.five_hour, window(peak, Some(1000 + shift)));
        assert_eq!(next.weekly, window(peak, Some(2000 + shift)));
        prev = next;
    }
}

#[test]
fn only_forward_resets_beyond_sixty_seconds_rearm_thresholds() {
    let prev = baseline(97.0, 97.0, 1000);
    for (shift, rearmed) in [
        (-100, false),
        (-61, false),
        (-60, false),
        (-1, false),
        (0, false),
        (1, false),
        (59, false),
        (60, false),
        (61, true),
        (100, true),
    ] {
        let (next, events) = crossings(&prev, &sample(97.0, 97.0, 1000 + shift), &THRESHOLDS);
        let expected = if rearmed {
            vec![
                event(LimitWindow::FiveHour, 80),
                event(LimitWindow::FiveHour, 95),
                event(LimitWindow::Weekly, 80),
                event(LimitWindow::Weekly, 95),
            ]
        } else {
            vec![]
        };
        assert_eq!(events, expected, "reset shift {shift:+}s");
        assert_eq!(
            crossings(&next, &sample(97.0, 97.0, 1000 + shift), &THRESHOLDS).1,
            vec![]
        );
    }
}

#[test]
fn reset_tolerance_handles_timestamp_extremes_without_overflow() {
    for (old, new, rearmed) in [
        (i64::MIN, i64::MAX, true),
        (i64::MAX, i64::MIN, false),
        (i64::MAX - 61, i64::MAX, true),
        (i64::MAX - 60, i64::MAX, false),
        (i64::MIN, i64::MIN + 60, false),
        (i64::MIN, i64::MIN + 61, true),
        (i64::MAX, i64::MAX, false),
    ] {
        let (prev, _) = crossings(
            &CrossingState::default(),
            &ok(window(97.0, Some(old)), None),
            &THRESHOLDS,
        );
        let (_, events) = crossings(&prev, &ok(window(97.0, Some(new)), None), &THRESHOLDS);
        let expected = if rearmed {
            vec![
                event(LimitWindow::FiveHour, 80),
                event(LimitWindow::FiveHour, 95),
            ]
        } else {
            vec![]
        };
        assert_eq!(events, expected, "{old} -> {new}");
    }
}
