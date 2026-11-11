//! How a provider's displayed status evolves after each fetch.

use super::{FetchError, ProviderStatus, ProviderUsage};

/// Previous data stays on screen this long after fetches start failing.
pub const STALE_SECS: i64 = 15 * 60;

/// Auth problems are reported immediately; transient errors keep recent data as `Stale`.
pub fn next_status(
    prev: &ProviderStatus,
    result: Result<ProviderUsage, FetchError>,
    now: i64,
) -> ProviderStatus {
    let err = match result {
        Ok(usage) => return ProviderStatus::Ok { usage, at: now },
        Err(err) => err,
    };
    let transient = matches!(
        err,
        FetchError::Network(_) | FetchError::Offline(_) | FetchError::BadResponse(_)
    );
    if transient
        && let ProviderStatus::Ok { usage, at } | ProviderStatus::Stale { usage, at, .. } = prev
        && now - at <= STALE_SECS
    {
        return ProviderStatus::Stale {
            usage: usage.clone(),
            at: *at,
            err,
        };
    }
    ProviderStatus::Failed(err)
}

/// Display-time view of a status at `now`: fetched data older than `max_age` (e.g. after
/// sleep) becomes "no recent data", and windows whose reset time has passed are dropped.
pub fn age_status(status: &ProviderStatus, now: i64, max_age: i64) -> ProviderStatus {
    let drop_reset = |usage: &ProviderUsage| {
        let keep = |w: &Option<super::WindowUsage>| {
            w.clone()
                .filter(|w| w.resets_at.is_none_or(|reset| reset > now))
        };
        ProviderUsage {
            five_hour: keep(&usage.five_hour),
            weekly: keep(&usage.weekly),
            weekly_by_model: usage
                .weekly_by_model
                .iter()
                .filter(|(_, window)| window.resets_at.is_none_or(|reset| reset > now))
                .cloned()
                .collect(),
            extra_usage_enabled: usage.extra_usage_enabled,
        }
    };
    match status {
        ProviderStatus::Ok { at, .. } | ProviderStatus::Stale { at, .. } if now - at > max_age => {
            ProviderStatus::Failed(FetchError::Network(String::new()))
        }
        ProviderStatus::Ok { usage, at } => ProviderStatus::Ok {
            usage: drop_reset(usage),
            at: *at,
        },
        ProviderStatus::Stale { usage, at, err } => ProviderStatus::Stale {
            usage: drop_reset(usage),
            at: *at,
            err: err.clone(),
        },
        ProviderStatus::FromSession { usage, at } => ProviderStatus::FromSession {
            usage: drop_reset(usage),
            at: *at,
        },
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{FetchError, ProviderStatus, ProviderUsage, WindowUsage};

    fn usage(pct: f32) -> ProviderUsage {
        ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: pct,
                resets_at: Some(5000),
            }),
            weekly: None,
            ..Default::default()
        }
    }

    #[test]
    fn ok_result_sets_ok() {
        let s = next_status(&ProviderStatus::Loading, Ok(usage(10.0)), 100);
        assert_eq!(
            s,
            ProviderStatus::Ok {
                usage: usage(10.0),
                at: 100
            }
        );
    }

    #[test]
    fn network_error_within_15min_is_stale() {
        let prev = ProviderStatus::Ok {
            usage: usage(10.0),
            at: 1000,
        };
        let err = FetchError::Network("timeout".into());
        let s = next_status(&prev, Err(err.clone()), 1000 + 899);
        assert_eq!(
            s,
            ProviderStatus::Stale {
                usage: usage(10.0),
                at: 1000,
                err
            }
        );
    }

    #[test]
    fn stale_stays_stale_until_window_passes() {
        let err = FetchError::BadResponse("HTTP 500".into());
        let prev = ProviderStatus::Stale {
            usage: usage(10.0),
            at: 1000,
            err: err.clone(),
        };
        let s = next_status(&prev, Err(err.clone()), 1000 + 900);
        assert_eq!(
            s,
            ProviderStatus::Stale {
                usage: usage(10.0),
                at: 1000,
                err
            }
        );
    }

    #[test]
    fn network_error_after_15min_fails() {
        let prev = ProviderStatus::Ok {
            usage: usage(10.0),
            at: 1000,
        };
        let err = FetchError::Network("timeout".into());
        let s = next_status(&prev, Err(err.clone()), 1000 + 901);
        assert_eq!(s, ProviderStatus::Failed(err));
    }

    #[test]
    fn offline_error_is_transient_like_network() {
        let prev = ProviderStatus::Ok {
            usage: usage(10.0),
            at: 1000,
        };
        let err = FetchError::Offline("host not found".into());
        let s = next_status(&prev, Err(err.clone()), 1000 + 899);
        assert_eq!(
            s,
            ProviderStatus::Stale {
                usage: usage(10.0),
                at: 1000,
                err
            }
        );
    }

    #[test]
    fn network_error_without_previous_data_fails() {
        let err = FetchError::Network("dns".into());
        let s = next_status(&ProviderStatus::Loading, Err(err.clone()), 10);
        assert_eq!(s, ProviderStatus::Failed(err));
    }

    #[test]
    fn expired_never_stale() {
        let prev = ProviderStatus::Ok {
            usage: usage(10.0),
            at: 1000,
        };
        let s = next_status(&prev, Err(FetchError::Expired), 1001);
        assert_eq!(s, ProviderStatus::Failed(FetchError::Expired));
    }
}

#[cfg(test)]
mod aging_tests {
    use super::*;
    use crate::limits::{FetchError, ProviderStatus, ProviderUsage, WindowUsage};

    fn usage(reset5: i64, resetw: i64) -> ProviderUsage {
        ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: 92.0,
                resets_at: Some(reset5),
            }),
            weekly: Some(WindowUsage {
                used_pct: 30.0,
                resets_at: Some(resetw),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn fresh_ok_is_unchanged() {
        let s = ProviderStatus::Ok {
            usage: usage(10_000, 90_000),
            at: 1000,
        };
        assert_eq!(age_status(&s, 1500, 900), s);
    }

    #[test]
    fn ok_older_than_max_age_has_no_data() {
        // Laptop slept for hours: the old value must not be shown as current.
        let s = ProviderStatus::Ok {
            usage: usage(100_000, 900_000),
            at: 1000,
        };
        assert!(matches!(
            age_status(&s, 1000 + 901, 900),
            ProviderStatus::Failed(FetchError::Network(_))
        ));
    }

    #[test]
    fn stale_older_than_max_age_has_no_data() {
        let s = ProviderStatus::Stale {
            usage: usage(100_000, 900_000),
            at: 1000,
            err: FetchError::Network("x".into()),
        };
        assert!(matches!(
            age_status(&s, 5000, 900),
            ProviderStatus::Failed(_)
        ));
    }

    #[test]
    fn window_past_reset_is_dropped() {
        let s = ProviderStatus::Ok {
            usage: usage(1200, 90_000),
            at: 1000,
        };
        let ProviderStatus::Ok { usage: u, .. } = age_status(&s, 1300, 900) else {
            panic!()
        };
        assert_eq!(u.five_hour, None);
        assert!(u.weekly.is_some());
    }

    #[test]
    fn session_data_keeps_age_but_drops_reset_windows() {
        let s = ProviderStatus::FromSession {
            usage: usage(1200, 90_000),
            at: 10,
        };
        let ProviderStatus::FromSession { usage: u, .. } = age_status(&s, 1300, 900) else {
            panic!()
        };
        assert_eq!(u.five_hour, None);
        assert!(u.weekly.is_some());
    }

    #[test]
    fn model_windows_expire_independently_without_losing_extra_usage() {
        let mut u = usage(10_000, 90_000);
        u.weekly_by_model = vec![
            (
                super::super::ModelKind::Opus,
                WindowUsage {
                    used_pct: 72.0,
                    resets_at: Some(1200),
                },
            ),
            (
                super::super::ModelKind::Sonnet,
                WindowUsage {
                    used_pct: 0.0,
                    resets_at: None,
                },
            ),
        ];
        u.extra_usage_enabled = true;
        let status = ProviderStatus::Ok { usage: u, at: 1000 };
        let ProviderStatus::Ok { usage, .. } = age_status(&status, 1200, 900) else {
            panic!("fresh status must remain OK")
        };
        assert_eq!(
            usage.weekly_by_model,
            vec![(
                super::super::ModelKind::Sonnet,
                WindowUsage {
                    used_pct: 0.0,
                    resets_at: None
                }
            )]
        );
        assert!(usage.extra_usage_enabled);
        assert_eq!(usage.five_hour.unwrap().used_pct, 92.0);
    }
}
