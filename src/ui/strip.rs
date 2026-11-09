//! The strip window: creation, message handling, timers, tooltip and context menu.

use super::app::{App, spawn_worker};
use crate::i18n::{Key, t};
use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Controls::{
    ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, NMTTDISPINFOW, TOOLTIPS_CLASSW,
    TTDT_AUTOPOP, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETDELAYTIME, TTM_SETMAXTIPWIDTH,
    TTN_GETDISPINFOW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_DBLCLKS, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyMenu, DestroyWindow, DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetCursorPos,
    GetMessageW, HWND_TOPMOST, IDC_ARROW, IDYES, KillTimer, LoadCursorW, MB_DEFBUTTON2,
    MB_ICONQUESTION, MB_SETFOREGROUND, MB_YESNO, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, PBT_APMRESUMEAUTOMATIC, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINEVENT_OUTOFCONTEXT,
    WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_LBUTTONDBLCLK, WM_NOTIFY,
    WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, PWSTR, w};

pub const WM_APP_LIMITS: u32 = WM_APP + 1;
pub const WM_APP_REASSERT: u32 = WM_APP + 2;
/// Sent by a second launch of the exe: the running instance opens Settings.
pub const WM_APP_OPEN_SETTINGS: u32 = WM_APP + 3;

const TIMER_HW: usize = 1;
const TIMER_TOPMOST: usize = 2;
const TIMER_DISKS: usize = 3;
/// One-shot: refresh limits right after a known reset.
const TIMER_RESET: usize = 4;
const TOPMOST_MS: u32 = 2000;
const DISKS_MS: u32 = 30_000;

const CMD_SETTINGS: usize = 1;
const CMD_REFRESH: usize = 2;
const CMD_AUTOSTART: usize = 3;
const CMD_EXIT: usize = 4;
const CMD_COPY: usize = 5;

mod notifications;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TIP_TEXT: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
}

static STRIP: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicIsize = AtomicIsize::new(0);

/// Runs `f` on the app unless it is already borrowed (re-entrant message during a modal loop).
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn show(hwnd: HWND, app: &mut App) {
    if !app.shown {
        // SAFETY: hwnd is our live window.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        app.shown = true;
    }
    reassert_topmost(hwnd);
}

pub fn hide(hwnd: HWND, app: &mut App) {
    if app.shown {
        // SAFETY: hwnd is our live window.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        app.shown = false;
    }
}

fn reassert_topmost(hwnd: HWND) {
    // SAFETY: hwnd is our live window.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

fn render(hwnd: HWND) {
    let Some(need_icons) = with_app(|app| app.needs_icons()) else {
        return;
    };
    let info = super::taskbar::query(need_icons);
    with_app(|app| app.render(hwnd, info, need_icons));
}

fn set_hw_timer(hwnd: HWND, secs: u32) {
    // SAFETY: hwnd is our live window; re-setting an id replaces the timer.
    unsafe { SetTimer(Some(hwnd), TIMER_HW, secs * 1000, None) };
}

fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn show_menu(hwnd: HWND) {
    let Some((lang, autostart)) = with_app(|app| (app.lang, crate::autostart::is_enabled())) else {
        return;
    };
    // SAFETY: menu is created, tracked and destroyed here; the app is not borrowed meanwhile.
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let item = |id: usize, key: Key, checked: bool| {
            let text = wide_z(t(lang, key));
            let flags = if checked {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        };
        item(CMD_SETTINGS, Key::MenuSettings, false);
        item(CMD_REFRESH, Key::MenuRefresh, false);
        item(CMD_COPY, Key::MenuCopyUsage, false);
        item(CMD_AUTOSTART, Key::MenuAutostart, autostart);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(CMD_EXIT, Key::MenuExit, false);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        cmd.0 as usize
    };
    match cmd {
        CMD_SETTINGS => super::settings::open(hwnd),
        CMD_REFRESH => {
            with_app(|app| app.control.wake());
        }
        CMD_COPY => {
            if let Some(text) = with_app(|app| app.tooltip()) {
                let _ = super::clipboard::set_text(hwnd, &text);
            }
        }
        CMD_AUTOSTART => {
            let _ = if autostart {
                crate::autostart::disable()
            } else {
                crate::autostart::enable()
            };
        }
        CMD_EXIT => {
            // SAFETY: hwnd is our live window.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        _ => {}
    }
}

/// (Re)arms the one-shot refresh after the next known reset, or cancels it when none is due
/// before the regular refresh. Re-arming replaces the pending timer, so each reset costs at
/// most one extra request.
fn arm_reset_timer(hwnd: HWND) {
    let delay = with_app(|app| app.reset_refresh_delay()).flatten();
    // SAFETY: the timer belongs to this window.
    unsafe {
        match delay {
            Some(secs) => {
                SetTimer(Some(hwnd), TIMER_RESET, secs * 1000, None);
            }
            None => {
                let _ = KillTimer(Some(hwnd), TIMER_RESET);
            }
        }
    }
}

/// Called by the settings window after the user applies changes.
pub fn config_applied(hwnd: HWND, cfg: crate::config::Config) {
    let secs = cfg.hw_interval_secs;
    with_app(|app| app.apply_config(cfg));
    notifications::cancel(hwnd);
    set_hw_timer(hwnd, secs.clamp(1, 5));
    render(hwnd);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result) = notifications::handle(hwnd, msg, wparam, lparam) {
        return result;
    }
    let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed) as u32;
    match msg {
        WM_TIMER => {
            match wparam.0 {
                TIMER_HW => {
                    with_app(|app| app.sample_hw());
                    // Picks up results whose notification arrived while the app was busy.
                    notifications::drain(hwnd);
                }
                TIMER_TOPMOST => {
                    with_app(|app| app.tick_icons());
                }
                TIMER_DISKS => {
                    with_app(|app| app.refresh_instances());
                }
                TIMER_RESET => {
                    // SAFETY: the timer belongs to this window.
                    unsafe {
                        let _ = KillTimer(Some(hwnd), TIMER_RESET);
                    }
                    with_app(|app| app.control.wake());
                }
                _ => {}
            }
            render(hwnd);
            LRESULT(0)
        }
        WM_APP_LIMITS => {
            notifications::drain(hwnd);
            arm_reset_timer(hwnd);
            render(hwnd);
            LRESULT(0)
        }
        WM_APP_REASSERT => {
            // Foreground changes coincide with apps opening/closing, which shifts the icons.
            with_app(|app| app.invalidate_icons());
            render(hwnd);
            LRESULT(0)
        }
        WM_APP_OPEN_SETTINGS => {
            super::settings::open(hwnd);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_app(|app| app.invalidate_icons());
            render(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            with_app(|app| {
                app.theme_changed();
                app.invalidate_icons();
            });
            render(hwnd);
            LRESULT(0)
        }
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => {
            // After sleep the cached limits are old: fetch right away.
            with_app(|app| app.control.wake());
            render(hwnd);
            LRESULT(1)
        }
        WM_RBUTTONUP => {
            show_menu(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            // Same as Refresh now; a single left click still does nothing.
            with_app(|app| app.control.wake());
            LRESULT(0)
        }
        WM_NOTIFY => {
            // SAFETY: WM_NOTIFY's lparam points to an NMHDR-prefixed struct.
            let hdr = unsafe { &*(lparam.0 as *const windows::Win32::UI::Controls::NMHDR) };
            if hdr.code == TTN_GETDISPINFOW {
                let text = with_app(|app| app.tooltip()).unwrap_or_default();
                TIP_TEXT.with(|tip| {
                    let mut tip = tip.borrow_mut();
                    *tip = wide_z(&text);
                    // SAFETY: lparam is an NMTTDISPINFOW for TTN_GETDISPINFOW; the buffer lives
                    // in a thread-local until the next notification.
                    unsafe {
                        let info = &mut *(lparam.0 as *mut NMTTDISPINFOW);
                        info.lpszText = PWSTR(tip.as_mut_ptr());
                    }
                });
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            with_app(|app| app.control.stop());
            // SAFETY: timers belong to this window.
            unsafe {
                for id in [TIMER_HW, TIMER_TOPMOST, TIMER_DISKS, TIMER_RESET] {
                    let _ = KillTimer(Some(hwnd), id);
                }
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ if taskbar_created != 0 && msg == taskbar_created => {
            notifications::cancel(hwnd);
            with_app(|app| app.invalidate_icons());
            render(hwnd);
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let strip = STRIP.load(Ordering::Relaxed);
    if strip != 0 {
        // SAFETY: posting to our own window; failure is harmless.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(strip as _)),
                WM_APP_REASSERT,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

fn create_tooltip(hwnd: HWND) {
    // SAFETY: standard tooltip setup on our window.
    unsafe {
        let Ok(tip) = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            PCWSTR::null(),
            WS_POPUP
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    TTS_NOPREFIX | TTS_ALWAYSTIP,
                ),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(hwnd),
            None,
            None,
            None,
        ) else {
            return;
        };
        let ti = TTTOOLINFOW {
            cbSize: size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd,
            uId: hwnd.0 as usize,
            // LPSTR_TEXTCALLBACKW: ask for text via TTN_GETDISPINFOW.
            lpszText: PWSTR(-1isize as *mut u16),
            ..Default::default()
        };
        SendMessageW(
            tip,
            TTM_ADDTOOLW,
            None,
            Some(LPARAM(&ti as *const _ as isize)),
        );
        SendMessageW(tip, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(420)));
        SendMessageW(
            tip,
            TTM_SETDELAYTIME,
            Some(WPARAM(TTDT_AUTOPOP as usize)),
            Some(LPARAM(30_000)),
        );
    }
}

/// Creates the window, starts timers and the worker, and runs the message loop.
pub fn run(app: App) -> i32 {
    // SAFETY: standard Win32 window bootstrap on the UI thread.
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
        let Ok(instance) = GetModuleHandleW(None) else {
            return 1;
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            // Windows sends WM_LBUTTONDBLCLK only to classes with CS_DBLCLKS.
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("WidgetUsageStrip"),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return 1;
        }
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            w!("WidgetUsageStrip"),
            w!("CCT"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return 1;
        };
        STRIP.store(hwnd.0 as isize, Ordering::Relaxed);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")) as isize,
            Ordering::Relaxed,
        );
        let hw_secs = app.cfg.hw_interval_secs;
        let control = app.control.clone();
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        create_tooltip(hwnd);
        with_app(|app| app.sample_hw());
        set_hw_timer(hwnd, hw_secs);
        SetTimer(Some(hwnd), TIMER_TOPMOST, TOPMOST_MS, None);
        SetTimer(Some(hwnd), TIMER_DISKS, DISKS_MS, None);
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        let inbox = spawn_worker(hwnd, control);
        with_app(|app| app.set_inbox(inbox));
        render(hwnd);
        // A left-aligned taskbar leaves no room, so the strip stays hidden: explain why.
        if with_app(|app| app.taskbar_hints().left_aligned) == Some(true) {
            super::settings::open(hwnd);
        }
        // One-time opt-in invitation; the app is not borrowed while the modal box runs.
        if let Some((body, kinds)) = with_app(|app| app.keepalive_prompt()).flatten() {
            let body = wide_z(&body);
            let answer = MessageBoxW(
                None,
                PCWSTR(body.as_ptr()),
                w!("CCT"),
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND,
            );
            with_app(|app| app.answer_keepalive_prompt(&kinds, answer == IDYES));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !super::settings::is_dialog_message(&msg) {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = UnhookWinEvent(hook);
        APP.with(|cell| cell.borrow_mut().take());
        msg.wParam.0 as i32
    }
}
