//! Live timing of what one strip tick does (#86). Run with
//! `cargo test --release -- --ignored idle_cost --nocapture`.

use super::layout::layout;
use super::render::{GAP, Renderer, SEP_W, to_px};
use super::taskbar;
use super::view::build_items;
use crate::config::Config;
use crate::hw::Sampler;
use crate::i18n::Lang;
use crate::limits::ProviderStatus;
use std::time::Instant;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

/// Mean microseconds of `f` over `n` runs.
fn time_us(n: u32, mut f: impl FnMut()) -> f64 {
    let start = Instant::now();
    for _ in 0..n {
        f();
    }
    start.elapsed().as_secs_f64() * 1e6 / f64::from(n)
}

#[test]
#[ignore = "times the live taskbar, counters and renderer"]
fn idle_cost() {
    // SAFETY: test thread initializes COM for UI Automation.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let info = taskbar::query(false).expect("taskbar");
    let dpi = info.dpi;
    let mut sampler = Sampler::new();
    sampler.sample();
    std::thread::sleep(std::time::Duration::from_secs(1));
    let hw = sampler.sample();
    let renderer = Renderer::new().expect("renderer");
    let loading = ProviderStatus::Loading;
    let cfg = Config::default();
    let items = build_items(&cfg, &hw, &loading, &loading, "P", Lang::En);
    let metrics = renderer.measure(&items, dpi);
    let kinds: Vec<_> = items.iter().map(|i| i.kind).zip(metrics.clone()).collect();
    let l = layout(
        &kinds,
        4000,
        to_px(GAP, dpi),
        to_px(SEP_W, dpi),
        renderer.ellipsis_px(dpi),
    );
    // SAFETY: a hidden layered window that only receives the painted bitmap.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            w!("Static"),
            w!("CCT bench"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        )
    }
    .expect("window");
    let size = (l.width, info.rect.height());

    let rows = [
        (
            "taskbar query",
            time_us(200, || {
                taskbar::query(false);
            }),
        ),
        (
            "icon lookup",
            time_us(20, || {
                taskbar::query(true);
            }),
        ),
        (
            "fullscreen check",
            time_us(200, || {
                super::app::foreground_is_fullscreen(info.monitor);
            }),
        ),
        (
            "hw sample",
            time_us(50, || {
                sampler.sample();
            }),
        ),
        (
            "measure",
            time_us(200, || {
                renderer.measure(&items, dpi);
            }),
        ),
        (
            "paint",
            time_us(200, || {
                renderer
                    .paint(hwnd, (0, 0), size, dpi, &items, &metrics, &l, false)
                    .expect("paint");
            }),
        ),
    ];
    // SAFETY: the window was created above on this thread.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    for (name, us) in rows {
        println!("{name:<18}{us:>10.1} us");
    }
}
