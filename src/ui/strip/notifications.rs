//! Deliver owned batches only after releasing App and temporary-icon state borrows.

use super::{WM_APP_REASSERT, with_app};
use crate::i18n::{Key, t};
use crate::ui::app::foreground_is_fullscreen;
use crate::ui::notifications::shell::{WindowsNotificationSink, decode_callback, timer_icon};
use crate::ui::notifications::{
    BalloonCommand, BalloonLife, NotificationSink, balloon_text, send_balloon,
};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    NIN_BALLOONHIDE, NIN_BALLOONSHOW, NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, WM_APP, WM_DESTROY, WM_SETTINGCHANGE, WM_TIMER,
};

const CALLBACK: u32 = WM_APP + 4;

thread_local! {
    static LIFE: RefCell<BalloonLife> = RefCell::new(BalloonLife::default());
    static CLOSING: Cell<bool> = const { Cell::new(false) };
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
    enabled
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
    if let Some(old) = old {
        shell.remove(old);
    }
    if !send_balloon(&shell, id, title, &body, || current(hwnd, id)) {
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
