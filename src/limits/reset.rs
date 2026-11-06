//! One-off refresh right after a known limit reset, so the strip does not keep showing a
//! window's old percentage until the next regular fetch.

use super::ProviderStatus;

/// Seconds to wait past the reset time, giving the provider time to roll the window over.
pub const RESET_GRACE_SECS: i64 = 5;

/// Delay in seconds until a one-off refresh just after the earliest future reset, or `None`
/// when no reset happens before the next regular refresh (`interval_secs` from now).
pub fn reset_refresh_delay(resets: &[i64], now: i64, interval_secs: u32) -> Option<u32> {
    let next = resets.iter().copied().filter(|&r| r > now).min()?;
    let delay = next - now + RESET_GRACE_SECS;
    (delay < i64::from(interval_secs)).then_some(delay as u32)
}

/// Reset times of every window the status still shows (including stale data).
pub fn status_resets(status: &ProviderStatus) -> Vec<i64> {
    let usage = match status {
        ProviderStatus::Ok { usage, .. }
        | ProviderStatus::Stale { usage, .. }
        | ProviderStatus::FromSession { usage, .. } => usage,
        ProviderStatus::Disabled | ProviderStatus::Loading | ProviderStatus::Failed(_) => {
            return Vec::new();
        }
    };
    let models = usage.weekly_by_model.iter().map(|(_, w)| w);
    [&usage.five_hour, &usage.weekly]
        .into_iter()
        .flatten()
        .chain(models)
        .filter_map(|w| w.resets_at)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{FetchError, ModelKind, ProviderUsage, WindowUsage};

    const NOW: i64 = 1_000_000;

    fn window(resets_at: Option<i64>) -> Option<WindowUsage> {
        Some(WindowUsage {
            used_pct: 50.0,
            resets_at,
        })
    }

    fn usage() -> ProviderUsage {
        ProviderUsage {
            five_hour: window(Some(10)),
            weekly: window(None),
            weekly_by_model: vec![(ModelKind::Opus, window(Some(30)).unwrap())],
            extra_usage_enabled: false,
        }
    }

    #[test]
    fn shown_statuses_report_every_known_reset() {
        let shown = [
            ProviderStatus::Ok {
                usage: usage(),
                at: 0,
            },
            ProviderStatus::Stale {
                usage: usage(),
                at: 0,
                err: FetchError::Network("x".into()),
            },
            ProviderStatus::FromSession {
                usage: usage(),
                at: 0,
            },
        ];
        for status in &shown {
            assert_eq!(status_resets(status), vec![10, 30], "{status:?}");
        }
    }

    #[test]
    fn statuses_without_data_report_no_resets() {
        for status in [
            ProviderStatus::Disabled,
            ProviderStatus::Loading,
            ProviderStatus::Failed(FetchError::NoLogin),
        ] {
            assert!(status_resets(&status).is_empty());
        }
    }

    #[test]
    fn reset_before_next_refresh_schedules_just_after_it() {
        assert_eq!(reset_refresh_delay(&[NOW + 30], NOW, 120), Some(35));
    }

    #[test]
    fn reset_covered_by_the_regular_refresh_is_ignored() {
        assert_eq!(reset_refresh_delay(&[NOW + 300], NOW, 120), None);
        // Grace included, the one-off would land at or after the regular refresh.
        assert_eq!(reset_refresh_delay(&[NOW + 115], NOW, 120), None);
    }

    #[test]
    fn past_or_current_reset_is_ignored() {
        assert_eq!(reset_refresh_delay(&[NOW - 10], NOW, 120), None);
        assert_eq!(reset_refresh_delay(&[NOW], NOW, 120), None);
    }

    #[test]
    fn earliest_future_reset_wins() {
        assert_eq!(
            reset_refresh_delay(&[NOW + 90, NOW - 5, NOW + 40, NOW + 600], NOW, 120),
            Some(45)
        );
    }

    #[test]
    fn no_resets_means_no_refresh() {
        assert_eq!(reset_refresh_delay(&[], NOW, 120), None);
    }
}
