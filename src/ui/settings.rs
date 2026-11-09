//! Settings window: provider and item switches, refresh intervals, start with Windows.

use super::app::{claude_provider, codex_provider};
use super::strip::{config_applied, with_app};
use super::view::setup_hints;
use crate::autostart;
use crate::config::Config;
use crate::hw::DiskReading;
use crate::i18n::{Key, Lang, fmt, t};
use crate::limits::Provider;
use crate::model::ItemId;
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, CreateFontIndirectW, DeleteObject, HBRUSH, HFONT, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateWindowExW,
    DefWindowProcW, DestroyWindow, ES_NUMBER, GetSystemMetrics, GetWindowTextW, HMENU, IDC_ARROW,
    IsDialogMessageW, LoadCursorW, MSG, NONCLIENTMETRICSW, RegisterClassExW, SM_CXSCREEN,
    SM_CYSCREEN, SPI_GETNONCLIENTMETRICS, SW_SHOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SendMessageW, SetForegroundWindow, ShowWindow, SystemParametersInfoW, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;

// Layout in DIPs.
const PAD: i32 = 14;
const ROW: i32 = 24;
const WIDTH: i32 = 380;

#[derive(Clone, Debug, PartialEq)]
enum Toggle {
    Claude,
    Codex,
    Item(ItemId),
    Autostart,
    KeepAliveClaude,
    KeepAliveCodex,
}

struct State {
    hwnd: HWND,
    parent: HWND,
    base: Config,
    checks: Vec<(Toggle, HWND)>,
    hw_edit: HWND,
    limits_edit: HWND,
    font: HFONT,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Lets Tab/Enter work inside the settings window. Call from the message loop.
pub fn is_dialog_message(msg: &MSG) -> bool {
    let hwnd = STATE.with(|s| s.borrow().as_ref().map(|s| s.hwnd));
    // SAFETY: hwnd is our live settings window.
    hwnd.is_some_and(|hwnd| unsafe { IsDialogMessageW(hwnd, msg) }.as_bool())
}

fn message_font() -> HFONT {
    let mut ncm = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    // SAFETY: ncm is sized correctly for SPI_GETNONCLIENTMETRICS.
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            ncm.cbSize,
            Some((&mut ncm as *mut NONCLIENTMETRICSW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        CreateFontIndirectW(&ncm.lfMessageFont)
    }
}

struct Builder {
    parent: HWND,
    font: HFONT,
    scale: f32,
    y: i32,
}

impl Builder {
    fn px(&self, dip: i32) -> i32 {
        (dip as f32 * self.scale).round() as i32
    }

    fn control(
        &self,
        class: PCWSTR,
        text: &str,
        style: WINDOW_STYLE,
        x: i32,
        w: i32,
        id: usize,
    ) -> HWND {
        let text = wide_z(text);
        // SAFETY: child control of our live window; font outlives the window.
        unsafe {
            let hwnd = CreateWindowExW(
                if class == w!("EDIT") {
                    WS_EX_CLIENTEDGE
                } else {
                    Default::default()
                },
                class,
                PCWSTR(text.as_ptr()),
                WS_CHILD | WS_VISIBLE | style,
                self.px(x),
                self.px(self.y),
                self.px(w),
                self.px(ROW - 2),
                Some(self.parent),
                Some(HMENU(id as _)),
                None,
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(1)),
            );
            hwnd
        }
    }

    fn header(&mut self, text: &str) {
        self.y += 4;
        self.control(w!("STATIC"), text, WINDOW_STYLE(0), PAD, WIDTH - 2 * PAD, 0);
        self.y += ROW;
    }

    /// Wrapped two-line notice (setup hints).
    fn note(&mut self, text: &str) {
        let text = wide_z(&format!("⚠ {text}"));
        // SAFETY: child control of our live window; font outlives the window.
        unsafe {
            let hwnd = CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                PCWSTR(text.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                self.px(PAD),
                self.px(self.y),
                self.px(WIDTH - 2 * PAD),
                self.px(2 * ROW + 4),
                Some(self.parent),
                None,
                None,
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        self.y += 2 * ROW + 10;
    }

    fn check(&mut self, text: &str, checked: bool) -> HWND {
        let style = WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP;
        let hwnd = self.control(w!("BUTTON"), text, style, PAD + 8, WIDTH - 2 * PAD - 8, 0);
        let state = if checked { BST_CHECKED } else { BST_UNCHECKED };
        // SAFETY: hwnd is a checkbox we just created.
        unsafe { SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(state.0 as usize)), None) };
        self.y += ROW;
        hwnd
    }

    fn number(&mut self, label: &str, value: u32) -> HWND {
        self.control(w!("STATIC"), label, WINDOW_STYLE(0), PAD + 8, 250, 0);
        let style = WINDOW_STYLE(ES_NUMBER as u32) | WS_TABSTOP | WS_BORDER;
        let edit = self.control(w!("EDIT"), &value.to_string(), style, PAD + 270, 60, 0);
        self.y += ROW + 4;
        edit
    }
}

fn login_text(lang: Lang, found: bool, missing: Key) -> String {
    if found {
        t(lang, Key::LoginFound).into()
    } else {
        t(lang, missing).into()
    }
}

/// Opens the settings window, or focuses it if it is already open.
pub fn open(parent: HWND) {
    if let Some(hwnd) = STATE.with(|s| s.borrow().as_ref().map(|s| s.hwnd)) {
        // SAFETY: hwnd is our live settings window.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    let Some((cfg, disks, lang, taskbar)) = with_app(|app| {
        (
            app.cfg.clone(),
            app.hw.disks.clone(),
            app.lang,
            app.taskbar_hints(),
        )
    }) else {
        return;
    };
    let hints = setup_hints(taskbar);
    create(parent, cfg, &disks, lang, &hints);
}

fn create(parent: HWND, cfg: Config, disks: &[DiskReading], lang: Lang, hints: &[Key]) {
    // SAFETY: standard window creation on the UI thread.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as _),
            lpszClassName: w!("WidgetUsageSettings"),
            ..Default::default()
        };
        // Fails harmlessly when already registered by an earlier open.
        RegisterClassExW(&class);
        let scale = GetDpiForSystem() as f32 / 96.0;
        let title = wide_z(t(lang, Key::SettingsTitle));
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            w!("WidgetUsageSettings"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        let font = message_font();
        let mut b = Builder {
            parent: hwnd,
            font,
            scale,
            y: PAD,
        };
        let mut checks = Vec::new();

        for key in hints {
            b.note(t(lang, *key));
        }
        b.header(t(lang, Key::SettingsProviders));
        let claude_login = login_text(lang, claude_provider().has_login(), Key::ClaudeNoLogin);
        let codex_login = login_text(lang, codex_provider().has_login(), Key::CodexNoLogin);
        checks.push((
            Toggle::Claude,
            b.check(&format!("Claude Code — {claude_login}"), cfg.claude_enabled),
        ));
        checks.push((
            Toggle::Codex,
            b.check(&format!("Codex — {codex_login}"), cfg.codex_enabled),
        ));

        let keep = t(lang, Key::KeepAliveSetting);
        checks.push((
            Toggle::KeepAliveClaude,
            b.check(&fmt(keep, "Claude Code"), cfg.keep_claude_login),
        ));
        checks.push((
            Toggle::KeepAliveCodex,
            b.check(&fmt(keep, "Codex"), cfg.keep_codex_login),
        ));

        b.header(t(lang, Key::SettingsItems));
        let weekly = t(lang, Key::SettingsWeekSuffix);
        let mut items: Vec<(ItemId, String)> = vec![
            (ItemId::Cpu, "CPU".into()),
            (ItemId::Ram, "RAM".into()),
            (ItemId::Gpu, "GPU".into()),
        ];
        items.extend(
            disks
                .iter()
                .map(|d| (ItemId::Disk(d.instance.clone()), d.label.clone())),
        );
        items.extend([
            (ItemId::ClaudeFive, "Claude 5h".into()),
            (ItemId::ClaudeWeek, format!("Claude {weekly}")),
            (ItemId::CodexFive, "Codex 5h".into()),
            (ItemId::CodexWeek, format!("Codex {weekly}")),
        ]);
        for (id, label) in items {
            let visible = cfg.is_visible(&id);
            checks.push((Toggle::Item(id), b.check(&label, visible)));
        }

        b.y += 6;
        let hw_edit = b.number(t(lang, Key::SettingsHwInterval), cfg.hw_interval_secs);
        let limits_edit = b.number(
            t(lang, Key::SettingsLimitsInterval),
            cfg.limits_interval_secs,
        );
        checks.push((
            Toggle::Autostart,
            b.check(t(lang, Key::MenuAutostart), autostart::is_enabled()),
        ));

        b.y += 8;
        let ok_style = WINDOW_STYLE(BS_DEFPUSHBUTTON as u32) | WS_TABSTOP;
        let cancel_style = WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP;
        b.control(
            w!("BUTTON"),
            t(lang, Key::Ok),
            ok_style,
            WIDTH - PAD - 180,
            85,
            ID_OK,
        );
        b.control(
            w!("BUTTON"),
            t(lang, Key::Cancel),
            cancel_style,
            WIDTH - PAD - 90,
            85,
            ID_CANCEL,
        );
        b.y += ROW + PAD;

        // Client size → window size, centered on the primary screen.
        let mut rect = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: b.px(WIDTH),
            bottom: b.px(b.y),
        };
        let _ = windows::Win32::UI::WindowsAndMessaging::AdjustWindowRectEx(
            &mut rect,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            false,
            WS_EX_DLGMODALFRAME,
        );
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;
        let _ = windows::Win32::UI::WindowsAndMessaging::MoveWindow(hwnd, x, y, w, h, false);

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                hwnd,
                parent,
                base: cfg,
                checks,
                hw_edit,
                limits_edit,
                font,
            })
        });
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn is_checked(hwnd: HWND) -> bool {
    // SAFETY: hwnd is one of our checkboxes.
    unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None) }.0 == BST_CHECKED.0 as isize
}

fn read_number(hwnd: HWND, fallback: u32) -> u32 {
    let mut buf = [0u16; 16];
    // SAFETY: buf is a valid output buffer.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
        .trim()
        .parse()
        .unwrap_or(fallback)
}

/// Builds the new config from the controls and applies it.
fn apply() {
    let Some((parent, cfg, autostart_on)) = STATE.with(|s| {
        let s = s.borrow();
        let s = s.as_ref()?;
        let mut cfg = s.base.clone();
        let mut autostart_on = None;
        for (toggle, hwnd) in &s.checks {
            let on = is_checked(*hwnd);
            match toggle {
                Toggle::Claude => cfg.claude_enabled = on,
                Toggle::Codex => cfg.codex_enabled = on,
                Toggle::Item(id) => cfg.set_visible(id, on),
                Toggle::Autostart => autostart_on = Some(on),
                Toggle::KeepAliveClaude => {
                    cfg.keep_claude_login = on;
                    cfg.keepalive_asked = true;
                }
                Toggle::KeepAliveCodex => {
                    cfg.keep_codex_login = on;
                    cfg.codex_keepalive_asked = true;
                }
            }
        }
        cfg.hw_interval_secs = read_number(s.hw_edit, cfg.hw_interval_secs);
        cfg.limits_interval_secs = read_number(s.limits_edit, cfg.limits_interval_secs);
        Some((s.parent, cfg.clamped(), autostart_on))
    }) else {
        return;
    };
    if let Some(on) = autostart_on
        && on != autostart::is_enabled()
    {
        let _ = if on {
            autostart::enable()
        } else {
            autostart::disable()
        };
    }
    config_applied(parent, cfg);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                ID_OK => {
                    apply();
                    // SAFETY: closing our own window.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                ID_CANCEL => {
                    // SAFETY: closing our own window.
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: closing our own window.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state) = STATE.with(|s| s.borrow_mut().take()) {
                // SAFETY: the font was created for this window, which is now gone.
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(state.font.0));
                }
            }
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
