//! Subscription limit types shared by the Claude and Codex providers.

pub mod backoff;
pub mod claude;
pub mod codex;
pub mod http;
pub mod keepalive;
pub mod notify;
pub mod ratelimit;
pub mod reset;
pub mod state;
pub mod switcher;
pub mod terminal;
pub mod worker;

/// Usage of one limit window (5-hour or weekly).
#[derive(Clone, Debug, PartialEq)]
pub struct WindowUsage {
    /// Percent used, already clamped to 0–100.
    pub used_pct: f32,
    /// Unix seconds when the window resets, if known.
    pub resets_at: Option<i64>,
}

/// Claude models with independently reported weekly limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Opus,
    Sonnet,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderUsage {
    pub five_hour: Option<WindowUsage>,
    pub weekly: Option<WindowUsage>,
    /// Only model windows the provider actually reported, in display order.
    pub weekly_by_model: Vec<(ModelKind, WindowUsage)>,
    /// True only when the provider explicitly enables extra usage.
    pub extra_usage_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FetchError {
    NoLogin,
    Expired,
    NoSubscription,
    Network(String),
    /// The host could not be reached at all (name not found, connection refused) and no
    /// Windows proxy is set, so another client on this PC can't get through either (#106).
    /// Transient, and shown exactly like `Network`.
    Offline(String),
    /// HTTP 429. Holds when to try again: from `Retry-After`, or set by the worker's wait
    /// (`ratelimit`) before the result reaches the UI. Never any header text (#87).
    RateLimited(Option<i64>),
    BadResponse(String),
}

/// What the strip shows for one provider.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderStatus {
    Disabled,
    Loading,
    Ok {
        usage: ProviderUsage,
        at: i64,
    },
    /// Latest fetch failed; showing data fetched at `at` (less than 15 min old).
    Stale {
        usage: ProviderUsage,
        at: i64,
        err: FetchError,
    },
    /// Data read from the CLI's own session log written at `at`.
    FromSession {
        usage: ProviderUsage,
        at: i64,
    },
    Failed(FetchError),
}

pub trait Provider: Send {
    fn fetch(&self) -> Result<ProviderUsage, FetchError>;
    fn has_login(&self) -> bool;
}

pub const FIVE_HOUR_SECS: i64 = 5 * 60 * 60;
pub const WEEK_SECS: i64 = 7 * 24 * 60 * 60;

/// Fraction of `window_len` (`0.0..=1.0`) that has elapsed at `now`, or `None` when
/// `resets_at` is missing or has already passed (#14).
pub fn elapsed_fraction(resets_at: Option<i64>, window_len: i64, now: i64) -> Option<f64> {
    if window_len <= 0 {
        return None;
    }
    let remaining = resets_at?.checked_sub(now)?;
    if remaining <= 0 {
        return None;
    }
    let elapsed = window_len.saturating_sub(remaining).max(0);
    Some((elapsed as f64 / window_len as f64).clamp(0.0, 1.0))
}

const MIN_ELAPSED_FRACTION: f64 = 0.05;

/// Current usage as a multiple of the linear pace (`1.0` means 100% of pace).
pub fn pace_ratio(
    resets_at: Option<i64>,
    window_len: i64,
    used_pct: f32,
    now: i64,
) -> Option<f64> {
    if !used_pct.is_finite() || used_pct <= 0.0 {
        return None;
    }
    let elapsed = elapsed_fraction(resets_at, window_len, now)?;
    (elapsed >= MIN_ELAPSED_FRACTION).then_some((f64::from(used_pct) / 100.0) / elapsed)
}

/// Unix timestamp when the current linear usage rate would reach 100%, if before reset.
pub fn projected_exhaustion(
    resets_at: Option<i64>,
    window_len: i64,
    used_pct: f32,
    now: i64,
) -> Option<i64> {
    if !used_pct.is_finite() || !(0.0 < used_pct && used_pct < 100.0) {
        return None;
    }
    let reset = resets_at?;
    let elapsed = elapsed_fraction(Some(reset), window_len, now)?;
    if elapsed < MIN_ELAPSED_FRACTION {
        return None;
    }
    let elapsed_secs = window_len as f64 * elapsed;
    let projected = now as f64 + elapsed_secs * (100.0 / f64::from(used_pct) - 1.0);
    if !projected.is_finite() || projected >= reset as f64 {
        return None;
    }
    let timestamp = projected.ceil();
    (timestamp >= i64::MIN as f64 && timestamp <= i64::MAX as f64).then_some(timestamp as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn elapsed_fraction_covers_both_windows_and_edge_cases() {
        for len in [FIVE_HOUR_SECS, WEEK_SECS] {
            let cases = [
                (None, None),
                (Some(NOW - 1), None),
                (Some(NOW), None),
                (Some(NOW + len), Some(0.0)),
                (Some(NOW + len + 10), Some(0.0)),
                (Some(NOW + len * 3 / 4), Some(0.25)),
                (Some(NOW + len / 2), Some(0.5)),
                (Some(NOW + len / 4), Some(0.75)),
            ];
            for (reset, expected) in cases {
                let got = elapsed_fraction(reset, len, NOW);
                match (got, expected) {
                    (Some(a), Some(b)) => {
                        assert!(
                            (a - b).abs() < 1e-9,
                            "len={len} reset={reset:?}: {a} != {b}"
                        )
                    }
                    (a, b) => assert_eq!(a, b, "len={len} reset={reset:?}"),
                }
            }
            let near_end = elapsed_fraction(Some(NOW + 1), len, NOW).unwrap();
            assert!(near_end > 0.999 && near_end < 1.0);
        }
        assert_eq!(elapsed_fraction(Some(NOW + 60), 0, NOW), None);
        assert_eq!(elapsed_fraction(Some(NOW + 60), -10, NOW), None);
        assert_eq!(elapsed_fraction(Some(i64::MAX), FIVE_HOUR_SECS, -1), None);
    }

    #[test]
    fn pace_ratio_is_table_driven_for_both_windows_and_edge_cases() {
        for window_len in [FIVE_HOUR_SECS, WEEK_SECS] {
            let reset = NOW + window_len;
            let cases = [
                ("start", 0.0, 0.0, None),
                ("guard after reset", 0.01, 5.0, None),
                ("below pace", 0.5, 20.0, Some(0.4)),
                ("on pace", 0.5, 50.0, Some(1.0)),
                ("above pace", 0.25, 50.0, Some(2.0)),
                ("near end", 0.75, 75.0, Some(1.0)),
            ];
            for (name, elapsed, used, expected) in cases {
                let now = NOW + (window_len as f64 * elapsed) as i64;
                let got = pace_ratio(Some(reset), window_len, used, now);
                match (got, expected) {
                    (Some(actual), Some(want)) => assert!(
                        (actual - want).abs() < 1e-9,
                        "{name}, len={window_len}: {actual} != {want}"
                    ),
                    (actual, want) => assert_eq!(actual, want, "{name}, len={window_len}"),
                }
            }
        }
        for (reset, used) in [(None, 20.0), (Some(NOW - 1), 20.0), (Some(NOW + FIVE_HOUR_SECS), 0.0), (Some(NOW + FIVE_HOUR_SECS), -1.0)] {
            assert_eq!(pace_ratio(reset, FIVE_HOUR_SECS, used, NOW), None);
        }
        assert_eq!(
            pace_ratio(
                Some(NOW + FIVE_HOUR_SECS),
                FIVE_HOUR_SECS,
                100.0,
                NOW + FIVE_HOUR_SECS / 2
            ),
            Some(2.0)
        );
    }

    #[test]
    fn projected_exhaustion_is_table_driven_for_both_windows_and_edge_cases() {
        for window_len in [FIVE_HOUR_SECS, WEEK_SECS] {
            let reset = NOW + window_len;
            let cases = [
                ("guard", 0.01, 5.0, None),
                ("below pace", 0.5, 20.0, None),
                ("above pace", 0.3, 60.0, Some(NOW + window_len / 2)),
                ("at limit", 0.5, 100.0, None),
            ];
            for (name, elapsed, used, expected) in cases {
                let now = NOW + (window_len as f64 * elapsed) as i64;
                let got = projected_exhaustion(Some(reset), window_len, used, now);
                assert_eq!(got, expected, "{name}, len={window_len}");
            }
        }
        for (reset, used) in [(None, 20.0), (Some(NOW - 1), 20.0), (Some(NOW + FIVE_HOUR_SECS), 0.0), (Some(NOW + FIVE_HOUR_SECS), 100.0)] {
            assert_eq!(projected_exhaustion(reset, FIVE_HOUR_SECS, used, NOW), None);
        }
    }
}
