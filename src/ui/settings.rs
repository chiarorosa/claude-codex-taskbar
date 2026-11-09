//! Settings window in the style of the Windows Settings app: cards, switches and chips, light
//! or dark like Windows apps, changes applied right away.
//!
//! Switches and chips are real `BUTTON` checkboxes (keyboard, Narrator, UI Automation) whose
//! look we paint through `NM_CUSTOMDRAW`; the number fields are real `EDIT`s. The window
//! paints the background, cards, texts and the focus ring itself.

mod layout;
mod model;
mod paint;
mod palette;

use super::app::{claude_provider, codex_provider};
use super::strip::{config_applied, with_app};
use super::view::setup_hints;
use crate::autostart;
use crate::config::Config;
use crate::config::{LABEL_MAX, normalize_label};
use crate::hw::DiskReading;
use crate::i18n::{Key, Lang, fmt, t};
use crate::limits::Provider;
use crate::limits::switcher::store::Store;
use crate::model::ItemId;
use layout::{Block, Ink, Page, Px, Row};
use model::{Toggle, Tone, apply_labels, apply_numbers, apply_toggle, status_line};
use paint::Painter;
use palette::{Palette, palette, read_theme};
use std::cell::RefCell;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DeleteObject, EndPaint, FF_DONTCARE, FW_NORMAL, GetMonitorInfoW, HBRUSH, HDC,
    HFONT, HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SetBkColor, SetTextColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, CDDS_PREPAINT, CDRF_SKIPDEFAULT, EM_SETLIMITTEXT, NM_CUSTOMDRAW,
    NMCUSTOMDRAW, NMHDR, SetScrollInfo,
};
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BN_KILLFOCUS, BN_SETFOCUS, BS_AUTOCHECKBOX, BS_NOTIFY,
    CreateWindowExW, DefWindowProcW, DestroyWindow, EN_KILLFOCUS, EN_SETFOCUS, ES_NUMBER,
    GetClientRect, GetScrollInfo, GetSystemMetrics, GetWindowTextW, HMENU, IDC_ARROW,
    IsDialogMessageW, LoadCursorW, MSG, MoveWindow, RegisterClassExW, SB_BOTTOM, SB_LINEDOWN,
    SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT,
    SCROLLINFO, SIF_PAGE, SIF_POS, SIF_RANGE, SIF_TRACKPOS, SM_CXSCREEN, SM_CXVSCROLL, SM_CYSCREEN,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowPos,
    SetWindowTextW, ShowWindow, UISF_HIDEFOCUS, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_MOUSEWHEEL,
    WM_NOTIFY, WM_PAINT, WM_QUERYUISTATE, WM_SETFONT, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE,
    WM_UPDATEUISTATE, WM_VSCROLL, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_EX_CONTROLPARENT, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{BOOL, PCWSTR, w};

/// What IsDialogMessage sends for Enter and Esc.
const ID_ENTER: usize = 1;
const ID_ESC: usize = 2;
const ID_HW: usize = 10;
const ID_LIMITS: usize = 11;
const ID_CLAUDE_LABEL: usize = 12;
const ID_CODEX_LABEL: usize = 13;
/// Switches and chips get ids from here, in `State::checks` order.
const ID_CHECK: usize = 100;

const STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_VSCROLL.0);
/// Scroll step for a wheel notch or an arrow click, in DIPs.
const SCROLL_LINE: f32 = 40.0;
const EX_STYLE: WINDOW_EX_STYLE = WS_EX_CONTROLPARENT;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Switch,
    Chip,
}

struct Check {
    toggle: Toggle,
    label: String,
    look: Look,
    hwnd: HWND,
}

/// What the page shows that does not come from the controls.
struct Facts {
    lang: Lang,
    hints: Vec<Key>,
    claude_login: bool,
    codex_login: bool,
    /// Under the notification switch: the configured thresholds.
    notify_sub: String,
    /// The saved account holding the live Claude login; its label lives in the account
    /// index instead of the config (#74).
    claude_slot: Option<u32>,
    /// The Claude label as last saved, to write the index only when it changes.
    claude_label: String,
}

struct State {
    hwnd: HWND,
    parent: HWND,
    facts: Facts,
    checks: Vec<Check>,
    /// Text fields as (id, control): the two intervals, then the two labels.
    edits: [(usize, HWND); 4],
    dpi: u32,
    light: bool,
    pal: Palette,
    painter: Painter,
    page: Page,
    edit_font: HFONT,
    field_brush: HBRUSH,
    /// Vertical scroll offset in pixels, when the page is taller than the work area.
    scroll: i32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 0xRRGGBB → GDI's 0x00BBGGRR.
fn colorref(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff))
}

/// Shared access; `None` while the state is being changed (re-entrant messages) or gone.
fn with_state<R>(f: impl FnOnce(&State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow().ok().and_then(|s| s.as_ref().map(f)))
}

fn with_state_mut<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

/// Lets Tab/Enter/Esc work inside the settings window. Call from the message loop.
pub fn is_dialog_message(msg: &MSG) -> bool {
    let hwnd = with_state(|s| s.hwnd);
    // SAFETY: hwnd is our live settings window.
    hwnd.is_some_and(|hwnd| unsafe { IsDialogMessageW(hwnd, msg) }.as_bool())
}

fn is_checked(hwnd: HWND) -> bool {
    // SAFETY: hwnd is one of our checkboxes.
    unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None) }.0 == BST_CHECKED.0 as isize
}

fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 16];
    // SAFETY: buf is a valid output buffer.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// The checkboxes the window has, with their labels and starting states.
fn check_specs(
    cfg: &Config,
    disks: &[DiskReading],
    lang: Lang,
) -> Vec<(Toggle, String, Look, bool)> {
    let weekly = t(lang, Key::SettingsWeekSuffix);
    let keep = t(lang, Key::KeepAliveRow).to_string();
    let mut specs = vec![
        (
            Toggle::Claude,
            "Claude Code".to_string(),
            Look::Switch,
            cfg.claude_enabled,
        ),
        (
            Toggle::KeepAliveClaude,
            keep.clone(),
            Look::Switch,
            cfg.keep_claude_login,
        ),
        (
            Toggle::Codex,
            "Codex".to_string(),
            Look::Switch,
            cfg.codex_enabled,
        ),
        (
            Toggle::KeepAliveCodex,
            keep,
            Look::Switch,
            cfg.keep_codex_login,
        ),
    ];
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
        let on = cfg.is_visible(&id);
        specs.push((Toggle::Item(id), label, Look::Chip, on));
    }
    specs.push((
        Toggle::Autostart,
        t(lang, Key::MenuAutostart).to_string(),
        Look::Switch,
        autostart::is_enabled(),
    ));
    specs.push((
        Toggle::NotifyNearLimit,
        t(lang, Key::NotifyNearLimit).to_string(),
        Look::Switch,
        cfg.notify_near_limit,
    ));
    specs
}

fn is_limit_item(toggle: &Toggle) -> bool {
    matches!(
        toggle,
        Toggle::Item(
            ItemId::ClaudeFive | ItemId::ClaudeWeek | ItemId::CodexFive | ItemId::CodexWeek
        )
    )
}

/// One provider card: the provider switch with its login line, then its keep-alive switch.
struct ProviderCard {
    toggle: Toggle,
    keep: Toggle,
    name: &'static str,
    tile: (char, u32),
    login: bool,
    missing: Key,
    keep_sub: Key,
    label_id: usize,
    label_sub: Key,
}

/// The page model from the facts and the current check states.
fn blocks(facts: &Facts, checks: &[(Toggle, String, bool)]) -> Vec<Block> {
    let lang = facts.lang;
    let id_of =
        |toggle: &Toggle| ID_CHECK + checks.iter().position(|(t, _, _)| t == toggle).unwrap_or(0);
    let on_of = |toggle: &Toggle| checks.iter().any(|(t, _, on)| t == toggle && *on);
    let state = |toggle: &Toggle| {
        let key = if on_of(toggle) {
            Key::SwitchOn
        } else {
            Key::SwitchOff
        };
        t(lang, key).to_string()
    };
    let provider = |p: ProviderCard| {
        let (tone, key) = status_line(p.login, p.missing);
        let status = match tone {
            Tone::Success => (format!("✓ {}", t(lang, key)), Ink::Success),
            Tone::Warning => (format!("⚠ {}", t(lang, key)), Ink::Warning),
        };
        Block::Card(vec![
            Row::Switch {
                id: id_of(&p.toggle),
                title: p.name.to_string(),
                sub: Some(status),
                tile: Some(p.tile),
                indent: false,
                state_text: state(&p.toggle),
            },
            Row::Switch {
                id: id_of(&p.keep),
                title: t(lang, Key::KeepAliveRow).to_string(),
                sub: Some((t(lang, p.keep_sub).to_string(), Ink::Sub)),
                tile: None,
                indent: true,
                state_text: state(&p.keep),
            },
            Row::Number {
                id: p.label_id,
                title: t(lang, Key::SettingsLabel).to_string(),
                sub: t(lang, p.label_sub).to_string(),
            },
        ])
    };
    let chips = |limits: bool| {
        checks
            .iter()
            .filter(|(t, _, _)| matches!(t, Toggle::Item(_)) && is_limit_item(t) == limits)
            .map(|(t, label, _)| (id_of(t), label.clone()))
            .collect::<Vec<_>>()
    };

    let mut out: Vec<Block> = facts
        .hints
        .iter()
        .map(|key| Block::Hint(format!("⚠ {}", t(lang, *key))))
        .collect();
    out.push(Block::Caption(t(lang, Key::SettingsProviders).to_string()));
    out.push(provider(ProviderCard {
        toggle: Toggle::Claude,
        keep: Toggle::KeepAliveClaude,
        name: "Claude Code",
        tile: ('C', crate::model::COLOR_CLAUDE),
        login: facts.claude_login,
        missing: Key::ClaudeNoLogin,
        keep_sub: Key::KeepAliveSubClaude,
        label_id: ID_CLAUDE_LABEL,
        label_sub: if facts.claude_slot.is_some() {
            Key::SettingsLabelSubAccount
        } else {
            Key::SettingsLabelSub
        },
    }));
    out.push(provider(ProviderCard {
        toggle: Toggle::Codex,
        keep: Toggle::KeepAliveCodex,
        name: "Codex",
        tile: ('X', crate::model::COLOR_CODEX),
        login: facts.codex_login,
        missing: Key::CodexNoLogin,
        keep_sub: Key::KeepAliveSubCodex,
        label_id: ID_CODEX_LABEL,
        label_sub: Key::SettingsLabelSub,
    }));
    out.push(Block::Caption(t(lang, Key::SettingsItems).to_string()));
    out.push(Block::Card(vec![
        Row::Chips {
            label: t(lang, Key::SettingsHardware).to_string(),
            chips: chips(false),
        },
        Row::Chips {
            label: t(lang, Key::SettingsLimits).to_string(),
            chips: chips(true),
        },
    ]));
    out.push(Block::Caption(t(lang, Key::SettingsGeneral).to_string()));
    out.push(Block::Card(vec![
        Row::Switch {
            id: id_of(&Toggle::Autostart),
            title: t(lang, Key::MenuAutostart).to_string(),
            sub: None,
            tile: None,
            indent: false,
            state_text: state(&Toggle::Autostart),
        },
        Row::Switch {
            id: id_of(&Toggle::NotifyNearLimit),
            title: t(lang, Key::NotifyNearLimit).to_string(),
            sub: Some((facts.notify_sub.clone(), Ink::Sub)),
            tile: None,
            indent: false,
            state_text: state(&Toggle::NotifyNearLimit),
        },
        Row::Number {
            id: ID_HW,
            title: t(lang, Key::SettingsHwInterval).to_string(),
            sub: t(lang, Key::HwIntervalRange).to_string(),
        },
        Row::Number {
            id: ID_LIMITS,
            title: t(lang, Key::SettingsLimitsInterval).to_string(),
            sub: t(lang, Key::LimitsIntervalRange).to_string(),
        },
    ]));
    out
}

fn edit_font(dpi: u32) -> HFONT {
    let height = -((13.0 * dpi as f32 / 96.0).round() as i32);
    // SAFETY: plain font creation; freed in `WM_DESTROY` or when the DPI changes.
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            FF_DONTCARE.0 as u32,
            w!("Segoe UI"),
        )
    }
}

fn set_dark_title(hwnd: HWND, light: bool) {
    let dark = BOOL::from(!light);
    // SAFETY: valid window and a BOOL-sized attribute; older builds reject it harmlessly.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const BOOL).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}

/// Window size for a client area, at `dpi`.
fn window_size(client: (i32, i32), dpi: u32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client.0,
        bottom: client.1,
    };
    // SAFETY: rect is a valid in/out rectangle.
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rect, STYLE, false, EX_STYLE, dpi);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn page_size() -> (i32, i32) {
    with_state(|s| (s.page.width, s.page.height)).unwrap_or((1, 1))
}

/// Rebuilds the page from the controls' current states and moves every control.
fn relayout() {
    let Some(specs) = with_state(|s| {
        s.checks
            .iter()
            .map(|c| (c.toggle.clone(), c.label.clone(), is_checked(c.hwnd)))
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    with_state_mut(|s| {
        let model = blocks(&s.facts, &specs);
        let title = t(s.facts.lang, Key::SettingsPageTitle);
        s.page = layout::layout(title, &model, s.dpi as f32 / 96.0, &s.painter);
    });
    place_children();
}

/// Moves every control to its rect on the page, minus the scroll offset.
fn place_children() {
    let placed = with_state(|s| {
        let scale = s.dpi as f32 / 96.0;
        let rect_of = |id: usize| {
            s.page
                .controls
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, r)| Px {
                    y: r.y - s.scroll,
                    ..*r
                })
        };
        let mut moves: Vec<(HWND, Px)> = s
            .checks
            .iter()
            .enumerate()
            .filter_map(|(i, c)| rect_of(ID_CHECK + i).map(|r| (c.hwnd, r)))
            .collect();
        let inset = (8.0 * scale).round() as i32;
        let line = (18.0 * scale).round() as i32;
        for (id, edit) in s.edits {
            if let Some(r) = rect_of(id) {
                let inner = Px {
                    x: r.x + inset,
                    y: r.y + (r.h - line) / 2,
                    w: r.w - 2 * inset,
                    h: line,
                };
                moves.push((edit, inner));
            }
        }
        (moves, s.hwnd)
    });
    let Some((moves, hwnd)) = placed else {
        return;
    };
    // SAFETY: moving our own child controls; no state borrow is held here.
    unsafe {
        for (child, r) in moves {
            let _ = MoveWindow(child, r.x, r.y, r.w, r.h, true);
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn client_height(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    // SAFETY: rect is a valid out-pointer for our own window.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect.bottom - rect.top
}

/// Syncs the scroll bar with the page and the client height; the bar hides when all fits.
fn update_scroll() {
    let Some((hwnd, content, offset)) = with_state(|s| (s.hwnd, s.page.height, s.scroll)) else {
        return;
    };
    let view = client_height(hwnd);
    let offset = layout::clamp_scroll(offset, content, view);
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: content - 1,
        nPage: view.max(0) as u32,
        nPos: offset,
        nTrackPos: 0,
    };
    // SAFETY: valid SCROLLINFO for our own window.
    unsafe {
        SetScrollInfo(hwnd, SB_VERT, &info, true);
    }
    scroll_to(offset);
}

/// Scrolls the page to `offset` (clamped) and moves the controls with it.
fn scroll_to(offset: i32) {
    let changed = with_state_mut(|s| {
        let offset = layout::clamp_scroll(offset, s.page.height, client_height(s.hwnd));
        let changed = offset != s.scroll;
        s.scroll = offset;
        (changed, s.hwnd, offset)
    });
    if let Some((true, hwnd, offset)) = changed {
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_POS,
            nPos: offset,
            ..Default::default()
        };
        // SAFETY: valid SCROLLINFO for our own window.
        unsafe {
            SetScrollInfo(hwnd, SB_VERT, &info, true);
        }
        place_children();
    }
}

fn scroll_by(delta: i32) {
    if let Some(offset) = with_state(|s| s.scroll) {
        scroll_to(offset + delta);
    }
}

/// Scrolls just enough to show the control with `id` (keyboard focus moved to it).
fn reveal_control(id: usize) {
    let target = with_state(|s| {
        let r = s.page.controls.iter().find(|(i, _)| *i == id)?.1;
        let margin = (8.0 * s.dpi as f32 / 96.0).round() as i32;
        Some(layout::reveal(s.scroll, r, client_height(s.hwnd), margin))
    })
    .flatten();
    if let Some(offset) = target {
        scroll_to(offset);
    }
}

fn line_px() -> i32 {
    with_state(|s| (SCROLL_LINE * s.dpi as f32 / 96.0).round() as i32).unwrap_or(40)
}

/// Work area of the monitor the window is on.
fn work_area(hwnd: HWND) -> Option<Px> {
    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: mi is sized for GetMonitorInfoW; the monitor handle comes from our window.
    let ok = unsafe {
        GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool()
    };
    let r = mi.rcWork;
    ok.then_some(Px {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    })
}

/// Sizes and places the window inside the work area, with room for the scroll bar when the
/// page does not fit, then syncs the bar. `anchor` keeps the left/top Windows suggested.
fn place_window(hwnd: HWND, dpi: u32, fallback: Px, anchor: Option<(i32, i32)>) {
    let work = work_area(hwnd).unwrap_or(fallback);
    let mut size = window_size(page_size(), dpi);
    if size.1 > work.h {
        // SAFETY: plain metric query.
        size.0 += unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) };
    }
    let mut r = layout::fit(work, size);
    if let Some((x, y)) = anchor {
        r.x = x;
        r.y = y.min(work.y + work.h - r.h).max(work.y);
    }
    // SAFETY: resizing our own window.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            r.x,
            r.y,
            r.w,
            r.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    update_scroll();
}

fn redraw_children() {
    let children: Vec<HWND> = with_state(|s| {
        s.checks
            .iter()
            .map(|c| c.hwnd)
            .chain(s.edits.map(|(_, e)| e))
            .collect()
    })
    .unwrap_or_default();
    // SAFETY: invalidating our own child controls.
    unsafe {
        for c in children {
            let _ = InvalidateRect(Some(c), None, true);
        }
    }
}

/// Re-reads the Windows theme and repaints.
fn retheme() {
    let theme = read_theme();
    let pal = palette(&theme);
    let Some(hwnd) = with_state_mut(|s| {
        s.light = theme.light;
        s.pal = pal;
        // SAFETY: replacing our own brush; the edits ask for it again on their next paint.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(s.field_brush.0));
            s.field_brush = CreateSolidBrush(colorref(pal.field));
        }
        s.hwnd
    }) else {
        return;
    };
    set_dark_title(hwnd, theme.light);
    relayout();
    redraw_children();
}

/// Applies a config built from the current app config, then refreshes the strip.
fn commit(parent: HWND, build: impl FnOnce(&Config) -> Config) {
    let Some(base) = with_app(|app| app.cfg.clone()) else {
        return;
    };
    config_applied(parent, build(&base));
}

/// Applies the switch or chip at `index` (only that field changes).
fn apply_check(index: usize) {
    let Some((parent, toggle, on)) = with_state(|s| {
        let c = s.checks.get(index)?;
        Some((s.parent, c.toggle.clone(), is_checked(c.hwnd)))
    })
    .flatten() else {
        return;
    };
    let mut autostart = None;
    commit(parent, |base| {
        let applied = apply_toggle(base, &toggle, on);
        autostart = applied.autostart;
        applied.cfg
    });
    if let Some(on) = autostart
        && on != autostart::is_enabled()
    {
        let _ = if on {
            autostart::enable()
        } else {
            autostart::disable()
        };
    }
}

/// Applies the interval and label fields and shows the values the config actually took.
fn apply_fields() {
    let Some((parent, edits, slot, saved_label)) = with_state(|s| {
        (
            s.parent,
            s.edits,
            s.facts.claude_slot,
            s.facts.claude_label.clone(),
        )
    }) else {
        return;
    };
    let texts = edits.map(|(_, e)| window_text(e));
    let Some(base) = with_app(|app| app.cfg.clone()) else {
        return;
    };
    let numbers = apply_numbers(&base, &texts[0], &texts[1]);
    let cfg = apply_labels(&numbers, &texts[2], &texts[3], slot.is_some());
    let claude = normalize_label(&texts[2]);
    // The live saved account keeps its own label in the account index (#74).
    let slot_changed = match slot {
        Some(id) if claude != saved_label => {
            let saved = Store::new(Store::default_dir())
                .set_label(id, &claude)
                .is_ok();
            with_state_mut(|s| s.facts.claude_label = claude.clone());
            saved
        }
        _ => false,
    };
    let shown = [
        cfg.hw_interval_secs.to_string(),
        cfg.limits_interval_secs.to_string(),
        if slot.is_some() {
            claude
        } else {
            cfg.claude_label.clone()
        },
        cfg.codex_label.clone(),
    ];
    for (((_, edit), text), want) in edits.iter().zip(&texts).zip(&shown) {
        if text != want {
            let wide = wide_z(want);
            // SAFETY: our own edit control; the EN_CHANGE it sends is ignored.
            unsafe {
                let _ = SetWindowTextW(*edit, PCWSTR(wide.as_ptr()));
            }
        }
    }
    if cfg != base || slot_changed {
        commit(parent, |_| cfg);
    }
}

/// Closes the settings window, if open, the way Esc does: pending field edits are applied
/// first. Used before a Claude account switch.
pub fn close_if_open() {
    if let Some(hwnd) = with_state(|s| s.hwnd) {
        close(hwnd);
    }
}

/// Opens the settings window, or focuses it if it is already open.
pub fn open(parent: HWND) {
    if let Some(hwnd) = with_state(|s| s.hwnd) {
        // SAFETY: hwnd is our live settings window.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    let Some((cfg, disks, lang, taskbar, monitor)) = with_app(|app| {
        (
            app.cfg.clone(),
            app.hw.disks.clone(),
            app.lang,
            app.taskbar_hints(),
            app.strip_monitor(),
        )
    }) else {
        return;
    };
    let facts = Facts {
        lang,
        hints: setup_hints(taskbar),
        claude_login: claude_provider().has_login(),
        codex_login: codex_provider().has_login(),
        notify_sub: fmt(
            t(lang, Key::NotifyNearLimitSub),
            &cfg.notify_thresholds
                .iter()
                .map(|p| format!("{p}%"))
                .collect::<Vec<_>>()
                .join(" · "),
        ),
        claude_slot: super::switcher_ui::live_slot(),
        claude_label: super::switcher_ui::live_claude_label(&cfg.claude_label),
    };
    let monitor = monitor.map(|m| Px {
        x: m.left,
        y: m.top,
        w: m.right - m.left,
        h: m.bottom - m.top,
    });
    create(parent, &cfg, &disks, facts, monitor);
}

fn create(parent: HWND, cfg: &Config, disks: &[DiskReading], facts: Facts, monitor: Option<Px>) {
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
            lpszClassName: w!("WidgetUsageSettings"),
            ..Default::default()
        };
        // Fails harmlessly when already registered by an earlier open.
        RegisterClassExW(&class);
        let primary = Px {
            x: 0,
            y: 0,
            w: GetSystemMetrics(SM_CXSCREEN),
            h: GetSystemMetrics(SM_CYSCREEN),
        };
        // Start on the target monitor so the window gets that monitor's DPI.
        let (x0, y0) = layout::center_on(monitor, primary, (1, 1));
        let title = wide_z(t(facts.lang, Key::SettingsTitle));
        let Ok(hwnd) = CreateWindowExW(
            EX_STYLE,
            w!("WidgetUsageSettings"),
            PCWSTR(title.as_ptr()),
            STYLE | WS_CLIPCHILDREN,
            x0,
            y0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        let theme = read_theme();
        let pal = palette(&theme);
        set_dark_title(hwnd, theme.light);
        let Ok(painter) = Painter::new(dpi) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let font = edit_font(dpi);

        let mut checks = Vec::new();
        for (i, (toggle, label, look, on)) in
            check_specs(cfg, disks, facts.lang).into_iter().enumerate()
        {
            let text = wide_z(&label);
            let style = WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WINDOW_STYLE((BS_AUTOCHECKBOX | BS_NOTIFY) as u32);
            let child = CreateWindowExW(
                Default::default(),
                w!("BUTTON"),
                PCWSTR(text.as_ptr()),
                style,
                0,
                0,
                1,
                1,
                Some(hwnd),
                Some(HMENU((ID_CHECK + i) as _)),
                None,
                None,
            )
            .unwrap_or_default();
            let state = if on { BST_CHECKED } else { BST_UNCHECKED };
            SendMessageW(child, BM_SETCHECK, Some(WPARAM(state.0 as usize)), None);
            checks.push(Check {
                toggle,
                label,
                look,
                hwnd: child,
            });
        }
        let edit = |id: usize, text: &str, number: bool| {
            let text = wide_z(text);
            let style = if number { ES_NUMBER as u32 } else { 0 };
            let child = CreateWindowExW(
                Default::default(),
                w!("EDIT"),
                PCWSTR(text.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(style),
                0,
                0,
                1,
                1,
                Some(hwnd),
                Some(HMENU(id as _)),
                None,
                None,
            )
            .unwrap_or_default();
            SendMessageW(
                child,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            if !number {
                SendMessageW(child, EM_SETLIMITTEXT, Some(WPARAM(LABEL_MAX)), None);
            }
            child
        };
        let edits = [
            (ID_HW, edit(ID_HW, &cfg.hw_interval_secs.to_string(), true)),
            (
                ID_LIMITS,
                edit(ID_LIMITS, &cfg.limits_interval_secs.to_string(), true),
            ),
            (
                ID_CLAUDE_LABEL,
                edit(ID_CLAUDE_LABEL, &facts.claude_label, false),
            ),
            (
                ID_CODEX_LABEL,
                edit(ID_CODEX_LABEL, &cfg.codex_label, false),
            ),
        ];

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                hwnd,
                parent,
                facts,
                checks,
                edits,
                dpi,
                light: theme.light,
                pal,
                painter,
                page: Page::default(),
                edit_font: font,
                field_brush: CreateSolidBrush(colorref(pal.field)),
                scroll: 0,
            })
        });
        relayout();

        place_window(hwnd, dpi, monitor.unwrap_or(primary), None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Paints one switch or chip in place of the native checkbox.
fn custom_draw(cd: &NMCUSTOMDRAW) -> Option<LRESULT> {
    if cd.dwDrawStage != CDDS_PREPAINT {
        return None;
    }
    with_state(|s| {
        let check = s.checks.iter().find(|c| c.hwnd == cd.hdr.hwndFrom)?;
        let r = Px {
            x: cd.rc.left,
            y: cd.rc.top,
            w: cd.rc.right - cd.rc.left,
            h: cd.rc.bottom - cd.rc.top,
        };
        let on = is_checked(check.hwnd);
        let _ = match check.look {
            Look::Switch => s.painter.switch(cd.hdc, r, on, &s.pal),
            Look::Chip => s.painter.chip(cd.hdc, r, &check.label, on, &s.pal),
        };
        Some(LRESULT(CDRF_SKIPDEFAULT as isize))
    })
    .flatten()
}

/// The focused switch or chip, when keyboard cues are showing. Fields show focus with their
/// accent underline instead.
fn focus_ring(s: &State, page: &Page) -> Option<Px> {
    // SAFETY: plain queries on our own window.
    let (focus, ui) = unsafe {
        (
            GetFocus(),
            SendMessageW(s.hwnd, WM_QUERYUISTATE, None, None),
        )
    };
    if ui.0 as u32 & UISF_HIDEFOCUS != 0 {
        return None;
    }
    let i = s.checks.iter().position(|c| c.hwnd == focus)?;
    page.controls
        .iter()
        .find(|(id, _)| *id == ID_CHECK + i)
        .map(|(_, r)| *r)
}

fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: standard paint cycle on our own window.
    unsafe {
        let hdc: HDC = BeginPaint(hwnd, &mut ps);
        let focus = GetFocus();
        with_state(|s| {
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let client = Px {
                x: 0,
                y: 0,
                w: rect.right,
                h: rect.bottom,
            };
            let page = s.page.shifted(-s.scroll);
            let _ = s
                .painter
                .page(hdc, client, &page, &s.pal, focus_ring(s, &page));
            for (id, edit) in s.edits {
                if let Some((_, r)) = page.controls.iter().find(|(i, _)| *i == id) {
                    let _ = s.painter.field(hdc, *r, focus == edit, &s.pal);
                }
            }
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

fn invalidate(hwnd: HWND) {
    // SAFETY: invalidating our own window.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn close(hwnd: HWND) {
    apply_fields();
    // SAFETY: closing our own window.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn dpi_changed(hwnd: HWND, dpi: u32, suggested: RECT) {
    let fonts = with_state_mut(|s| {
        s.dpi = dpi;
        let _ = s.painter.set_dpi(dpi);
        let old = s.edit_font;
        s.edit_font = edit_font(dpi);
        (old, s.edit_font, s.edits.map(|(_, e)| e))
    });
    if let Some((old, font, edits)) = fonts {
        // SAFETY: our own edits take the new font before the old one is freed.
        unsafe {
            for e in edits {
                SendMessageW(
                    e,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
            let _ = DeleteObject(HGDIOBJ(old.0));
        }
    }
    relayout();
    let fallback = Px {
        x: suggested.left,
        y: suggested.top,
        w: suggested.right - suggested.left,
        h: suggested.bottom - suggested.top,
    };
    place_window(hwnd, dpi, fallback, Some((suggested.left, suggested.top)));
    redraw_children();
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let (id, code) = (wparam.0 & 0xffff, ((wparam.0 >> 16) & 0xffff) as u32);
            match (id, code) {
                (ID_ESC, _) => close(hwnd),
                (ID_ENTER, _) => apply_fields(),
                (ID_HW | ID_LIMITS | ID_CLAUDE_LABEL | ID_CODEX_LABEL, EN_KILLFOCUS) => {
                    apply_fields();
                    invalidate(hwnd);
                }
                (ID_HW | ID_LIMITS | ID_CLAUDE_LABEL | ID_CODEX_LABEL, EN_SETFOCUS) => {
                    reveal_control(id);
                    invalidate(hwnd);
                }
                (i, BN_CLICKED) if i >= ID_CHECK => {
                    apply_check(i - ID_CHECK);
                    relayout();
                }
                (i, BN_SETFOCUS) if i >= ID_CHECK => {
                    reveal_control(i);
                    invalidate(hwnd);
                }
                (i, BN_KILLFOCUS) if i >= ID_CHECK => invalidate(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            // SAFETY: WM_NOTIFY's lParam points at an NMHDR.
            let hdr = unsafe { &*(lparam.0 as *const NMHDR) };
            if hdr.code == NM_CUSTOMDRAW {
                // SAFETY: NM_CUSTOMDRAW from a button carries an NMCUSTOMDRAW.
                let cd = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
                if let Some(r) = custom_draw(cd) {
                    return r;
                }
            }
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as _);
            with_state(|s| {
                // SAFETY: hdc is the edit's DC for this paint.
                unsafe {
                    SetTextColor(hdc, colorref(s.pal.text));
                    SetBkColor(hdc, colorref(s.pal.field));
                }
                LRESULT(s.field_brush.0 as isize)
            })
            // SAFETY: default handling when the state is busy.
            .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
        }
        WM_VSCROLL => {
            let view = client_height(hwnd);
            let line = line_px();
            match (wparam.0 & 0xffff) as i32 {
                c if c == SB_LINEUP.0 => scroll_by(-line),
                c if c == SB_LINEDOWN.0 => scroll_by(line),
                c if c == SB_PAGEUP.0 => scroll_by(-view),
                c if c == SB_PAGEDOWN.0 => scroll_by(view),
                c if c == SB_TOP.0 => scroll_to(0),
                c if c == SB_BOTTOM.0 => scroll_to(i32::MAX),
                c if c == SB_THUMBTRACK.0 || c == SB_THUMBPOSITION.0 => {
                    let mut info = SCROLLINFO {
                        cbSize: size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_TRACKPOS,
                        ..Default::default()
                    };
                    // SAFETY: valid SCROLLINFO for our own window.
                    if unsafe { GetScrollInfo(hwnd, SB_VERT, &mut info) }.is_ok() {
                        scroll_to(info.nTrackPos);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_SIZE => {
            update_scroll();
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
            scroll_by(-delta * line_px() / 120);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_UPDATEUISTATE => {
            invalidate(hwnd);
            // SAFETY: default handling keeps the UI state bookkeeping.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
            retheme();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: WM_DPICHANGED's lParam points at the suggested window rect.
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            dpi_changed(hwnd, (wparam.0 & 0xffff) as u32, suggested);
            LRESULT(0)
        }
        WM_CLOSE => {
            close(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state) = STATE.with(|s| s.borrow_mut().take()) {
                // SAFETY: the window is gone; its font and brush are no longer in use.
                unsafe {
                    let _ = DeleteObject(HGDIOBJ(state.edit_font.0));
                    let _ = DeleteObject(HGDIOBJ(state.field_brush.0));
                }
            }
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
