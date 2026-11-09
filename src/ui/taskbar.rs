//! Taskbar geometry and environment: where the free left corner is and when to hide.

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
    CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_AutomationIdPropertyId,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{ABM_GETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
use windows::core::{BSTR, HSTRING, w};

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

impl IconCache {
    fn period(&self) -> u32 {
        (ICON_PERIOD_TICKS << self.failures.min(4)).min(ICON_MAX_BACKOFF_TICKS)
    }

    pub fn needs_lookup(&self) -> bool {
        !self.tried || (self.dirty && self.failures == 0) || self.ticks >= self.period()
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

fn start_button_left(taskbar: HWND) -> Option<i32> {
    // SAFETY: COM is initialized on the UI thread by main; all calls are on that thread.
    unsafe {
        let uia: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let root = uia.ElementFromHandle(taskbar).ok()?;
        let cond = uia
            .CreatePropertyCondition(
                UIA_AutomationIdPropertyId,
                &VARIANT::from(BSTR::from("StartButton")),
            )
            .ok()?;
        let start = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
        let rect = start.CurrentBoundingRectangle().ok()?;
        (rect.right > rect.left).then_some(rect.left)
    }
}

pub fn taskbar_hwnd() -> Option<HWND> {
    // SAFETY: plain window lookup.
    unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok()
}

/// Queries the primary taskbar. `with_icons` runs the (slower) UI Automation lookup.
pub fn query(with_icons: bool) -> Option<TaskbarInfo> {
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
    Some(TaskbarInfo {
        rect,
        monitor,
        // SAFETY: hwnd is a live window.
        dpi: unsafe { GetDpiForWindow(hwnd) }.max(96),
        autohide: state & ABS_AUTOHIDE != 0,
        visible: overlap > 4,
        left_aligned: icons_left_aligned(build, reg_dword(ADVANCED_KEY, "TaskbarAl")),
        windows_10: is_windows_10(build),
        widgets_button_on: widgets_button_on(build, reg_dword(ADVANCED_KEY, "TaskbarDa")),
        icons_left: if with_icons {
            start_button_left(hwnd)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
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
        let info = super::query(true).expect("taskbar");
        println!("{info:?} available={}", super::available_width(&info, 8));
        assert!(info.rect.width() > 0 && info.dpi >= 96);
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
