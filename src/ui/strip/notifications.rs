//! Deliver owned batches only after releasing App and temporary-icon state borrows.

use super::{WM_APP_REASSERT, with_app};
use crate::i18n::{Key, t};
use crate::ui::app::foreground_is_fullscreen;
use crate::ui::notifications::shell::{WindowsNotificationSink, decode_callback, timer_icon};
use crate::ui::notifications::{
    BalloonCommand, BalloonLife, NotificationSink, balloon_text, send_balloon,
};
use crate::ui::placement::NoticeKind;
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    NIN_BALLOONHIDE, NIN_BALLOONSHOW, NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, WM_APP, WM_DESTROY, WM_SETTINGCHANGE, WM_TIMER,
};

/// Balloon callbacks; WM_APP + 4 is the switcher's result message.
pub(super) const CALLBACK: u32 = WM_APP + 5;

thread_local! {
    static LIFE: RefCell<BalloonLife> = RefCell::new(BalloonLife::default());
    static CLOSING: Cell<bool> = const { Cell::new(false) };
    /// The placement notice's icon (#10): it does not depend on the near-limit opt-in.
    static PLACEMENT: Cell<Option<u16>> = const { Cell::new(None) };
}

fn sink(hwnd: HWND) -> WindowsNotificationSink {
    WindowsNotificationSink {
        hwnd,
        callback: CALLBACK,
        shown: |id| LIFE.with(|life| life.borrow().is_shown(id)),
    }
}

fn current(hwnd: HWND, id: u16) -> bool {
    // SAFETY: IsWindow is a read-only validity check, including during owner destruction.
    if CLOSING.with(Cell::get) || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return false;
    }
    let Some((enabled, monitor)) = with_app(|app| {
        (
            app.cfg.notify_near_limit,
            app.taskbar.as_ref().map(|t| t.monitor),
        )
    }) else {
        return false;
    };
    (enabled || PLACEMENT.with(Cell::get) == Some(id))
        && !monitor.is_some_and(foreground_is_fullscreen)
        && LIFE.with(|life| life.borrow().is_current(id))
}

pub(super) fn cancel(hwnd: HWND) {
    let id = LIFE.with(|life| life.borrow_mut().cancel());
    if let Some(id) = id {
        sink(hwnd).remove(id);
    }
}

pub(super) fn drain(hwnd: HWND) {
    if CLOSING.with(Cell::get) {
        return;
    }
    let Some(notices) = with_app(|app| app.drain_limits()).filter(|n| !n.is_empty()) else {
        return;
    };
    let shell = sink(hwnd);
    if !shell.accepts() {
        cancel(hwnd);
        return;
    }
    let text = with_app(|app| {
        balloon_text(app.lang, &app.cfg, &notices).map(|body| (t(app.lang, Key::NotifyTitle), body))
    })
    .flatten();
    let Some((title, body)) = text else { return };
    let (old, id) = LIFE.with(|life| life.borrow_mut().begin());
    PLACEMENT.with(|p| p.set(None));
    if let Some(old) = old {
        shell.remove(old);
    }
    if !send_balloon(&shell, id, title, &body, || current(hwnd, id)) {
        LIFE.with(|life| life.borrow_mut().finish(id));
    }
}

/// Whether the placement notice could be shown now: CCT is not closing, the shell accepts
/// notifications and no fullscreen app covers the taskbar's monitor. Until then the notice
/// stays pending (#10).
pub(super) fn placement_deliverable(hwnd: HWND) -> bool {
    if CLOSING.with(Cell::get) || !sink(hwnd).accepts() {
        return false;
    }
    let monitor = with_app(|app| app.taskbar.as_ref().map(|t| t.monitor)).flatten();
    !monitor.is_some_and(foreground_is_fullscreen)
}

/// Whether a balloon callback is a click on the placement notice, which opens Settings.
fn opens_settings(event: u32, id: u16, placement: Option<u16>) -> bool {
    event == NIN_BALLOONUSERCLICK && placement == Some(id)
}

/// The message for each placement notice; all share [`Key::NotifyPlacementTitle`].
fn placement_body(kind: NoticeKind) -> Key {
    match kind {
        NoticeKind::MovedRight => Key::NotifyMovedRight,
        NoticeKind::MovedLeft => Key::NotifyMovedLeft,
        NoticeKind::Hidden => Key::NotifyPlacementBody,
    }
}

/// Sends a placement notice (#10). Unlike limit notices it is not behind the near-limit
/// opt-in; like them it is dropped over a fullscreen app or when CCT closes.
pub(super) fn send_placement(hwnd: HWND, kind: NoticeKind) {
    if CLOSING.with(Cell::get) {
        return;
    }
    let shell = sink(hwnd);
    if !shell.accepts() {
        return;
    }
    let Some(lang) = with_app(|app| app.lang) else {
        return;
    };
    let (old, id) = LIFE.with(|life| life.borrow_mut().begin());
    if let Some(old) = old {
        shell.remove(old);
    }
    PLACEMENT.with(|p| p.set(Some(id)));
    let title = t(lang, Key::NotifyPlacementTitle);
    let body = t(lang, placement_body(kind));
    if !send_balloon(&shell, id, title, body, || current(hwnd, id)) {
        LIFE.with(|life| life.borrow_mut().finish(id));
    }
}

fn finish(hwnd: HWND, id: u16) {
    LIFE.with(|life| life.borrow_mut().finish(id));
    sink(hwnd).remove(id);
}

pub(super) fn handle(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        CALLBACK => {
            let (id, event) = decode_callback(lparam);
            if event == NIN_BALLOONSHOW
                && LIFE.with(|life| life.borrow_mut().mark_shown(id))
                && (!current(hwnd, id) || !sink(hwnd).send(BalloonCommand::ArmCleanup, id, "", ""))
            {
                finish(hwnd, id);
            }
            if [NIN_BALLOONHIDE, NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK].contains(&event) {
                finish(hwnd, id);
            }
            if opens_settings(event, id, PLACEMENT.with(Cell::get)) {
                crate::ui::settings::open(hwnd);
            }
            Some(LRESULT(0))
        }
        WM_TIMER if timer_icon(wparam.0).is_some() => {
            let id = timer_icon(wparam.0).unwrap();
            finish(hwnd, id);
            // A killed timer can still have an old WM_TIMER queued; its ID is independent.
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            CLOSING.with(|closing| closing.set(true));
            cancel(hwnd);
            None
        }
        WM_SETTINGCHANGE | WM_APP_REASSERT => {
            let active = LIFE.with(|life| life.borrow().active_id());
            if active.is_some_and(|id| !sink(hwnd).accepts() || !current(hwnd, id)) {
                cancel(hwnd);
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_placement_notice_has_its_own_body() {
        assert_eq!(
            placement_body(NoticeKind::MovedRight),
            Key::NotifyMovedRight
        );
        assert_eq!(placement_body(NoticeKind::MovedLeft), Key::NotifyMovedLeft);
        assert_eq!(placement_body(NoticeKind::Hidden), Key::NotifyPlacementBody);
    }

    #[test]
    fn only_a_click_on_the_placement_balloon_opens_settings() {
        assert!(opens_settings(NIN_BALLOONUSERCLICK, 7, Some(7)));
        // A limit balloon, or no placement balloon at all.
        assert!(!opens_settings(NIN_BALLOONUSERCLICK, 7, Some(8)));
        assert!(!opens_settings(NIN_BALLOONUSERCLICK, 7, None));
        // Shown, closed or timed out: not a click.
        for event in [NIN_BALLOONSHOW, NIN_BALLOONHIDE, NIN_BALLOONTIMEOUT] {
            assert!(!opens_settings(event, 7, Some(7)), "{event}");
        }
    }
}
