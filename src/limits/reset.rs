//! One-off refresh right after a known limit reset, so the strip does not keep showing a
//! window's old percentage until the next regular fetch.

/// Seconds to wait past the reset time, giving the provider time to roll the window over.
pub const RESET_GRACE_SECS: i64 = 5;

/// Delay in seconds until a one-off refresh just after the earliest future reset, or `None`
/// when no reset happens before the next regular refresh (`interval_secs` from now).
pub fn reset_refresh_delay(resets: &[i64], now: i64, interval_secs: u32) -> Option<u32> {
    let next = resets.iter().copied().filter(|&r| r > now).min()?;
    let delay = next - now + RESET_GRACE_SECS;
    (delay < i64::from(interval_secs)).then_some(delay as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

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
