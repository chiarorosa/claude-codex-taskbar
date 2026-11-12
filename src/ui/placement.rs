//! Placement watch: turns per-attempt placement outcomes into a settled
//! failure flag and at most one notification per CCT run. Pure logic, no Win32.

use super::taskbar::{Side, TaskbarInfo};

/// Consecutive `LookupFailed` observations needed before the failure settles.
pub const LOOKUP_FAILS_TO_SETTLE: u32 = 3;

/// Result of one attempt to place the strip next to the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Placed,
    /// Both edges were read but the gap is too small.
    NoRoom,
    /// The tray geometry could not be read.
    LookupFailed,
}

/// What one uncovered render means for the watch: `None` on the left side, which hides
/// silently as before and never notifies; on the right, a missing edge is a failed lookup and
/// a layout with no width (both edges read) means the gap is too small.
pub fn outcome_of(side: Side, info: &TaskbarInfo, layout_width: i32) -> Option<Outcome> {
    match side {
        Side::Left => None,
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

/// Lives for one CCT run.
#[derive(Default, Debug)]
pub struct PlacementWatch {
    consecutive_failures: u32,
    settled: bool,
    notified: bool,
}

impl PlacementWatch {
    pub fn observe(&mut self, outcome: Outcome) -> Verdict {
        match outcome {
            Outcome::Placed => {
                self.consecutive_failures = 0;
                self.settled = false;
            }
            Outcome::NoRoom => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                self.settled = true;
            }
            Outcome::LookupFailed => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                if self.consecutive_failures >= LOOKUP_FAILS_TO_SETTLE {
                    self.settled = true;
                }
            }
        }
        let notify = self.settled && !self.notified;
        if notify {
            self.notified = true;
        }
        Verdict {
            settled_failure: self.settled,
            notify,
        }
    }

    pub fn settled_failure(&self) -> bool {
        self.settled
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

    #[test]
    fn no_room_settles_and_notifies_immediately() {
        let mut w = PlacementWatch::default();
        assert_eq!(w.observe(Outcome::NoRoom), v(true, true));
        assert!(w.settled_failure());
    }

    #[test]
    fn lookup_failures_settle_after_three_in_a_row() {
        let mut w = PlacementWatch::default();
        assert_eq!(w.observe(Outcome::LookupFailed), v(false, false));
        assert_eq!(w.observe(Outcome::LookupFailed), v(false, false));
        assert_eq!(w.observe(Outcome::LookupFailed), v(true, true));
    }

    #[test]
    fn transient_failures_then_success_never_notify() {
        let mut w = PlacementWatch::default();
        for o in [
            Outcome::LookupFailed,
            Outcome::LookupFailed,
            Outcome::Placed,
            Outcome::LookupFailed,
            Outcome::LookupFailed,
        ] {
            assert!(!w.observe(o).notify);
        }
        assert!(!w.settled_failure());
    }

    #[test]
    fn no_room_notifies_once_then_recovers_silently() {
        let mut w = PlacementWatch::default();
        assert_eq!(w.observe(Outcome::NoRoom), v(true, true));
        assert_eq!(w.observe(Outcome::Placed), v(false, false));
        assert_eq!(w.observe(Outcome::NoRoom), v(true, false));
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

    #[test]
    fn staying_settled_does_not_repeat_notify() {
        let mut w = PlacementWatch::default();
        assert_eq!(w.observe(Outcome::NoRoom), v(true, true));
        assert_eq!(w.observe(Outcome::NoRoom), v(true, false));
        assert_eq!(w.observe(Outcome::NoRoom), v(true, false));
    }
}
