use super::*;
use crate::limits::{FetchError, ProviderUsage, WindowUsage};

const NOW: i64 = 1_000_000;

fn enabled() -> Config {
    Config {
        notify_near_limit: true,
        claude_enabled: true,
        codex_enabled: true,
        ..Default::default()
    }
}

fn reading(pct: f32, at: i64) -> ProviderStatus {
    ProviderStatus::Ok {
        usage: ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: pct,
                resets_at: Some(NOW + 18_000),
            }),
            weekly: Some(WindowUsage {
                used_pct: pct,
                resets_at: Some(NOW + 604_800),
            }),
            ..Default::default()
        },
        at,
    }
}

fn notices(kind: Kind, threshold: u32) -> Vec<Notice> {
    [LimitWindow::FiveHour, LimitWindow::Weekly]
        .into_iter()
        .map(|window| Notice {
            kind,
            event: Event { window, threshold },
        })
        .collect()
}

#[test]
fn opt_in_crossings_include_every_event_and_provider() {
    let mut state = Notifications::default();
    for kind in [Kind::Claude, Kind::Codex] {
        assert_eq!(
            state.observe(&enabled(), kind, &reading(70.0, NOW), NOW),
            vec![]
        );
        assert_eq!(
            state.observe(&enabled(), kind, &reading(97.0, NOW), NOW),
            vec![
                Notice {
                    kind,
                    event: Event {
                        window: LimitWindow::FiveHour,
                        threshold: 80
                    }
                },
                Notice {
                    kind,
                    event: Event {
                        window: LimitWindow::FiveHour,
                        threshold: 95
                    }
                },
                Notice {
                    kind,
                    event: Event {
                        window: LimitWindow::Weekly,
                        threshold: 80
                    }
                },
                Notice {
                    kind,
                    event: Event {
                        window: LimitWindow::Weekly,
                        threshold: 95
                    }
                },
            ]
        );
        assert_eq!(
            state.observe(&enabled(), kind, &reading(97.0, NOW), NOW),
            vec![]
        );
    }
}

#[test]
fn disabled_notifications_and_providers_do_not_replay_crossings() {
    for (notify, provider) in [(false, true), (true, false)] {
        let mut state = Notifications::default();
        let cfg = Config {
            notify_near_limit: notify,
            claude_enabled: provider,
            ..enabled()
        };
        assert_eq!(
            state.observe(&cfg, Kind::Claude, &reading(70.0, NOW), NOW),
            vec![]
        );
        assert_eq!(
            state.observe(&cfg, Kind::Claude, &reading(97.0, NOW), NOW),
            vec![]
        );
        assert_eq!(
            state.observe(&enabled(), Kind::Claude, &reading(97.0, NOW), NOW),
            vec![]
        );
    }
}

#[test]
fn non_ok_readings_do_not_emit_or_change_history() {
    let ProviderStatus::Ok { usage, .. } = reading(97.0, NOW) else {
        unreachable!()
    };
    let mut state = Notifications::default();
    state.observe(&enabled(), Kind::Claude, &reading(70.0, NOW), NOW);
    for status in [
        ProviderStatus::Disabled,
        ProviderStatus::Loading,
        ProviderStatus::Failed(FetchError::NoLogin),
        ProviderStatus::Stale {
            usage: usage.clone(),
            at: NOW,
            err: FetchError::Network("offline".into()),
        },
        ProviderStatus::FromSession { usage, at: NOW },
    ] {
        let previous = state.clone();
        assert_eq!(
            state.observe(&enabled(), Kind::Claude, &status, NOW),
            vec![]
        );
        assert_eq!(state, previous);
    }
    assert_eq!(
        state.observe(&enabled(), Kind::Claude, &reading(80.0, NOW), NOW),
        notices(Kind::Claude, 80)
    );
}

#[test]
fn queued_old_results_and_expired_windows_are_not_fresh_alerts() {
    let mut state = Notifications::default();
    state.observe(&enabled(), Kind::Claude, &reading(70.0, NOW), NOW);
    let previous = state.clone();
    assert_eq!(
        state.observe(&enabled(), Kind::Claude, &reading(97.0, NOW - 901), NOW),
        vec![]
    );
    assert_eq!(state, previous);
    let ProviderStatus::Ok { mut usage, .. } = reading(97.0, NOW) else {
        unreachable!()
    };
    usage.five_hour.as_mut().unwrap().resets_at = Some(NOW);
    assert_eq!(
        state.observe(
            &enabled(),
            Kind::Claude,
            &ProviderStatus::Ok { usage, at: NOW },
            NOW
        ),
        vec![
            Notice {
                kind: Kind::Claude,
                event: Event {
                    window: LimitWindow::Weekly,
                    threshold: 80
                }
            },
            Notice {
                kind: Kind::Claude,
                event: Event {
                    window: LimitWindow::Weekly,
                    threshold: 95
                }
            }
        ]
    );
    assert_eq!(
        state.observe(&enabled(), Kind::Claude, &reading(80.0, NOW), NOW),
        vec![Notice {
            kind: Kind::Claude,
            event: Event {
                window: LimitWindow::FiveHour,
                threshold: 80
            }
        }]
    );
}

#[test]
fn freshness_uses_the_existing_refresh_cadence_and_hardware_has_no_alerts() {
    let cfg = Config {
        limits_interval_secs: 3600,
        ..enabled()
    };
    let mut state = Notifications::default();
    state.observe(&cfg, Kind::Claude, &reading(70.0, NOW), NOW);
    assert_eq!(
        state.observe(&cfg, Kind::Claude, &reading(80.0, NOW - 901), NOW),
        notices(Kind::Claude, 80)
    );
    let previous = state.clone();
    assert_eq!(
        state.observe(&cfg, Kind::Hardware, &reading(97.0, NOW), NOW),
        vec![]
    );
    assert_eq!(state, previous);
}

#[test]
fn balloon_groups_highest_crossings_with_exact_localized_text() {
    let mut batch = notices(Kind::Codex, 80);
    batch.extend(notices(Kind::Claude, 95));
    batch.extend(notices(Kind::Claude, 80));
    for (lang, title, body) in [
        (
            Lang::En,
            "CCT — Usage warning",
            "Claude Code 5h reached 95%\nClaude Code weekly reached 95%\nCodex 5h reached 80%\nCodex weekly reached 80%",
        ),
        (
            Lang::PtBr,
            "CCT — Aviso de uso",
            "Claude Code 5h atingiu 95%\nClaude Code semanal atingiu 95%\nCodex 5h atingiu 80%\nCodex semanal atingiu 80%",
        ),
    ] {
        assert_eq!(t(lang, Key::NotifyTitle), title);
        assert_eq!(balloon_text(lang, &enabled(), &batch), Some(body.into()));
        assert!(body.encode_utf16().count() < 256);
        assert!(title.encode_utf16().count() < 64);
    }
    assert_eq!(t(Lang::En, Key::NotifyNearLimit), "Notify near limit");
    assert_eq!(
        t(Lang::PtBr, Key::NotifyNearLimit),
        "Avisar ao se aproximar do limite"
    );
}

#[test]
fn text_filters_current_settings_and_empty_batches() {
    let cfg = Config {
        claude_enabled: false,
        ..enabled()
    };
    let mut batch = notices(Kind::Claude, 95);
    assert_eq!(balloon_text(Lang::En, &cfg, &batch), None);
    batch.extend(notices(Kind::Codex, 80));
    assert_eq!(
        balloon_text(Lang::En, &cfg, &batch),
        Some("Codex 5h reached 80%\nCodex weekly reached 80%".into())
    );
    assert_eq!(balloon_text(Lang::En, &enabled(), &[]), None);
    assert_eq!(balloon_text(Lang::En, &Config::default(), &batch), None);
}

#[test]
fn utf16_buffers_are_terminated_and_do_not_split_surrogates() {
    assert_eq!(wide_array::<4>("a💡b"), [97, 0xd83d, 0xdca1, 0]);
    assert_eq!(wide_array::<3>("a💡"), [97, 0, 0]);
    assert_eq!(wide_array::<1>("hello"), [0]);
    assert_eq!(wide_array::<0>("hello"), [0u16; 0]);
    assert_eq!(wide_array::<4>(""), [0, 0, 0, 0]);
}
