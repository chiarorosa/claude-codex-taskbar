//! Placement watch (#10): turns each render's [`Placed`] into notices. A state other than
//! the preferred side (the other side, or hidden) raises one notice once it settles; it is
//! not repeated while that state lasts, nor after a return to the preferred side shorter
//! than [`SETTLE_SECS`], and returning to the preferred side is silent. A fallback for
//! `Auto` (never chosen in Settings) is silent too. Pure logic, no Win32: the
//! caller passes the time as whole seconds from a monotonic clock.

use super::taskbar::{Placed, Side, side_order};
use crate::config::Placement;

/// How long a non-preferred state must last, unchanged, before it settles.
pub const SETTLE_SECS: u64 = 10;

/// Nothing settles within this many seconds of startup or of the last
/// [`PlacementWatch::restart`] (Explorer and the taskbar are still settling then).
pub const GRACE_SECS: u64 = 30;

/// Whether the next taskbar query should read the right-side edges: when the right side is
/// preferred, or the left side had no room at the last render (or, before any render, the
/// icons are left-aligned). A left-side strip that fits never pays for the edge lookups.
pub fn wants_edges(
    placement: Placement,
    left_fitted_last: Option<bool>,
    left_aligned: bool,
) -> bool {
    side_order(placement)[0] == Side::Right
        || match left_fitted_last {
            Some(fitted) => !fitted,
            None => left_aligned,
        }
}

/// Whether a render without the right-side edges must leave the decision to a second pass
/// that reads them: on a horizontal taskbar, with something to show, when the left side has
/// no room. That pass neither hides the strip nor counts toward settling, so a strip that
/// moves to the right does not blink and never passes through `Hidden`.
pub fn needs_second_pass(
    horizontal: bool,
    edges_read: bool,
    has_items: bool,
    left_fits: bool,
) -> bool {
    horizontal && !edges_read && has_items && !left_fits
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

/// The placement state a notice kind is about.
fn state_of(kind: NoticeKind) -> Placed {
    match kind {
        NoticeKind::MovedRight => Placed::Fallback(Side::Right),
        NoticeKind::MovedLeft => Placed::Fallback(Side::Left),
        NoticeKind::Hidden => Placed::Hidden,
    }
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
    /// The last state that raised a notice; cleared once `Preferred` has held
    /// [`SETTLE_SECS`] and when an undelivered notice is dropped.
    last_notified: Option<Placed>,
    /// When the current run at the preferred side began.
    preferred_since: Option<u64>,
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
                // An undelivered notice is dropped at once and does not count as notified.
                if self.pending.take().is_some() {
                    self.last_notified = None;
                }
                // A brief return does not re-arm the notice for the same state.
                let since = *self.preferred_since.get_or_insert(now_secs);
                if now_secs.saturating_sub(since) >= SETTLE_SECS {
                    self.last_notified = None;
                }
            }
            Placed::Fallback(_) | Placed::Hidden => {
                self.preferred_since = None;
                if self.current.map(|(state, _)| state) != Some(placed) {
                    // After a restart `current` is empty and a pending notice is kept
                    // until the state settles again; a different state drops it, and an
                    // undelivered notice does not count as notified.
                    if self.current.is_some() && self.pending.take().is_some() {
                        self.last_notified = None;
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
        // A notice kept across a restart is for a state that may not be the one now
        // settled: drop it (undelivered, so not notified) before any silent return.
        if self.pending.is_some_and(|n| state_of(n.kind) != placed) {
            self.pending = None;
            self.last_notified = None;
        }
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

    /// The placement preference changed in Settings: forget everything but the current
    /// grace period (a click does not start a new one).
    pub fn reset(&mut self) {
        *self = Self {
            grace_from: self.grace_from,
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
        // The return holds long enough to count.
        w.observe(Preferred(Side::Left), true, LATER + 11);
        w.observe(Preferred(Side::Left), true, LATER + 11 + SETTLE_SECS);
        // A short flap does not settle, so it does not notify.
        w.observe(Fallback(Side::Right), true, LATER + 22);
        w.observe(Fallback(Side::Right), true, LATER + 31);
        assert_eq!(w.pending_notice(), None);
        w.observe(Fallback(Side::Right), true, LATER + 32);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn brief_return_does_not_renotify() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        w.notice_sent();
        // Back at the preferred side for 1 s only, then the same fallback again.
        w.observe(Preferred(Side::Left), true, LATER + 11);
        assert_eq!(w.pending_notice(), None);
        for now in LATER + 12..LATER + 60 {
            w.observe(Fallback(Side::Right), true, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
    }

    #[test]
    fn a_brief_return_drops_an_undelivered_notice_without_silencing_it() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        assert!(w.pending_notice().is_some());
        // Not delivered, then a 1 s return clears it at once.
        w.observe(Preferred(Side::Left), true, LATER + 11);
        assert_eq!(w.pending_notice(), None);
        w.observe(Fallback(Side::Right), true, LATER + 12);
        w.observe(Fallback(Side::Right), true, LATER + 12 + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn a_silent_settle_drops_a_stale_pending_notice() {
        let mut w = PlacementWatch::default();
        w.observe(Preferred(Side::Left), false, LATER - 1);
        w.observe(Hidden, false, LATER);
        w.observe(Hidden, false, LATER + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, false));
        // Explorer restarts before delivery; the auto fallback settles silently.
        w.restart(LATER + 11);
        for now in LATER + 11..=LATER + 11 + GRACE_SECS {
            w.observe(Fallback(Side::Right), false, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
    }

    #[test]
    fn needs_second_pass_table() {
        // (horizontal, edges read, has items, left fits) -> defer to the edge pass
        let table = [
            (true, false, true, false, true),
            (true, false, true, true, false),
            (true, true, true, false, false),
            (true, false, false, false, false),
            (false, false, true, false, false),
        ];
        for (horizontal, edges, items, fits, want) in table {
            assert_eq!(
                needs_second_pass(horizontal, edges, items, fits),
                want,
                "{horizontal} {edges} {items} {fits}"
            );
        }
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
        w.reset();
        // No memory and no new grace: the same fallback notifies again once settled.
        w.observe(Fallback(Side::Right), true, LATER + 20);
        w.observe(Fallback(Side::Right), true, LATER + 20 + SETTLE_SECS - 1);
        assert_eq!(w.pending_notice(), None);
        w.observe(Fallback(Side::Right), true, LATER + 20 + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
        // Hidden right after a reset counts as "before the strip was shown".
        let mut w = shown_watch();
        w.reset();
        w.observe(Hidden, true, LATER);
        w.observe(Hidden, true, LATER + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::Hidden, true));
        // A reset during the startup grace keeps that grace, it does not extend it.
        let mut w = PlacementWatch::default();
        w.reset();
        for now in 5..GRACE_SECS {
            w.observe(Fallback(Side::Right), true, now);
            assert_eq!(w.pending_notice(), None, "{now}");
        }
        w.observe(Fallback(Side::Right), true, GRACE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn a_dropped_pending_notice_does_not_silence_a_return_to_its_state() {
        let mut w = shown_watch();
        w.observe(Fallback(Side::Right), true, LATER);
        w.observe(Fallback(Side::Right), true, LATER + 10);
        assert!(w.pending_notice().is_some());
        // Not delivered (e.g. fullscreen app), then a short excursion drops it.
        w.observe(Hidden, true, LATER + 11);
        w.observe(Fallback(Side::Right), true, LATER + 15);
        w.observe(Fallback(Side::Right), true, LATER + 15 + SETTLE_SECS);
        assert_eq!(w.pending_notice(), notice(NoticeKind::MovedRight, false));
    }

    #[test]
    fn wants_edges_table() {
        use Placement::{Auto, Left, Right};
        // (placement, left fitted at the last render, left-aligned icons) -> read edges
        let table = [
            (Right, None, false, true),
            (Right, Some(true), false, true),
            (Right, Some(false), true, true),
            (Left, Some(true), false, false),
            (Left, Some(true), true, false),
            (Left, Some(false), false, true),
            (Left, None, false, false),
            (Left, None, true, true),
            (Auto, Some(true), false, false),
            (Auto, Some(false), false, true),
            (Auto, None, false, false),
            (Auto, None, true, true),
        ];
        for (placement, fitted, aligned, want) in table {
            assert_eq!(
                wants_edges(placement, fitted, aligned),
                want,
                "{placement:?} {fitted:?} {aligned}"
            );
        }
    }
}
