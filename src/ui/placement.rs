//! Placement watch: turns per-attempt placement outcomes into a settled
//! failure flag and at most one notification per CCT run. Pure logic, no Win32: the
//! caller passes the time as whole seconds from a monotonic clock.

use super::taskbar::{Placed, Side, TaskbarInfo, is_horizontal};

/// How long a failure (`NoRoom` or `LookupFailed`, in any mix) must last, without a
/// `Placed` in between, before it settles.
pub const SETTLE_SECS: u64 = 10;

/// Nothing settles within this many seconds of startup or of the last
/// [`LegacyWatch::restart`] (Explorer and the taskbar are still settling then).
pub const GRACE_SECS: u64 = 30;

/// Result of one attempt to place the strip next to the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyOutcome {
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
pub fn legacy_outcome_of(
    side: Side,
    info: &TaskbarInfo,
    layout_width: i32,
) -> Option<LegacyOutcome> {
    match side {
        Side::Left => None,
        Side::Right if !is_horizontal(info.rect) => None,
        Side::Right if info.apps_right.is_none() || info.tray_left.is_none() => {
            Some(LegacyOutcome::LookupFailed)
        }
        Side::Right if layout_width == 0 => Some(LegacyOutcome::NoRoom),
        Side::Right => Some(LegacyOutcome::Placed),
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
pub struct LegacyWatch {
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

impl LegacyWatch {
    pub fn observe(&mut self, outcome: LegacyOutcome, now_secs: u64) -> Verdict {
        match outcome {
            LegacyOutcome::Placed => {
                self.failing_since = None;
                self.settled = false;
                self.ever_placed = true;
                self.pending = None;
            }
            LegacyOutcome::NoRoom | LegacyOutcome::LookupFailed => {
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

/// Which message a settled placement state raises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    MovedRight,
    MovedLeft,
    Hidden,
}

/// One notification to deliver; `open_settings` also opens the Settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub open_settings: bool,
}

/// Watches the placement of one CCT run. A non-preferred state (fallback side or hidden)
/// must hold [`SETTLE_SECS`], outside the [`GRACE_SECS`] grace, to settle; each settled
/// state different from the last notified one raises one notice.
#[derive(Default, Debug)]
pub struct PlacementWatch {
    /// Start of the current grace period.
    grace_from: u64,
    /// The current non-preferred state and when it began.
    current: Option<(Placed, u64)>,
    /// The current state, once it has held long enough.
    settled: Option<Placed>,
    /// The last state that raised (or was exempt from) a notice; cleared by `Preferred`.
    last_notified: Option<Placed>,
    /// The strip has been shown this run (since the last reset).
    ever_shown: bool,
    pending: Option<Notice>,
}

impl PlacementWatch {
    pub fn observe(&mut self, placed: Placed, explicit: bool, now_secs: u64) {
        match placed {
            Placed::Preferred(_) => {
                self.current = None;
                self.settled = None;
                self.pending = None;
                self.last_notified = None;
            }
            Placed::Fallback(_) | Placed::Hidden => {
                if self.current.map(|(state, _)| state) != Some(placed) {
                    // After a restart `current` is empty and a pending notice is kept
                    // until the state settles again; a different state drops it.
                    if self.current.is_some() {
                        self.pending = None;
                    }
                    self.current = Some((placed, now_secs));
                    self.settled = None;
                }
                let since = self.current.map_or(now_secs, |(_, since)| since);
                let past_grace = now_secs >= self.grace_from.saturating_add(GRACE_SECS);
                if past_grace
                    && now_secs.saturating_sub(since) >= SETTLE_SECS
                    && self.settled.is_none()
                {
                    self.settled = Some(placed);
                    self.settle(placed, explicit);
                }
            }
        }
        // Set last: a Hidden notice asks whether the strip was shown *before* it.
        if !matches!(placed, Placed::Hidden) {
            self.ever_shown = true;
        }
    }

    fn settle(&mut self, placed: Placed, explicit: bool) {
        if self.last_notified == Some(placed) {
            return;
        }
        let kind = match placed {
            Placed::Fallback(_) if !explicit => return,
            Placed::Fallback(Side::Right) => NoticeKind::MovedRight,
            Placed::Fallback(Side::Left) => NoticeKind::MovedLeft,
            Placed::Hidden => NoticeKind::Hidden,
            Placed::Preferred(_) => return,
        };
        self.last_notified = Some(placed);
        self.pending = Some(Notice {
            kind,
            // Only the startup decision opens Settings; a state that settles after the
            // strip was shown gets the balloon alone.
            open_settings: kind == NoticeKind::Hidden && !self.ever_shown,
        });
    }

    /// The notice still to be delivered, while its state is still the settled one.
    pub fn pending_notice(&self) -> Option<Notice> {
        self.pending.filter(|_| self.settled.is_some())
    }

    /// The pending notice has been delivered.
    pub fn notice_sent(&mut self) {
        self.pending = None;
    }

    /// Explorer restarted (`TaskbarCreated`): forget the current state and start a new
    /// grace period; the notification history stays.
    pub fn restart(&mut self, now_secs: u64) {
        self.grace_from = now_secs;
        self.current = None;
        self.settled = None;
    }

    /// The placement preference changed in Settings: forget everything and start a new
    /// grace period.
    pub fn reset(&mut self, now_secs: u64) {
        *self = Self {
            grace_from: now_secs,
            ..Self::default()
        };
    }

    /// Neither side has had room long enough.
    pub fn hidden_settled(&self) -> bool {
        self.settled == Some(Placed::Hidden)
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
    fn placed_watch() -> LegacyWatch {
        let mut w = LegacyWatch::default();
        assert_eq!(w.observe(LegacyOutcome::Placed, LATER - 1), v(false, false));
        w
    }

    #[test]
    fn nothing_settles_during_the_startup_grace_period() {
        let mut w = LegacyWatch::default();
        for now in 0..GRACE_SECS {
            assert_eq!(
                w.observe(LegacyOutcome::NoRoom, now),
                v(false, false),
                "{now}"
            );
        }
        // Failing since 0: the failure settles as soon as the grace period ends.
        assert_eq!(w.observe(LegacyOutcome::NoRoom, GRACE_SECS), v(true, true));
        assert!(w.settled_failure());
    }

    #[test]
    fn a_transient_failure_during_grace_never_settles() {
        let mut w = LegacyWatch::default();
        for now in 0..25 {
            assert!(!w.observe(LegacyOutcome::LookupFailed, now).notify);
        }
        assert_eq!(w.observe(LegacyOutcome::Placed, 25), v(false, false));
        assert_eq!(
            w.observe(LegacyOutcome::Placed, GRACE_SECS + 5),
            v(false, false)
        );
        // A new failure after grace starts its own clock.
        assert_eq!(w.observe(LegacyOutcome::NoRoom, 40), v(false, false));
        assert_eq!(w.observe(LegacyOutcome::NoRoom, 49), v(false, false));
        assert!(!w.settled_failure());
    }

    #[test]
    fn a_failure_settles_after_ten_seconds_not_nine() {
        for outcome in [LegacyOutcome::NoRoom, LegacyOutcome::LookupFailed] {
            let mut w = placed_watch();
            assert_eq!(w.observe(outcome, LATER), v(false, false));
            assert_eq!(w.observe(outcome, LATER + SETTLE_SECS - 1), v(false, false));
            assert_eq!(w.observe(outcome, LATER + SETTLE_SECS), v(true, true));
        }
    }

    #[test]
    fn a_success_restarts_the_settle_clock() {
        let mut w = placed_watch();
        w.observe(LegacyOutcome::LookupFailed, LATER);
        w.observe(LegacyOutcome::Placed, LATER + 9);
        assert_eq!(
            w.observe(LegacyOutcome::LookupFailed, LATER + 10),
            v(false, false)
        );
        assert_eq!(
            w.observe(LegacyOutcome::LookupFailed, LATER + 19),
            v(false, false)
        );
        assert_eq!(
            w.observe(LegacyOutcome::LookupFailed, LATER + 20),
            v(true, true)
        );
    }

    #[test]
    fn no_room_then_lookup_failed_is_one_continuous_failure() {
        let mut w = placed_watch();
        assert_eq!(w.observe(LegacyOutcome::NoRoom, LATER), v(false, false));
        assert_eq!(
            w.observe(LegacyOutcome::LookupFailed, LATER + 5),
            v(false, false)
        );
        assert_eq!(
            w.observe(LegacyOutcome::LookupFailed, LATER + 10),
            v(true, true)
        );
    }

    #[test]
    fn restart_opens_a_new_grace_period() {
        let mut w = placed_watch();
        w.observe(LegacyOutcome::NoRoom, LATER);
        assert!(w.observe(LegacyOutcome::NoRoom, LATER + 10).settled_failure);
        w.restart(LATER + 20);
        assert!(!w.settled_failure());
        for now in LATER + 20..LATER + 20 + GRACE_SECS {
            assert!(
                !w.observe(LegacyOutcome::NoRoom, now).settled_failure,
                "{now}"
            );
        }
        // Settles again after the new grace period, but the one notice was already used.
        assert_eq!(
            w.observe(LegacyOutcome::NoRoom, LATER + 20 + GRACE_SECS),
            v(true, false)
        );
    }

    #[test]
    fn notifies_once_then_recovers_silently() {
        let mut w = placed_watch();
        w.observe(LegacyOutcome::NoRoom, LATER);
        assert_eq!(w.observe(LegacyOutcome::NoRoom, LATER + 10), v(true, true));
        assert_eq!(w.observe(LegacyOutcome::NoRoom, LATER + 11), v(true, false));
        assert_eq!(
            w.observe(LegacyOutcome::Placed, LATER + 12),
            v(false, false)
        );
        w.observe(LegacyOutcome::NoRoom, LATER + 20);
        assert_eq!(w.observe(LegacyOutcome::NoRoom, LATER + 30), v(true, false));
    }

    #[test]
    fn a_startup_failure_opens_settings() {
        let mut w = LegacyWatch::default();
        assert_eq!(w.pending_notice(), None);
        w.observe(LegacyOutcome::NoRoom, 0);
        w.observe(LegacyOutcome::NoRoom, GRACE_SECS);
        assert_eq!(w.pending_notice(), Some(true));
        w.notice_sent();
        assert_eq!(w.pending_notice(), None);
    }

    #[test]
    fn a_runtime_failure_shows_only_the_balloon() {
        let mut w = placed_watch();
        w.observe(LegacyOutcome::NoRoom, LATER);
        w.observe(LegacyOutcome::NoRoom, LATER + 10);
        assert_eq!(w.pending_notice(), Some(false));
    }

    #[test]
    fn going_back_to_placed_drops_the_pending_notice() {
        let mut w = placed_watch();
        w.observe(LegacyOutcome::NoRoom, LATER);
        w.observe(LegacyOutcome::NoRoom, LATER + 10);
        w.observe(LegacyOutcome::Placed, LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(LegacyOutcome::NoRoom, LATER + 20);
        w.observe(LegacyOutcome::NoRoom, LATER + 30);
        assert_eq!(w.pending_notice(), None, "one notice per run");
    }

    #[test]
    fn a_pending_notice_waits_while_the_failure_is_not_settled() {
        let mut w = placed_watch();
        w.observe(LegacyOutcome::NoRoom, LATER);
        w.observe(LegacyOutcome::NoRoom, LATER + 10);
        w.restart(LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(LegacyOutcome::NoRoom, LATER + 11);
        w.observe(LegacyOutcome::NoRoom, LATER + 11 + GRACE_SECS);
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
            assert_eq!(legacy_outcome_of(Side::Left, &i, 0), None);
            assert_eq!(legacy_outcome_of(Side::Left, &i, 300), None);
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
        assert_eq!(legacy_outcome_of(Side::Right, &vertical, 0), None);
        assert_eq!(legacy_outcome_of(Side::Right, &vertical, 300), None);
    }

    #[test]
    fn a_missing_edge_is_a_lookup_failure() {
        for i in [
            info(None, Some(1600)),
            info(Some(900), None),
            info(None, None),
        ] {
            assert_eq!(
                legacy_outcome_of(Side::Right, &i, 0),
                Some(LegacyOutcome::LookupFailed)
            );
        }
    }

    #[test]
    fn both_edges_and_no_width_is_no_room() {
        let i = info(Some(1590), Some(1600));
        assert_eq!(
            legacy_outcome_of(Side::Right, &i, 0),
            Some(LegacyOutcome::NoRoom)
        );
    }

    #[test]
    fn both_edges_and_a_width_is_placed() {
        let i = info(Some(900), Some(1600));
        assert_eq!(
            legacy_outcome_of(Side::Right, &i, 300),
            Some(LegacyOutcome::Placed)
        );
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use crate::ui::taskbar::Placed::{Fallback, Hidden, Preferred};

    const LATER: u64 = 100;

    fn notice(kind: NoticeKind, open_settings: bool) -> Option<Notice> {
        Some(Notice {
            kind,
            open_settings,
        })
    }

    /// A watch that has shown the strip at its preferred side, well after startup.
    fn shown_watch() -> PlacementWatch {
        let mut w = PlacementWatch::default();
        w.observe(Preferred(Side::Left), true, LATER - 1);
        w
    }

    #[test]
    fn fallback_settles_after_10s_and_notifies_moved_right() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + SETTLE_SECS - 1);
        assert_eq!(w.pending_notice(), None);
        w.observe(Fallback(Side::Right), true, LATER + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
        assert!(!w.hidden_settled());
        // Mirror image.
        let mut w = shown_watch();
        w.observe(Fallback(Side::Left), true, LATER);
        w.observe(Fallback(Side::Left), true, LATER + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedLeft, false));
    }

    #[test]
    fn nothing_settles_during_the_startup_grace_period() {
        let mut w = PlacementWatch::default();
        for now in 0..GRACE_SECS {
            w.observe(Fallback(Side::Right), true, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
        w.observe(Fallback(Side::Right), true, GRACE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn auto_fallback_is_silent() {
        let mut w = shown_watch();
        for now in LATER..LATER + 60 {
            w.observe(Fallback(Side::Right), false, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
        w.observe(Hidden, false, LATER + 60);
        w.observe(Hidden, false, LATER + 60 + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, false));
        assert!(w.hidden_settled());
    }

    #[test]
    fn return_to_preferred_is_silent_and_clears() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + SETTLE_SECS);
        assert!(w.pending_notice().is_some());
        w.observe(Preferred(Side::Left), true, LATER + 11);
        assert_eq!(w.pending_notice(), None);
        assert!(!w.hidden_settled());
    }

    #[test]
    fn fallback_then_back_then_fallback_notifies_twice_only_when_settled() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        assert!(w.pending_notice().is_some());
        w.notice_sent();
        w.observe(Preferred(Side::Left), true, LATER + 11);
        // A short flap does not settle, so it does not notify.
        w.observe(Fallback(Side::Right), true, LATER + 20);
        w.observe(Fallback(Side::Right), true, LATER + 29);
        assert_eq!(w.pending_notice(), None);
        w.observe(Fallback(Side::Right), true, LATER + 30);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn fallback_to_hidden_notifies_again() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        w.notice_sent();
        w.observe(Hidden, true, LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(Hidden, true, LATER + 21);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, false));
    }

    #[test]
    fn staying_in_state_does_not_repeat() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        w.notice_sent();
        for now in LATER + 11..LATER + 40 {
            w.observe(Fallback(Side::Right), true, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
    }

    #[test]
    fn hidden_at_startup_opens_settings_runtime_does_not() {
        let mut w = PlacementWatch::default();
        w.observe(Hidden, true, 0);
        w.observe(Hidden, true, GRACE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, true));
        w.notice_sent();
        assert_eq!(w.pending_notice(), None);

        let mut w = shown_watch();
        w.observe(Hidden, true, LATER);
        w.observe(Hidden, true, LATER + 10);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, false));
    }

    #[test]
    fn restart_opens_new_grace() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        w.notice_sent();
        w.restart(LATER + 20);
        assert!(!w.hidden_settled());
        for now in LATER + 20..LATER + 20 + GRACE_SECS {
            w.observe(Fallback(Side::Right), true, now);
        }
        w.observe(Fallback(Side::Right), true, LATER + 20 + GRACE_SECS);
        // Settled again, but this state was already notified.
        assert_eq!(w.pending_notice(), None);
    }

    #[test]
    fn a_pending_notice_waits_while_not_settled() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        w.restart(LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(Fallback(Side::Right), true, LATER + 11);
        w.observe(Fallback(Side::Right), true, LATER + 11 + GRACE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn reset_forgets_state() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        w.notice_sent();
        w.reset(LATER + 20);
        // New grace and no memory: the same fallback notifies again once settled.
        for now in LATER + 20..LATER + 20 + GRACE_SECS {
            w.observe(Fallback(Side::Right), true, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
        w.observe(Fallback(Side::Right), true, LATER + 20 + GRACE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
        // Hidden right after a reset counts as "before the strip was shown".
        let mut w = shown_watch();
        w.reset(LATER);
        w.observe(Hidden, true, LATER + GRACE_SECS);
        w.observe(Hidden, true, LATER + GRACE_SECS + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, true));
    }
}
