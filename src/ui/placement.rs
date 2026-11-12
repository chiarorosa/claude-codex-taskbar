//! Placement watch: turns per-attempt placement outcomes into a settled
//! failure flag and at most one notification per CCT run. Pure logic, no Win32.

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

    #[test]
    fn staying_settled_does_not_repeat_notify() {
        let mut w = PlacementWatch::default();
        assert_eq!(w.observe(Outcome::NoRoom), v(true, true));
        assert_eq!(w.observe(Outcome::NoRoom), v(true, false));
        assert_eq!(w.observe(Outcome::NoRoom), v(true, false));
    }
}
