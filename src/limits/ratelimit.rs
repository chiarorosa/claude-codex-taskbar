//! Waiting out an HTTP 429 per provider (#87).
//!
//! While a provider is rate-limiting us, every extra request can extend the block, so its
//! worker thread skips the provider until the next-try time: the response's `Retry-After`
//! (capped at an hour, so a bogus value can't freeze the strip), or a doubling delay when it
//! is missing (the interval, 2x, 4x, ... capped at 30 min, or at one interval if that is
//! longer). *Refresh now* waits too; the tooltip shows when the next try is. A success, or
//! a new Claude login, starts over.

use super::FetchError;

pub const MAX_RETRY_AFTER_SECS: i64 = 60 * 60;
pub const MAX_DOUBLING_SECS: i64 = 30 * 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RateLimit {
    misses: u32,
    until: i64,
}

impl RateLimit {
    pub fn allows(&self, now: i64) -> bool {
        now >= self.until
    }

    /// Notes one fetch result. A 429 sets the wait and writes the next-try time into the
    /// error, so the tooltip shows exactly when CCT will ask again.
    pub fn record<T>(&mut self, result: &mut Result<T, FetchError>, now: i64, interval_secs: i64) {
        match result {
            Ok(_) => *self = RateLimit::default(),
            Err(FetchError::RateLimited(retry_at)) => {
                self.misses = self.misses.saturating_add(1);
                let interval = interval_secs.max(1);
                let doubling = interval
                    .saturating_mul(1 << (self.misses - 1).min(16))
                    .min(MAX_DOUBLING_SECS.max(interval));
                self.until = match *retry_at {
                    Some(at) => at.clamp(now, now.saturating_add(MAX_RETRY_AFTER_SECS)),
                    None => now.saturating_add(doubling),
                };
                *retry_at = Some(self.until);
            }
            // Other errors neither extend nor end a wait.
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    fn limited(at: Option<i64>) -> Result<(), FetchError> {
        Err(FetchError::RateLimited(at))
    }

    #[test]
    fn retry_after_sets_the_wait_and_the_shown_time() {
        let mut limit = RateLimit::default();
        let mut result = limited(Some(NOW + 90));
        limit.record(&mut result, NOW, 120);
        assert_eq!(result, limited(Some(NOW + 90)));
        assert!(!limit.allows(NOW + 89));
        assert!(limit.allows(NOW + 90));
    }

    #[test]
    fn retry_after_is_capped_at_an_hour_and_never_in_the_past() {
        let mut limit = RateLimit::default();
        let mut result = limited(Some(NOW + 7 * 24 * 3600));
        limit.record(&mut result, NOW, 120);
        assert_eq!(result, limited(Some(NOW + MAX_RETRY_AFTER_SECS)));

        let mut limit = RateLimit::default();
        let mut result = limited(Some(NOW - 50));
        limit.record(&mut result, NOW, 120);
        assert_eq!(result, limited(Some(NOW)));
        assert!(limit.allows(NOW));
    }

    #[test]
    fn without_retry_after_the_wait_doubles_from_the_interval_up_to_30_min() {
        let mut limit = RateLimit::default();
        let mut now = NOW;
        for want in [120, 240, 480, 960, 1800, 1800] {
            let mut result = limited(None);
            limit.record(&mut result, now, 120);
            assert_eq!(result, limited(Some(now + want)), "{want}");
            now += want;
        }
    }

    #[test]
    fn the_doubling_wait_is_never_shorter_than_one_interval() {
        let mut limit = RateLimit::default();
        let mut result = limited(None);
        limit.record(&mut result, NOW, 3600);
        assert_eq!(result, limited(Some(NOW + 3600)));
        let mut result = limited(None);
        limit.record(&mut result, NOW + 3600, 3600);
        assert_eq!(result, limited(Some(NOW + 7200)));
    }

    #[test]
    fn a_success_starts_over_and_other_errors_change_nothing() {
        let mut limit = RateLimit::default();
        limit.record(&mut limited(None), NOW, 60);
        limit.record(&mut limited(None), NOW + 60, 60);
        let before = limit;
        limit.record(
            &mut Err::<(), _>(FetchError::Network("timed out".into())),
            NOW + 180,
            60,
        );
        assert_eq!(limit, before);

        limit.record(&mut Ok(()), NOW + 200, 60);
        assert!(limit.allows(NOW + 200));
        let mut result = limited(None);
        limit.record(&mut result, NOW + 300, 60);
        assert_eq!(result, limited(Some(NOW + 360)), "back to one interval");
    }
}
