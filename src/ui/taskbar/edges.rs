//! Edges for the right placement (#10): where the app buttons end and the tray begins.
//!
//! Both edges are screen pixels. Any failed lookup gives `None` for that edge, never a
//! guess: a wrong edge would put the strip over taskbar icons.
//!
//! Windows 11 (verified on build 26200, primary taskbar 0,1032-1920,1080):
//! - Tray: the `TrayNotifyWnd` child window of `Shell_TrayWnd`. Its rectangle starts at the
//!   "Show hidden icons" chevron (UI Automation id `SystemTrayIcon`, class
//!   `SystemTray.NormalButton`) and runs through the notification icons, input indicator,
//!   network, volume, clock and the Show desktop sliver, so its left edge is the left edge of
//!   the whole right group.
//! - Apps: the UI Automation element with id `TaskbarFrame` (class
//!   `Taskbar.TaskbarFrameAutomationPeer`), inside the XAML island under `Shell_TrayWnd`. Its
//!   children are the `TaskbarFrameRepeater` group, which holds Start (`StartButton`), Task
//!   view (`TaskViewButton`) and the app buttons (`Appid: …`, class
//!   `Taskbar.TaskListButtonAutomationPeer`), and any other button the shell places beside it
//!   (e.g. Widgets). The largest right edge among those children is where the apps end. The
//!   legacy `ReBarWindow32` / `MSTaskSwWClass` windows still exist on Windows 11 but no longer
//!   match the buttons, so they are not used there.
//!
//! Windows 10 (not testable on the development machine; from the shell's window classes):
//! - Tray: the same `TrayNotifyWnd` child window (notification icons, clock, action center).
//! - Apps: `Shell_TrayWnd` → `ReBarWindow32` → `MSTaskSwWClass` → `MSTaskListWClass`, whose UI
//!   Automation children are the app buttons. `MSTaskSwWClass` itself spans its whole band up
//!   to the tray, so it is skipped; every other visible `ReBarWindow32` child (toolbars,
//!   deskbands) and every other visible `Shell_TrayWnd` child at or right of the rebar (e.g.
//!   News and interests) counts as occupied. With no buttons the apps end at the rebar's left
//!   edge (Start, search and Task view sit left of it).

use super::{Rect, needs_full_find};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationCacheRequest, IUIAutomationElement,
    TreeScope_Children, TreeScope_Descendants, UIA_AutomationIdPropertyId,
    UIA_BoundingRectanglePropertyId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetWindowRect, IsWindowVisible,
};
use windows::core::{BSTR, PCWSTR, w};

/// Right edge of the app buttons: the largest `right` among non-empty `rects`, or
/// `left_group_right` when there are none. `None` when both are missing.
pub(super) fn apps_right_of(rects: &[Rect], left_group_right: Option<i32>) -> Option<i32> {
    rects
        .iter()
        .filter(|r| r.width() > 0)
        .map(|r| r.right)
        .max()
        .or(left_group_right)
}

/// An edge is trusted only inside the taskbar's own span.
fn within(bar: Rect, x: i32) -> Option<i32> {
    (bar.left <= x && x <= bar.right).then_some(x)
}

fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    // SAFETY: hwnd came from a window lookup; r is a valid out pointer. A window that has
    // just been destroyed makes the call fail, which becomes None.
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    Some(Rect::from(r))
}

fn child(parent: HWND, class: PCWSTR) -> Option<HWND> {
    // SAFETY: plain child-window lookup by class.
    unsafe { FindWindowExW(Some(parent), None, class, None) }.ok()
}

/// Visible direct children of `parent` with their class names.
fn visible_children(parent: HWND) -> Vec<(String, Rect)> {
    let mut out = Vec::new();
    let mut after = None;
    // SAFETY: plain child-window enumeration; the class buffer outlives each call.
    unsafe {
        while let Ok(h) = FindWindowExW(Some(parent), after, PCWSTR::null(), PCWSTR::null()) {
            after = Some(h);
            if !IsWindowVisible(h).as_bool() {
                continue;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(h, &mut buf).max(0) as usize;
            if let Some(r) = window_rect(h) {
                out.push((String::from_utf16_lossy(&buf[..n]), r));
            }
        }
    }
    out
}

/// Left edge of the system tray (`TrayNotifyWnd`, see the module notes for each OS).
pub(super) fn tray_left(taskbar: HWND, bar: Rect) -> Option<i32> {
    let r = window_rect(child(taskbar, w!("TrayNotifyWnd"))?)?;
    if r.width() <= 0 {
        return None;
    }
    within(bar, r.left)
}

/// UI Automation objects kept between lookups, like the Start button's (#86): the client,
/// a cache request for bounding rectangles, and the element whose children are the apps.
#[derive(Default)]
struct AppButtons {
    uia: Option<IUIAutomation>,
    rects_request: Option<IUIAutomationCacheRequest>,
    /// The taskbar window the container was found under.
    taskbar: isize,
    container: Option<IUIAutomationElement>,
    warm_reads: u32,
}

impl AppButtons {
    /// Bounding rectangles of the container's children, fetched in one cross-process call.
    fn child_rects(&self, container: &IUIAutomationElement) -> Option<Vec<Rect>> {
        let uia = self.uia.as_ref()?;
        let request = self.rects_request.as_ref()?;
        // SAFETY: COM is initialized on the UI thread; a dead element makes the calls fail.
        unsafe {
            let all = uia.CreateTrueCondition().ok()?;
            let children = container
                .FindAllBuildCache(TreeScope_Children, &all, request)
                .ok()?;
            let n = children.Length().ok()?;
            let mut rects = Vec::with_capacity(n.max(0) as usize);
            for i in 0..n {
                let rect = children
                    .GetElement(i)
                    .ok()?
                    .CachedBoundingRectangle()
                    .ok()?;
                rects.push(Rect::from(rect));
            }
            Some(rects)
        }
    }

    fn find_container(&mut self, taskbar: HWND, windows_10: bool) -> Option<IUIAutomationElement> {
        // SAFETY: COM is initialized on the UI thread by main; all calls are on that thread.
        unsafe {
            if self.uia.is_none() {
                self.uia = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
                self.rects_request = None;
            }
            let uia = self.uia.as_ref()?;
            if self.rects_request.is_none() {
                let request = uia.CreateCacheRequest().ok()?;
                request.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
                self.rects_request = Some(request);
            }
            if windows_10 {
                let rebar = child(taskbar, w!("ReBarWindow32"))?;
                let band = child(rebar, w!("MSTaskSwWClass"))?;
                let list = child(band, w!("MSTaskListWClass"))?;
                uia.ElementFromHandle(list).ok()
            } else {
                let root = uia.ElementFromHandle(taskbar).ok()?;
                let cond = uia
                    .CreatePropertyCondition(
                        UIA_AutomationIdPropertyId,
                        &VARIANT::from(BSTR::from("TaskbarFrame")),
                    )
                    .ok()?;
                root.FindFirst(TreeScope_Descendants, &cond).ok()
            }
        }
    }

    /// Rectangles of the app buttons (Windows 10: children of `MSTaskListWClass`) or of the
    /// children of `TaskbarFrame` (Windows 11: the `TaskbarFrameRepeater` group holding Start,
    /// Task view and the app buttons, plus any button beside it such as Widgets).
    fn rects(&mut self, taskbar: HWND, windows_10: bool) -> Option<Vec<Rect>> {
        let same_taskbar = self.taskbar == taskbar.0 as isize;
        if !needs_full_find(self.container.is_some(), same_taskbar, self.warm_reads)
            && let Some(rects) = self.container.as_ref().and_then(|c| self.child_rects(c))
        {
            self.warm_reads += 1;
            return Some(rects);
        }
        self.container = None;
        let found = self
            .find_container(taskbar, windows_10)
            .and_then(|container| {
                let rects = self.child_rects(&container)?;
                self.container = Some(container);
                self.taskbar = taskbar.0 as isize;
                self.warm_reads = 0;
                Some(rects)
            });
        if found.is_none() {
            // Start from a new client next time; a failed one may be tied to a dead Explorer.
            self.uia = None;
            self.rects_request = None;
        }
        found
    }
}

thread_local! {
    static APP_BUTTONS: std::cell::Cell<Option<AppButtons>> = const { std::cell::Cell::new(None) };
}

fn app_rects(taskbar: HWND, windows_10: bool) -> Option<Vec<Rect>> {
    // UI Automation can pump messages, so a render can re-enter this lookup. The cache is
    // taken out for the call: a nested lookup starts cold instead of sharing it.
    let mut cache = APP_BUTTONS.take().unwrap_or_default();
    let rects = cache.rects(taskbar, windows_10);
    APP_BUTTONS.set(Some(cache));
    rects
}

/// Drops the cached app-button container, so the next lookup searches for it again.
pub fn invalidate_edges() {
    if let Some(mut cache) = APP_BUTTONS.take() {
        cache.container = None;
        APP_BUTTONS.set(Some(cache));
    }
}

/// Windows 10 bands that are not the task list but still occupy the taskbar between the
/// apps and the tray (see the module notes).
fn windows10_bands(taskbar: HWND) -> Option<(Rect, Vec<Rect>)> {
    let rebar = child(taskbar, w!("ReBarWindow32"))?;
    let rebar_rect = window_rect(rebar)?;
    let mut bands: Vec<Rect> = visible_children(rebar)
        .into_iter()
        .filter(|(class, _)| class != "MSTaskSwWClass")
        .map(|(_, r)| r)
        .collect();
    bands.extend(
        visible_children(taskbar)
            .into_iter()
            .filter(|(class, r)| {
                class != "ReBarWindow32" && class != "TrayNotifyWnd" && r.left >= rebar_rect.left
            })
            .map(|(_, r)| r),
    );
    Some((rebar_rect, bands))
}

/// Right edge of the taskbar icons (see the module notes for each OS).
pub(super) fn apps_right(taskbar: HWND, bar: Rect, windows_10: bool) -> Option<i32> {
    let mut rects = app_rects(taskbar, windows_10)?;
    let left_group_right = if windows_10 {
        let (rebar, bands) = windows10_bands(taskbar)?;
        rects.extend(bands);
        Some(rebar.left)
    } else {
        None
    };
    within(bar, apps_right_of(&rects, left_group_right)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, right: i32) -> Rect {
        Rect {
            left,
            top: 1032,
            right,
            bottom: 1080,
        }
    }

    #[test]
    fn apps_right_is_max_right_of_buttons() {
        let rects = [r(851, 895), r(1115, 1159), r(895, 939)];
        assert_eq!(apps_right_of(&rects, Some(800)), Some(1159));
        assert_eq!(apps_right_of(&rects, None), Some(1159));
        assert_eq!(apps_right_of(&[], Some(800)), Some(800));
        assert_eq!(apps_right_of(&[], None), None);
    }

    #[test]
    fn zero_width_rects_are_ignored() {
        let rects = [r(0, 0), r(851, 895), r(1500, 1500)];
        assert_eq!(apps_right_of(&rects, None), Some(895));
        assert_eq!(apps_right_of(&[r(0, 0)], Some(800)), Some(800));
        assert_eq!(apps_right_of(&[r(0, 0)], None), None);
    }

    #[test]
    fn edges_outside_the_taskbar_are_not_trusted() {
        let bar = r(0, 1920);
        assert_eq!(within(bar, 1666), Some(1666));
        assert_eq!(within(bar, 1920), Some(1920));
        assert_eq!(within(bar, -1), None);
        assert_eq!(within(bar, 1921), None);
    }
}

#[cfg(test)]
mod live {
    use crate::ui::taskbar::query;

    #[test]
    #[ignore = "reads the live taskbar"]
    fn live_edges_are_ordered() {
        // SAFETY: test thread initializes COM for UI Automation.
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let info = query(true, true, true).expect("taskbar");
        println!(
            "rect={:?} windows_10={} apps_right={:?} tray_left={:?}",
            info.rect, info.windows_10, info.apps_right, info.tray_left
        );
        let apps_right = info.apps_right.expect("apps_right");
        let tray_left = info.tray_left.expect("tray_left");
        assert!(
            info.rect.left <= apps_right,
            "apps_right inside the taskbar"
        );
        assert!(apps_right < tray_left, "apps end before the tray");
        assert!(tray_left <= info.rect.right, "tray_left inside the taskbar");
        // A second (warm) read agrees with the first.
        let again = query(true, true, true).expect("taskbar");
        assert_eq!(again.apps_right, Some(apps_right));
        assert_eq!(again.tray_left, Some(tray_left));
        super::super::invalidate_edges();
        let cold = query(true, true, true).expect("taskbar");
        assert_eq!(cold.apps_right, Some(apps_right));
    }

    #[test]
    #[ignore = "reads the live taskbar"]
    fn no_edges_unless_asked() {
        let info = query(false, true, false).expect("taskbar");
        assert_eq!((info.apps_right, info.tray_left), (None, None));
    }
}
