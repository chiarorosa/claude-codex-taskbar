//! Placement watch: turns per-attempt placement outcomes into a settled
//! failure flag and at most one notification per CCT run. Pure logic, no Win32: the
//! caller passes the time as whole seconds from a monotonic clock.

use super::taskbar::{Side, TaskbarInfo, is_horizontal};

/// How long a failure (`NoRoom` or `LookupFailed`, in any mix) must last, without a
/// `Placed` in between, before it settles.
pub const SETTLE_SECS: u64 = 10;

/// Nothing settles within this many seconds of startup or of the last
/// [`PlacementWatch::restart`] (Explorer and the taskbar are still settling then).
pub const GRACE_SECS: u64 = 30;

/// Result of one attempt to place the strip next to the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Placed,
    /// Both edges were read but the gap is too small.
    NoRoom,
    /// The tray geometry could not be read.
    LookupFailed,
}

/// What one uncovered render means for the watch: `None` on the left side and on a
/// vertical taskbar, which hide silently as before and never notify; on the right of a
/// horizontal taskbar, a missing edge is a failed lookup and a layout with no width (both
/// edges read) means the gap is too small.
pub fn outcome_of(side: Side, info: &TaskbarInfo, layout_width: i32) -> Option<Outcome> {
    match side {
        Side::Left => None,
        Side::Right if !is_horizontal(info.rect) => None,
        Side::Right if info.apps_right.is_none() || info.tray_left.is_none() => {
            Some(Outcome::LookupFailed)
        }
        Side::Right if layout_width == 0 => Some(Outcome::NoRoom),
        Side::Right => Some(Outcome::Placed),
    }
}

/// What the caller should do after an observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub settled_failure: bool,
    pub notify: bool,
}

/// Lives for one CCT run. The startup grace period starts at second 0.
#[derive(Default, Debug)]
pub struct PlacementWatch {
    /// Start of the current grace period.
    grace_from: u64,
    /// When the current, uninterrupted failure began.
    failing_since: Option<u64>,
    settled: bool,
    /// The one notice of this run has been raised.
    notified: bool,
    /// The strip has been placed next to the tray at least once this run.
    ever_placed: bool,
    /// The raised notice is still to be delivered; `true` when it should open Settings.
    pending: Option<bool>,
}

impl PlacementWatch {
    pub fn observe(&mut self, outcome: Outcome, now_secs: u64) -> Verdict {
        match outcome {
            Outcome::Placed => {
                self.failing_since = None;
                self.settled = false;
                self.ever_placed = true;
                self.pending = None;
            }
            Outcome::NoRoom | Outcome::LookupFailed => {
                let since = *self.failing_since.get_or_insert(now_secs);
                let past_grace = now_secs >= self.grace_from.saturating_add(GRACE_SECS);
                if past_grace && now_secs.saturating_sub(since) >= SETTLE_SECS {
                    self.settled = true;
                }
            }
        }
        let notify = self.settled && !self.notified;
        if notify {
            self.notified = true;
            // Only the startup decision opens Settings; a failure that settles after the
            // strip was placed shows the balloon alone.
            self.pending = Some(!self.ever_placed);
        }
        Verdict {
            settled_failure: self.settled,
            notify,
        }
    }

    /// Explorer restarted (`TaskbarCreated`): forget the current failure and start a new
    /// grace period. The one notice per run stays used.
    pub fn restart(&mut self, now_secs: u64) {
        self.grace_from = now_secs;
        self.failing_since = None;
        self.settled = false;
    }

    pub fn settled_failure(&self) -> bool {
        self.settled
    }

    /// The notice still to be delivered, while its failure is settled: `Some(true)` when it
    /// should also open Settings.
    pub fn pending_notice(&self) -> Option<bool> {
        self.pending.filter(|_| self.settled)
    }

    /// The pending notice has been delivered.
    pub fn notice_sent(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::taskbar::Rect;

    fn v(settled_failure: bool, notify: bool) -> Verdict {
        Verdict {
            settled_failure,
            notify,
        }
    }

    /// Seconds well after the startup grace period.
    const LATER: u64 = 100;

    /// A watch whose strip has been placed once, well after startup.
    fn placed_watch() -> PlacementWatch {
        let mut w = PlacementWatch::default();
        assert_eq!(w.observe(Outcome::Placed, LATER - 1), v(false, false));
        w
    }

    #[test]
    fn nothing_settles_during_the_startup_grace_period() {
        let mut w = PlacementWatch::default();
        for now in 0..GRACE_SECS {
            assert_eq!(w.observe(Outcome::NoRoom, now), v(false, false), "{now}");
        }
        // Failing since 0: the failure settles as soon as the grace period ends.
        assert_eq!(w.observe(Outcome::NoRoom, GRACE_SECS), v(true, true));
        assert!(w.settled_failure());
    }

    #[test]
    fn a_transient_failure_during_grace_never_settles() {
        let mut w = PlacementWatch::default();
        for now in 0..25 {
            assert!(!w.observe(Outcome::LookupFailed, now).notify);
        }
        assert_eq!(w.observe(Outcome::Placed, 25), v(false, false));
        assert_eq!(w.observe(Outcome::Placed, GRACE_SECS + 5), v(false, false));
        // A new failure after grace starts its own clock.
        assert_eq!(w.observe(Outcome::NoRoom, 40), v(false, false));
        assert_eq!(w.observe(Outcome::NoRoom, 49), v(false, false));
        assert!(!w.settled_failure());
    }

    #[test]
    fn a_failure_settles_after_ten_seconds_not_nine() {
        for outcome in [Outcome::NoRoom, Outcome::LookupFailed] {
            let mut w = placed_watch();
            assert_eq!(w.observe(outcome, LATER), v(false, false));
            assert_eq!(w.observe(outcome, LATER + SETTLE_SECS - 1), v(false, false));
            assert_eq!(w.observe(outcome, LATER + SETTLE_SECS), v(true, true));
        }
    }

    #[test]
    fn a_success_restarts_the_settle_clock() {
        let mut w = placed_watch();
        w.observe(Outcome::LookupFailed, LATER);
        w.observe(Outcome::Placed, LATER + 9);
        assert_eq!(
            w.observe(Outcome::LookupFailed, LATER + 10),
            v(false, false)
        );
        assert_eq!(
            w.observe(Outcome::LookupFailed, LATER + 19),
            v(false, false)
        );
        assert_eq!(w.observe(Outcome::LookupFailed, LATER + 20), v(true, true));
    }

    #[test]
    fn no_room_then_lookup_failed_is_one_continuous_failure() {
        let mut w = placed_watch();
        assert_eq!(w.observe(Outcome::NoRoom, LATER), v(false, false));
        assert_eq!(w.observe(Outcome::LookupFailed, LATER + 5), v(false, false));
        assert_eq!(w.observe(Outcome::LookupFailed, LATER + 10), v(true, true));
    }

    #[test]
    fn restart_opens_a_new_grace_period() {
        let mut w = placed_watch();
        w.observe(Outcome::NoRoom, LATER);
        assert!(w.observe(Outcome::NoRoom, LATER + 10).settled_failure);
        w.restart(LATER + 20);
        assert!(!w.settled_failure());
        for now in LATER + 20..LATER + 20 + GRACE_SECS {
            assert!(!w.observe(Outcome::NoRoom, now).settled_failure, "{now}");
        }
        // Settles again after the new grace period, but the one notice was already used.
        assert_eq!(
            w.observe(Outcome::NoRoom, LATER + 20 + GRACE_SECS),
            v(true, false)
        );
    }

    #[test]
    fn notifies_once_then_recovers_silently() {
        let mut w = placed_watch();
        w.observe(Outcome::NoRoom, LATER);
        assert_eq!(w.observe(Outcome::NoRoom, LATER + 10), v(true, true));
        assert_eq!(w.observe(Outcome::NoRoom, LATER + 11), v(true, false));
        assert_eq!(w.observe(Outcome::Placed, LATER + 12), v(false, false));
        w.observe(Outcome::NoRoom, LATER + 20);
        assert_eq!(w.observe(Outcome::NoRoom, LATER + 30), v(true, false));
    }

    #[test]
    fn a_startup_failure_opens_settings() {
        let mut w = PlacementWatch::default();
        assert_eq!(w.pending_notice(), None);
        w.observe(Outcome::NoRoom, 0);
        w.observe(Outcome::NoRoom, GRACE_SECS);
        assert_eq!(w.pending_notice(), Some(true));
        w.notice_sent();
        assert_eq!(w.pending_notice(), None);
    }

    #[test]
    fn a_runtime_failure_shows_only_the_balloon() {
        let mut w = placed_watch();
        w.observe(Outcome::NoRoom, LATER);
        w.observe(Outcome::NoRoom, LATER + 10);
        assert_eq!(w.pending_notice(), Some(false));
    }

    #[test]
    fn going_back_to_placed_drops_the_pending_notice() {
        let mut w = placed_watch();
        w.observe(Outcome::NoRoom, LATER);
        w.observe(Outcome::NoRoom, LATER + 10);
        w.observe(Outcome::Placed, LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(Outcome::NoRoom, LATER + 20);
        w.observe(Outcome::NoRoom, LATER + 30);
        assert_eq!(w.pending_notice(), None, "one notice per run");
    }

    #[test]
    fn a_pending_notice_waits_while_the_failure_is_not_settled() {
        let mut w = placed_watch();
        w.observe(Outcome::NoRoom, LATER);
        w.observe(Outcome::NoRoom, LATER + 10);
        w.restart(LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(Outcome::NoRoom, LATER + 11);
        w.observe(Outcome::NoRoom, LATER + 11 + GRACE_SECS);
        assert_eq!(w.pending_notice(), Some(false));
    }

    fn info(apps_right: Option<i32>, tray_left: Option<i32>) -> TaskbarInfo {
        let bar = Rect {
            left: 0,
            top: 1032,
            right: 1920,
            bottom: 1080,
        };
        TaskbarInfo {
            rect: bar,
            monitor: bar,
            dpi: 96,
            autohide: false,
            visible: true,
            left_aligned: false,
            windows_10: true,
            widgets_button_on: false,
            icons_left: None,
            apps_right,
            tray_left,
            side: Side::Right,
        }
    }

    #[test]
    fn left_side_never_feeds_the_watch() {
        for i in [info(None, None), info(Some(900), Some(1600))] {
            assert_eq!(outcome_of(Side::Left, &i, 0), None);
            assert_eq!(outcome_of(Side::Left, &i, 300), None);
        }
    }

    #[test]
    fn a_vertical_taskbar_never_feeds_the_watch() {
        let vertical = TaskbarInfo {
            rect: Rect {
                left: 1872,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            ..info(None, None)
        };
        assert_eq!(outcome_of(Side::Right, &vertical, 0), None);
        assert_eq!(outcome_of(Side::Right, &vertical, 300), None);
    }

    #[test]
    fn a_missing_edge_is_a_lookup_failure() {
        for i in [
            info(None, Some(1600)),
            info(Some(900), None),
            info(None, None),
        ] {
            assert_eq!(outcome_of(Side::Right, &i, 0), Some(Outcome::LookupFailed));
        }
    }

    #[test]
    fn both_edges_and_no_width_is_no_room() {
        let i = info(Some(1590), Some(1600));
        assert_eq!(outcome_of(Side::Right, &i, 0), Some(Outcome::NoRoom));
    }

    #[test]
    fn both_edges_and_a_width_is_placed() {
        let i = info(Some(900), Some(1600));
        assert_eq!(outcome_of(Side::Right, &i, 300), Some(Outcome::Placed));
    }
}
