//! Taskbar geometry and environment: where the free left corner is and when to hide.

use crate::config::Placement;
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
    UIA_AutomationIdPropertyId,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
use windows::core::{BSTR, HSTRING, w};

mod edges;

pub use edges::invalidate_edges;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaskbarInfo {
    pub rect: Rect,
    pub monitor: Rect,
    pub dpi: u32,
    pub autohide: bool,
    /// False while an auto-hidden taskbar is slid off screen.
    pub visible: bool,
    pub left_aligned: bool,
    /// Windows 10 has neither a centered taskbar nor a Widgets button.
    pub windows_10: bool,
    /// Always false on Windows 10, which has no Widgets button.
    pub widgets_button_on: bool,
    /// Left edge of the Start button, i.e. where the taskbar icons begin.
    pub icons_left: Option<i32>,
    /// Right edge of the taskbar icons (screen px); `None` when unknown.
    pub apps_right: Option<i32>,
    /// Left edge of the system tray (screen px); `None` when unknown.
    pub tray_left: Option<i32>,
    /// Which end of the taskbar the strip is drawn at (set by the render).
    pub side: Side,
}

/// Free width between the taskbar icons and the system tray, minus a margin on each side.
/// Zero when either edge is unknown or the gap is crowded.
pub fn right_gap_width(info: &TaskbarInfo, margin: i32) -> i32 {
    match (info.apps_right, info.tray_left) {
        (Some(apps_right), Some(tray_left)) => (tray_left - apps_right - 2 * margin).max(0),
        _ => 0,
    }
}

/// Left edge of a strip of `strip_width` that ends one margin before the tray.
pub fn right_strip_left(tray_left: i32, margin: i32, strip_width: i32) -> i32 {
    tray_left - margin - strip_width
}

/// Half-width assumed for the centered icon group when UI Automation is unavailable
/// (room for ~18 icons), so the strip errs on the side of not covering them.
const FALLBACK_HALF_ICONS_DIP: i32 = 400;

/// Free width left of the icons, from the Start button or a conservative estimate.
pub fn available_width(info: &TaskbarInfo, margin: i32) -> i32 {
    if info.left_aligned {
        return 0;
    }
    let free = match info.icons_left {
        Some(icons_left) => icons_left - info.rect.left,
        None => info.rect.width() / 2 - (FALLBACK_HALF_ICONS_DIP * info.dpi as i32) / 96,
    };
    (free - 2 * margin).max(0)
}

/// Caches the UI Automation lookup of where the taskbar icons start.
///
/// Successful lookups refresh every [`ICON_PERIOD_TICKS`] ticks or when invalidated (icons
/// shift as apps open and close). Failed lookups keep the last good value and back off.
#[derive(Debug, Default)]
pub struct IconCache {
    last: Option<i32>,
    ticks: u32,
    failures: u32,
    dirty: bool,
    tried: bool,
}

pub const ICON_PERIOD_TICKS: u32 = 5;
const ICON_MAX_BACKOFF_TICKS: u32 = 30;

/// Ticks between lookups after `failures` failed ones in a row: 5, 10, 20, then 30.
fn backoff(failures: u32) -> u32 {
    (ICON_PERIOD_TICKS << failures.min(4)).min(ICON_MAX_BACKOFF_TICKS)
}

impl IconCache {
    pub fn needs_lookup(&self) -> bool {
        !self.tried || (self.dirty && self.failures == 0) || self.ticks >= backoff(self.failures)
    }

    pub fn on_lookup(&mut self, result: Option<i32>) {
        self.tried = true;
        self.ticks = 0;
        self.dirty = false;
        match result {
            Some(v) => {
                self.last = Some(v);
                self.failures = 0;
            }
            None => self.failures = self.failures.saturating_add(1),
        }
    }

    pub fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    pub fn value(&self) -> Option<i32> {
        self.last
    }
}

/// Caches the right edge of the app buttons for the right placement (#10). Even warm, the
/// UI Automation read costs about 2.7 ms, so a successful read is reused for
/// [`ICON_PERIOD_TICKS`] ticks or until invalidated (apps open and close). A failed read is
/// never kept as a guess: the value is dropped (the strip stays hidden) and reads back off
/// like the icons' ([`IconCache`]), ignoring the cheap dirty mark until one succeeds.
#[derive(Debug, Default)]
pub struct AppsEdgeCache {
    last: Option<i32>,
    ticks: u32,
    failures: u32,
    dirty: bool,
    tried: bool,
}

impl AppsEdgeCache {
    pub fn needs_lookup(&self) -> bool {
        !self.tried || (self.dirty && self.failures == 0) || self.ticks >= backoff(self.failures)
    }

    pub fn on_lookup(&mut self, result: Option<i32>) {
        self.tried = true;
        self.last = result;
        self.ticks = 0;
        self.dirty = false;
        self.failures = match result {
            Some(_) => 0,
            None => self.failures.saturating_add(1),
        };
    }

    pub fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Cheap mark (e.g. a foreground change): read again soon, unless reads are failing.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// The taskbar itself changed (Explorer, display or settings): read on the next render
    /// and forget the back-off.
    pub fn reset(&mut self) {
        self.dirty = true;
        self.failures = 0;
    }

    pub fn value(&self) -> Option<i32> {
        self.last
    }
}

/// Hide only for fullscreen on the taskbar's own monitor: the shell's "busy" state is
/// system-wide, so it is trusted only when the foreground window is on that monitor.
pub fn hides_for_fullscreen(
    shell_busy: bool,
    fg_covers_monitor: bool,
    fg_monitor: Rect,
    taskbar_monitor: Rect,
) -> bool {
    fg_monitor == taskbar_monitor && (shell_busy || fg_covers_monitor)
}

/// A non-shell foreground window covering the whole monitor.
pub fn is_fullscreen(fg: Rect, monitor: Rect, fg_is_shell: bool) -> bool {
    !fg_is_shell
        && fg.left <= monitor.left
        && fg.top <= monitor.top
        && fg.right >= monitor.right
        && fg.bottom >= monitor.bottom
}

/// First Windows 11 build; Windows 10 has no centered taskbar and no `TaskbarAl` value.
pub const WIN11_FIRST_BUILD: u32 = 22000;

/// Windows 10 or older. An unknown build is treated as Windows 11: every supported
/// release reports its build through `RtlGetVersion`.
pub fn is_windows_10(build: Option<u32>) -> bool {
    build.is_some_and(|b| b < WIN11_FIRST_BUILD)
}

/// Whether the taskbar icons start at the left edge, leaving no free corner for the strip.
/// `build` is the Windows build number; `taskbar_al` is the `TaskbarAl` registry value.
pub fn icons_left_aligned(build: Option<u32>, taskbar_al: Option<u32>) -> bool {
    is_windows_10(build) || taskbar_al == Some(0)
}

/// Which end of the taskbar the strip sits at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
    #[default]
    Left,
    Right,
}

/// Sides to try, most preferred first. `Auto` behaves like `Left`.
pub fn side_order(placement: Placement) -> [Side; 2] {
    match placement {
        Placement::Auto | Placement::Left => [Side::Left, Side::Right],
        Placement::Right => [Side::Right, Side::Left],
    }
}

/// Whether the user picked a side by hand (everything except `Auto`).
pub fn is_explicit(placement: Placement) -> bool {
    placement != Placement::Auto
}

/// Where the strip ended up for a given preference order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placed {
    /// The first side in the order has room.
    Preferred(Side),
    /// Only the second side has room.
    Fallback(Side),
    /// Neither side has room.
    Hidden,
}

/// Picks the first side in `order` for which `fits` holds.
pub fn place(order: [Side; 2], fits: impl Fn(Side) -> bool) -> Placed {
    if fits(order[0]) {
        Placed::Preferred(order[0])
    } else if fits(order[1]) {
        Placed::Fallback(order[1])
    } else {
        Placed::Hidden
    }
}

/// Whether the Widgets button is present and enabled. Windows 10 has no Widgets button
/// at all; on Windows 11 a missing `TaskbarDa` means the default, which is on.
pub fn widgets_button_on(build: Option<u32>, taskbar_da: Option<u32>) -> bool {
    !is_windows_10(build) && taskbar_da != Some(0)
}

const ADVANCED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: data/size describe a valid u32 buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

/// Windows build number. `RtlGetVersion` reports the real build regardless of the app
/// manifest (`GetVersionExW` reports Windows 8's 9200 without one, e.g. in test binaries).
fn windows_build() -> Option<u32> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: the size field is set and info is a valid out pointer.
    unsafe { RtlGetVersion(&mut info) }.ok().ok()?;
    Some(info.dwBuildNumber)
}

/// Warm reads of the Start button's rectangle allowed before the element is searched for
/// again, in case the taskbar rebuilt its buttons without invalidating the old one.
const FULL_FIND_EVERY: u32 = 30;

/// UI Automation objects kept between lookups (#86): creating the client and searching the
/// taskbar tree costs ~14 ms, re-reading the Start button's rectangle a fraction of that.
#[derive(Default)]
struct StartButton {
    uia: Option<IUIAutomation>,
    /// The taskbar window the element was found under.
    taskbar: isize,
    element: Option<IUIAutomationElement>,
    warm_reads: u32,
}

/// Whether the Start button must be searched for instead of re-read.
fn needs_full_find(has_element: bool, same_taskbar: bool, warm_reads: u32) -> bool {
    !has_element || !same_taskbar || warm_reads >= FULL_FIND_EVERY
}

fn left_edge(element: &IUIAutomationElement) -> Option<i32> {
    // SAFETY: a live cross-process property read; it fails once the element is gone.
    let rect = unsafe { element.CurrentBoundingRectangle() }.ok()?;
    (rect.right > rect.left).then_some(rect.left)
}

impl StartButton {
    fn find(&mut self, taskbar: HWND) -> Option<i32> {
        // SAFETY: COM is initialized on the UI thread by main; all calls are on that thread.
        unsafe {
            if self.uia.is_none() {
                self.uia = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
            }
            let uia = self.uia.as_ref()?;
            let root = uia.ElementFromHandle(taskbar).ok()?;
            let cond = uia
                .CreatePropertyCondition(
                    UIA_AutomationIdPropertyId,
                    &VARIANT::from(BSTR::from("StartButton")),
                )
                .ok()?;
            let start = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
            let left = left_edge(&start)?;
            self.element = Some(start);
            self.taskbar = taskbar.0 as isize;
            self.warm_reads = 0;
            Some(left)
        }
    }

    fn left(&mut self, taskbar: HWND) -> Option<i32> {
        let same_taskbar = self.taskbar == taskbar.0 as isize;
        if !needs_full_find(self.element.is_some(), same_taskbar, self.warm_reads)
            && let Some(left) = self.element.as_ref().and_then(left_edge)
        {
            self.warm_reads += 1;
            return Some(left);
        }
        self.element = None;
        let found = self.find(taskbar);
        if found.is_none() {
            // Start from a new client next time; a failed one may be tied to a dead Explorer.
            self.uia = None;
        }
        found
    }
}

thread_local! {
    static START_BUTTON: std::cell::Cell<Option<StartButton>> = const { std::cell::Cell::new(None) };
}

fn start_button_left(taskbar: HWND) -> Option<i32> {
    // UI Automation can pump messages, so a render can re-enter this lookup. The cache is
    // taken out for the call: a nested lookup starts cold instead of sharing it.
    let mut cache = START_BUTTON.take().unwrap_or_default();
    let left = cache.left(taskbar);
    START_BUTTON.set(Some(cache));
    left
}

pub fn taskbar_hwnd() -> Option<HWND> {
    // SAFETY: plain window lookup.
    unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok()
}

/// A taskbar wider than tall (at the bottom or top of the screen).
pub fn is_horizontal(rect: Rect) -> bool {
    rect.width() > rect.height()
}

/// Whether the app and tray edges are read: only when asked for and on a horizontal
/// taskbar. A vertical taskbar has no gap beside the tray, so the right side never fits there.
pub fn reads_edges(with_edges: bool, rect: Rect) -> bool {
    with_edges && is_horizontal(rect)
}

/// Queries the primary taskbar. `with_icons` runs the (slower) UI Automation lookup of the
/// Start button. The tray edge is read only when `with_edges` is set on a horizontal taskbar
/// ([`reads_edges`]), and the app edge (UI Automation, see [`AppsEdgeCache`]) only then and
/// when `with_apps` is set. [`TaskbarInfo::side`] is left at [`Side::Left`]; the render sets
/// the side it draws.
pub fn query(with_icons: bool, with_apps: bool, with_edges: bool) -> Option<TaskbarInfo> {
    let hwnd = taskbar_hwnd()?;
    let mut rect = RECT::default();
    // SAFETY: hwnd is a live window; rect is a valid out pointer.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    let rect = Rect::from(rect);
    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: cbSize is set.
    unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi) }
        .ok()
        .ok()?;
    let monitor = Rect::from(mi.rcMonitor);
    let mut abd = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        ..Default::default()
    };
    // SAFETY: cbSize is set.
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut abd) } as u32;
    let build = windows_build();
    let overlap = rect.bottom.min(monitor.bottom) - rect.top.max(monitor.top);
    let windows_10 = is_windows_10(build);
    let left_aligned = icons_left_aligned(build, reg_dword(ADVANCED_KEY, "TaskbarAl"));
    let (apps_right, tray_left) = if reads_edges(with_edges, rect) {
        (
            with_apps
                .then(|| edges::apps_right(hwnd, rect, windows_10))
                .flatten(),
            edges::tray_left(hwnd, rect),
        )
    } else {
        (None, None)
    };
    Some(TaskbarInfo {
        rect,
        monitor,
        // SAFETY: hwnd is a live window.
        dpi: unsafe { GetDpiForWindow(hwnd) }.max(96),
        autohide: state & ABS_AUTOHIDE != 0,
        visible: overlap > 4,
        left_aligned,
        windows_10,
        widgets_button_on: widgets_button_on(build, reg_dword(ADVANCED_KEY, "TaskbarDa")),
        icons_left: if with_icons {
            start_button_left(hwnd)
        } else {
            None
        },
        apps_right,
        tray_left,
        side: Side::Left,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_order_table() {
        assert_eq!(side_order(Placement::Auto), [Side::Left, Side::Right]);
        assert_eq!(side_order(Placement::Left), [Side::Left, Side::Right]);
        assert_eq!(side_order(Placement::Right), [Side::Right, Side::Left]);
    }

    #[test]
    fn auto_is_not_explicit() {
        assert!(!is_explicit(Placement::Auto));
        assert!(is_explicit(Placement::Left));
        assert!(is_explicit(Placement::Right));
    }

    #[test]
    fn place_table() {
        use Side::{Left, Right};
        let lr = [Left, Right];
        let rl = [Right, Left];
        let fits = |l: bool, r: bool| move |s: Side| if s == Left { l } else { r };
        assert_eq!(place(lr, fits(true, true)), Placed::Preferred(Left));
        assert_eq!(place(lr, fits(true, false)), Placed::Preferred(Left));
        assert_eq!(place(lr, fits(false, true)), Placed::Fallback(Right));
        assert_eq!(place(lr, fits(false, false)), Placed::Hidden);
        assert_eq!(place(rl, fits(true, true)), Placed::Preferred(Right));
        assert_eq!(place(rl, fits(false, true)), Placed::Preferred(Right));
        assert_eq!(place(rl, fits(true, false)), Placed::Fallback(Left));
        assert_eq!(place(rl, fits(false, false)), Placed::Hidden);
    }

    fn info(icons_left: Option<i32>) -> TaskbarInfo {
        TaskbarInfo {
            rect: Rect {
                left: 0,
                top: 1032,
                right: 1920,
                bottom: 1080,
            },
            monitor: Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            dpi: 96,
            autohide: false,
            visible: true,
            left_aligned: false,
            windows_10: false,
            widgets_button_on: false,
            icons_left,
            apps_right: None,
            tray_left: None,
            side: Side::Left,
        }
    }

    fn right_info(apps_right: Option<i32>, tray_left: Option<i32>) -> TaskbarInfo {
        TaskbarInfo {
            apps_right,
            tray_left,
            ..info(None)
        }
    }

    #[test]
    fn right_gap_is_space_between_apps_and_tray() {
        let i = right_info(Some(900), Some(1600));
        assert_eq!(right_gap_width(&i, 8), 684);
    }

    #[test]
    fn right_gap_is_zero_when_an_edge_is_missing() {
        assert_eq!(right_gap_width(&right_info(None, Some(1600)), 8), 0);
        assert_eq!(right_gap_width(&right_info(Some(900), None), 8), 0);
        assert_eq!(right_gap_width(&right_info(None, None), 8), 0);
    }

    #[test]
    fn right_gap_is_zero_when_crowded() {
        let i = right_info(Some(1595), Some(1600));
        assert_eq!(right_gap_width(&i, 8), 0);
    }

    #[test]
    fn right_gap_scales_margin_with_dpi() {
        use crate::ui::app::MARGIN_DIP;
        use crate::ui::render::to_px;
        let margin = to_px(MARGIN_DIP, 144);
        assert_eq!(margin, 12);
        let i = right_info(Some(900), Some(1600));
        assert_eq!(right_gap_width(&i, margin), 700 - 2 * margin);
        assert_eq!(right_gap_width(&i, margin), 676);
    }

    #[test]
    fn edges_are_read_only_when_asked_on_a_horizontal_taskbar() {
        let bottom = info(None).rect;
        let vertical = Rect {
            left: 1872,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(is_horizontal(bottom));
        assert!(!is_horizontal(vertical));
        assert!(reads_edges(true, bottom));
        assert!(!reads_edges(true, vertical));
        assert!(!reads_edges(false, bottom));
        assert!(!reads_edges(false, vertical));
    }

    /// Ticks until the cache asks for a read again.
    fn ticks_to_lookup(c: &mut AppsEdgeCache) -> u32 {
        let mut n = 0;
        while !c.needs_lookup() {
            c.tick();
            n += 1;
            assert!(n <= 100, "never asks again");
        }
        n
    }

    #[test]
    fn apps_edge_failures_back_off_like_the_icons() {
        let mut c = AppsEdgeCache::default();
        assert!(c.needs_lookup());
        let mut waits = Vec::new();
        for _ in 0..5 {
            c.on_lookup(None);
            // A failed read is never cached as a guess.
            assert_eq!(c.value(), None);
            waits.push(ticks_to_lookup(&mut c));
        }
        assert_eq!(waits, [10, 20, 30, 30, 30]);
        // A success ends the back-off.
        c.on_lookup(Some(1159));
        assert_eq!(c.value(), Some(1159));
        assert_eq!(ticks_to_lookup(&mut c), ICON_PERIOD_TICKS);
    }

    #[test]
    fn apps_edge_dirty_mark_waits_out_the_back_off_but_reset_does_not() {
        let mut c = AppsEdgeCache::default();
        c.on_lookup(None);
        // Foreground changes do not retry a failing read on every switch.
        c.invalidate();
        assert!(!c.needs_lookup());
        // Explorer, display or settings changed: read right away and drop the back-off.
        c.reset();
        assert!(c.needs_lookup());
        c.on_lookup(None);
        assert_eq!(ticks_to_lookup(&mut c), 10);
    }

    #[test]
    fn apps_edge_refreshes_on_the_icon_cadence() {
        let mut c = AppsEdgeCache::default();
        c.on_lookup(Some(1159));
        for _ in 1..ICON_PERIOD_TICKS {
            c.tick();
            assert!(!c.needs_lookup());
        }
        c.tick();
        assert!(c.needs_lookup());
        c.on_lookup(Some(1203));
        assert!(!c.needs_lookup());
        assert_eq!(c.value(), Some(1203));
    }

    #[test]
    fn apps_edge_invalidate_forces_a_read_and_a_failure_drops_the_value() {
        let mut c = AppsEdgeCache::default();
        c.on_lookup(Some(1159));
        c.invalidate();
        assert!(c.needs_lookup());
        c.on_lookup(None);
        assert_eq!(c.value(), None);
    }

    #[test]
    fn strip_left_is_flush_with_tray() {
        assert_eq!(right_strip_left(1600, 8, 300), 1292);
    }

    #[test]
    fn available_uses_icons_left() {
        assert_eq!(available_width(&info(Some(700)), 8), 684);
    }

    #[test]
    fn available_fallback_estimate() {
        assert_eq!(available_width(&info(None), 8), 544);
    }

    #[test]
    fn available_fallback_scales_with_dpi() {
        let i = TaskbarInfo {
            dpi: 144,
            ..info(None)
        };
        assert_eq!(available_width(&i, 8), 960 - 600 - 16);
    }

    #[test]
    fn available_left_aligned_is_zero() {
        let i = TaskbarInfo {
            left_aligned: true,
            ..info(Some(700))
        };
        assert_eq!(available_width(&i, 8), 0);
    }

    #[test]
    fn windows10_is_always_left_aligned() {
        // Windows 10 never writes TaskbarAl; Start and search sit at the far left.
        assert!(icons_left_aligned(Some(19045), None));
        assert!(icons_left_aligned(Some(19045), Some(1)));
    }

    #[test]
    fn windows11_follows_taskbar_alignment() {
        assert!(
            !icons_left_aligned(Some(22631), None),
            "centered is the default"
        );
        assert!(!icons_left_aligned(Some(22631), Some(1)));
        assert!(icons_left_aligned(Some(22631), Some(0)));
        assert!(!icons_left_aligned(Some(WIN11_FIRST_BUILD), None));
    }

    #[test]
    fn unknown_build_follows_taskbar_alignment() {
        assert!(!icons_left_aligned(None, None));
        assert!(icons_left_aligned(None, Some(0)));
    }

    #[test]
    fn windows10_has_no_widgets_button() {
        assert!(!widgets_button_on(Some(19045), None));
        assert!(!widgets_button_on(Some(19045), Some(1)));
        assert!(!widgets_button_on(Some(19045), Some(0)));
    }

    #[test]
    fn windows11_widgets_button_follows_taskbar_da() {
        assert!(
            widgets_button_on(Some(22631), None),
            "a missing TaskbarDa is the default, which is on"
        );
        assert!(widgets_button_on(Some(WIN11_FIRST_BUILD), Some(1)));
        assert!(!widgets_button_on(Some(22631), Some(0)));
    }

    #[test]
    fn unknown_build_keeps_widgets_button_heuristic() {
        assert!(widgets_button_on(None, None));
        assert!(!widgets_button_on(None, Some(0)));
    }

    #[test]
    fn available_never_negative() {
        assert_eq!(available_width(&info(Some(5)), 8), 0);
    }

    #[test]
    fn fullscreen_true_when_covering() {
        let m = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(is_fullscreen(m, m, false));
        let bigger = Rect {
            left: -8,
            top: -8,
            right: 1928,
            bottom: 1088,
        };
        assert!(is_fullscreen(bigger, m, false));
    }

    #[test]
    fn fullscreen_false_for_desktop() {
        let m = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(!is_fullscreen(m, m, true));
    }

    #[test]
    fn fullscreen_false_when_smaller() {
        let m = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let maximized = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1032,
        };
        assert!(!is_fullscreen(maximized, m, false));
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore = "reads the live taskbar"]
    fn query_smoke() {
        // SAFETY: test thread initializes COM for UI Automation.
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let info = super::query(true, true, true).expect("taskbar");
        println!("{info:?} available={}", super::available_width(&info, 8));
        assert!(info.rect.width() > 0 && info.dpi >= 96);
    }
}

#[cfg(test)]
mod start_button_tests {
    use super::{FULL_FIND_EVERY, needs_full_find};

    #[test]
    fn a_known_button_on_the_same_taskbar_is_only_re_read() {
        assert!(!needs_full_find(true, true, 0));
        assert!(!needs_full_find(true, true, FULL_FIND_EVERY - 1));
    }

    #[test]
    fn the_button_is_searched_for_when_unknown_moved_or_due() {
        assert!(needs_full_find(false, true, 0));
        // Explorer restarted: the taskbar is a new window.
        assert!(needs_full_find(true, false, 0));
        assert!(needs_full_find(true, true, FULL_FIND_EVERY));
    }
}

#[cfg(test)]
mod icon_cache_tests {
    use super::*;

    #[test]
    fn first_render_looks_up() {
        assert!(IconCache::default().needs_lookup());
    }

    #[test]
    fn success_is_cached_until_period_or_invalidate() {
        let mut c = IconCache::default();
        c.on_lookup(Some(700));
        assert_eq!(c.value(), Some(700));
        assert!(!c.needs_lookup());
        c.invalidate();
        assert!(c.needs_lookup());
        c.on_lookup(Some(650));
        for _ in 0..4 {
            c.tick();
        }
        assert!(!c.needs_lookup());
        c.tick();
        assert!(c.needs_lookup());
    }

    #[test]
    fn failure_keeps_last_good_value_and_backs_off() {
        let mut c = IconCache::default();
        c.on_lookup(Some(700));
        c.invalidate();
        c.on_lookup(None);
        assert_eq!(c.value(), Some(700));
        // A failed lookup is not retried on every invalidate (e.g. each foreground change).
        c.invalidate();
        assert!(!c.needs_lookup());
        for _ in 0..9 {
            c.tick();
        }
        assert!(!c.needs_lookup());
        c.tick();
        assert!(c.needs_lookup());
    }
}

#[cfg(test)]
mod fullscreen_monitor_tests {
    use super::*;

    const PRIMARY: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    const SECOND: Rect = Rect {
        left: 1920,
        top: 0,
        right: 4480,
        bottom: 1440,
    };

    #[test]
    fn fullscreen_on_other_monitor_keeps_strip() {
        assert!(!hides_for_fullscreen(true, true, SECOND, PRIMARY));
    }

    #[test]
    fn shell_busy_on_taskbar_monitor_hides() {
        assert!(hides_for_fullscreen(true, false, PRIMARY, PRIMARY));
    }

    #[test]
    fn covering_window_on_taskbar_monitor_hides() {
        assert!(hides_for_fullscreen(false, true, PRIMARY, PRIMARY));
    }

    #[test]
    fn nothing_fullscreen_keeps_strip() {
        assert!(!hides_for_fullscreen(false, false, PRIMARY, PRIMARY));
    }
}
