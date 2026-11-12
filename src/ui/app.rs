//! Application state and wiring: config, sampler, limits worker and the strip window.

use super::frame::{FrameCache, FrameKey};
use super::layout::{Layout, layout};
use super::notifications::{Notice, Notifications};
use super::placement::{Notice as PlacementNotice, PlacementWatch};
use super::render::{GAP, Renderer, SEP_W, to_px};
use super::strip;
use super::taskbar::{
    self, AppsEdgeCache, IconCache, Placed, Side, TaskbarInfo, available_width,
    hides_for_fullscreen, is_explicit, is_fullscreen, is_horizontal, place, right_gap_width,
    right_strip_left, side_order,
};
use super::view::{ItemView, TaskbarHints, build_items, tooltip_text};
use crate::config::{self, Config};
use crate::hw::{HwSnapshot, Sampler};
use crate::i18n::{Key, fmt, t};
use crate::i18n::{Lang, detect_lang};
use crate::limits::claude::ClaudeProvider;
use crate::limits::codex::CodexProvider;
use crate::limits::keepalive::{KeepAlive, Outcome, invitation_kinds, run_login_status};
use crate::limits::reset::{reset_refresh_delay, status_resets};
use crate::limits::state::{STALE_SECS, age_status};
use crate::limits::worker::{Control, Fetcher, LimitResult, spawn_queued};
use crate::limits::{Provider, ProviderStatus};
use crate::model::{Kind, provider_name};
use crate::timeutil::format_local_hhmm;
use crate::timeutil::now_unix;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowRect, PostMessageW,
};
use windows::core::HSTRING;

/// Gap between the strip and what it sits next to, in DIPs: the taskbar's left edge on the
/// left side; the app buttons and the tray on the right side (#10).
pub(crate) const MARGIN_DIP: f32 = 8.0;

mod notifications;

pub struct App {
    pub cfg: Config,
    pub cfg_path: PathBuf,
    pub lang: Lang,
    sampler: Sampler,
    pub hw: HwSnapshot,
    pub claude: ProviderStatus,
    pub codex: ProviderStatus,
    pub control: Arc<Control>,
    renderer: Renderer,
    pub(super) taskbar: Option<TaskbarInfo>,
    icons: IconCache,
    /// Right edge of the app buttons, for the right side (#10).
    apps_edge: AppsEdgeCache,
    /// Settled placement states, for one notice per state (#10).
    placement: PlacementWatch,
    /// Whether the left side had room at the last render; `None` before the first one.
    left_fitted_last: Option<bool>,
    /// Where the strip went at the last painted frame, re-observed while it stays current.
    placed_last: Option<Placed>,
    /// Monotonic clock for the placement watch.
    started: Instant,
    items: Vec<ItemView>,
    layout: Option<Layout>,
    frame: FrameCache,
    light: bool,
    pub shown: bool,
    inbox: Option<Receiver<LimitResult>>,
    keepalive_claude: KeepAlive,
    keepalive_codex: KeepAlive,
    notifications: Notifications,
    /// Label before the Claude gauges; resolved from the saved accounts (#74).
    claude_label: String,
}

fn light_theme() -> bool {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: data/size describe a valid u32 buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            &HSTRING::from("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && data == 1
}

pub(super) fn foreground_is_fullscreen(monitor: taskbar::Rect) -> bool {
    // SAFETY: read-only shell and window queries; failures mean "not fullscreen".
    unsafe {
        let shell_busy = SHQueryUserNotificationState().is_ok_and(|state| {
            [
                QUNS_BUSY,
                QUNS_RUNNING_D3D_FULL_SCREEN,
                QUNS_PRESENTATION_MODE,
            ]
            .contains(&state)
        });
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
            return false;
        }
        let fg_monitor = taskbar::Rect::from(mi.rcMonitor);
        let mut class = [0u16; 64];
        let n = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        let is_shell = matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd");
        let mut r = RECT::default();
        let covers =
            GetWindowRect(fg, &mut r).is_ok() && is_fullscreen(r.into(), monitor, is_shell);
        hides_for_fullscreen(shell_busy && !is_shell, covers, fg_monitor, monitor)
    }
}

impl App {
    pub fn new() -> windows::core::Result<App> {
        let cfg_path = config::default_path();
        let mut cfg = Config::load_from(&cfg_path);
        if !cfg.first_run_done {
            cfg.claude_enabled = claude_provider().has_login();
            cfg.codex_enabled = codex_provider().has_login();
            cfg.first_run_done = true;
            let _ = cfg.save_to(&cfg_path);
        }
        let control = Control::new(cfg.limits_interval_secs);
        let mut app = App {
            lang: detect_lang(),
            sampler: Sampler::new(),
            hw: HwSnapshot::default(),
            claude: ProviderStatus::Disabled,
            codex: ProviderStatus::Disabled,
            control,
            renderer: Renderer::new()?,
            taskbar: None,
            icons: IconCache::default(),
            apps_edge: AppsEdgeCache::default(),
            placement: PlacementWatch::default(),
            left_fitted_last: None,
            placed_last: None,
            started: Instant::now(),
            items: Vec::new(),
            layout: None,
            frame: FrameCache::default(),
            light: light_theme(),
            shown: false,
            inbox: None,
            keepalive_claude: KeepAlive::default(),
            keepalive_codex: KeepAlive::default(),
            notifications: Notifications::default(),
            claude_label: super::switcher_ui::live_claude_label(&cfg.claude_label),
            cfg,
            cfg_path,
        };
        app.apply_provider_flags();
        Ok(app)
    }

    /// Syncs worker flags and statuses with the config's provider switches.
    pub fn apply_provider_flags(&mut self) {
        for (kind, on, status) in [
            (Kind::Claude, self.cfg.claude_enabled, &mut self.claude),
            (Kind::Codex, self.cfg.codex_enabled, &mut self.codex),
        ] {
            self.control.set_enabled(kind, on);
            if !on {
                *status = ProviderStatus::Disabled;
            } else if *status == ProviderStatus::Disabled {
                *status = ProviderStatus::Loading;
            }
        }
        self.control.set_interval(self.cfg.limits_interval_secs);
    }

    pub fn apply_config(&mut self, cfg: Config) {
        let cfg = cfg.clamped();
        // Settings applies every click; only limit-related changes justify a fetch.
        let wake = self.cfg.needs_limits_wake(&cfg);
        let moved = self.cfg.placement != cfg.placement;
        self.cfg = cfg;
        if moved {
            // A new choice starts a fresh watch and a fresh look at both sides.
            self.placement.reset();
            self.invalidate_frame();
            self.apps_edge.reset();
            taskbar::invalidate_edges();
        }
        let _ = self.cfg.save_to(&self.cfg_path);
        self.refresh_claude_label();
        self.apply_provider_flags();
        if wake {
            self.control.wake();
        }
    }

    pub fn sample_hw(&mut self) {
        self.hw = self.sampler.sample();
    }

    pub fn refresh_instances(&mut self) {
        self.sampler.refresh_instances();
    }

    /// Re-reads which saved Claude account is live and its label.
    pub fn refresh_claude_label(&mut self) {
        self.claude_label = super::switcher_ui::live_claude_label(&self.cfg.claude_label);
    }

    fn keepalive_of(&mut self, kind: Kind) -> Option<(bool, &ProviderStatus, &mut KeepAlive)> {
        match kind {
            Kind::Claude => Some((
                self.cfg.keep_claude_login,
                &self.claude,
                &mut self.keepalive_claude,
            )),
            Kind::Codex => Some((
                self.cfg.keep_codex_login,
                &self.codex,
                &mut self.keepalive_codex,
            )),
            Kind::Hardware => None,
        }
    }

    /// Opt-in: once a provider's token has expired, let its official CLI renew it.
    fn keep_login_active(&mut self, kind: Kind, now: i64) {
        let Some((enabled, status, keepalive)) = self.keepalive_of(kind) else {
            return;
        };
        keepalive.observe(status, now);
        if !keepalive.should_run(enabled, status, now) {
            return;
        }
        keepalive.started(now);
        let control = self.control.clone();
        std::thread::spawn(move || {
            run_login_status(kind);
            // Re-read the credentials right away to see whether the CLI renewed them.
            control.wake();
        });
    }

    /// First-run invitation for the opt-in keep-alive: the message and the providers it
    /// covers, or `None` when every logged-in provider was already asked.
    pub fn keepalive_prompt(&self) -> Option<(String, Vec<Kind>)> {
        let kinds = invitation_kinds(
            self.cfg.keepalive_asked,
            self.cfg.codex_keepalive_asked,
            claude_provider().has_login(),
            codex_provider().has_login(),
        );
        if kinds.is_empty() {
            return None;
        }
        let names: Vec<&str> = kinds.iter().map(|k| provider_name(*k)).collect();
        let body = fmt(t(self.lang, Key::KeepAlivePromptBody), &names.join(" / "));
        Some((body, kinds))
    }

    pub fn answer_keepalive_prompt(&mut self, kinds: &[Kind], enable: bool) {
        let mut cfg = self.cfg.clone();
        for kind in kinds {
            match kind {
                Kind::Claude => {
                    cfg.keepalive_asked = true;
                    cfg.keep_claude_login = enable;
                }
                Kind::Codex => {
                    cfg.codex_keepalive_asked = true;
                    cfg.keep_codex_login = enable;
                }
                Kind::Hardware => {}
            }
        }
        self.apply_config(cfg);
    }

    pub fn theme_changed(&mut self) {
        self.light = light_theme();
    }

    /// Asks for a fresh UI Automation icon lookup (icons may have moved). Cheap: only marks
    /// the caches dirty.
    pub fn invalidate_icons(&mut self) {
        self.icons.invalidate();
        self.apps_edge.invalidate();
    }

    /// Asks for a fresh read of the app buttons' edge only (an app window came or went).
    pub fn invalidate_apps_edge(&mut self) {
        self.apps_edge.invalidate();
    }

    /// The taskbar itself changed: read the app edge on the next render, without back-off.
    pub fn reset_apps_edge(&mut self) {
        self.apps_edge.reset();
    }

    pub fn tick_icons(&mut self) {
        self.icons.tick();
        self.apps_edge.tick();
    }

    /// Statuses as of now: data older than the refresh cadence allows is not shown as current.
    fn displayed_statuses(&self) -> (ProviderStatus, ProviderStatus) {
        let now = now_unix();
        let max_age = STALE_SECS.max(2 * i64::from(self.cfg.limits_interval_secs));
        (
            age_status(&self.claude, now, max_age),
            age_status(&self.codex, now, max_age),
        )
    }

    /// Seconds until a one-off refresh just after the next known limit reset, if it comes
    /// before the regular refresh.
    pub fn reset_refresh_delay(&self) -> Option<u32> {
        let resets: Vec<i64> = [&self.claude, &self.codex]
            .into_iter()
            .flat_map(status_resets)
            .collect();
        reset_refresh_delay(&resets, now_unix(), self.cfg.limits_interval_secs)
    }

    pub fn set_inbox(&mut self, inbox: Receiver<LimitResult>) {
        self.inbox = Some(inbox);
    }

    /// Applies every limit result the worker has queued.
    pub fn drain_limits(&mut self) -> Vec<Notice> {
        let results: Vec<LimitResult> = match &self.inbox {
            Some(inbox) => inbox.try_iter().collect(),
            None => return Vec::new(),
        };
        results
            .into_iter()
            .flat_map(|result| self.on_limits(result))
            .collect()
    }

    /// Whether the next taskbar query should run the UI Automation icon lookup.
    pub fn needs_icons(&self) -> bool {
        self.icons.needs_lookup()
    }

    /// Whether the next taskbar query should read the app buttons' edge (when it reads the
    /// right-side edges at all).
    pub fn needs_apps_edge(&self) -> bool {
        self.apps_edge.needs_lookup()
    }

    /// Whether the next taskbar query should read the right-side edges.
    pub fn needs_edges(&self) -> bool {
        super::placement::wants_edges(
            self.cfg.placement,
            self.left_fitted_last,
            self.taskbar.as_ref().is_some_and(|t| t.left_aligned),
        )
    }

    /// The placement notice still to be delivered while its state is settled (#10).
    pub fn pending_placement_notice(&self) -> Option<PlacementNotice> {
        self.placement.pending_notice()
    }

    /// The placement notice has been delivered.
    pub fn placement_notice_sent(&mut self) {
        self.placement.notice_sent();
    }

    /// Explorer restarted: the placement watch starts a new grace period.
    pub fn restart_placement(&mut self) {
        self.placement.restart(self.started.elapsed().as_secs());
    }

    /// Recomputes position and layout from a fresh taskbar query, then paints or hides.
    /// The query runs outside the `App` borrow because UI Automation can pump messages.
    /// `with_edges` says the query was asked for the right-side edges, `looked_up_apps` that
    /// it also ran the app-edge lookup.
    pub fn render(
        &mut self,
        hwnd: HWND,
        info: Option<TaskbarInfo>,
        looked_up_icons: bool,
        looked_up_apps: bool,
        with_edges: bool,
    ) {
        let Some(mut info) = info else {
            strip::hide(hwnd, self);
            return;
        };
        if looked_up_icons {
            self.icons.on_lookup(info.icons_left);
        }
        info.icons_left = self.icons.value();
        let horizontal = is_horizontal(info.rect);
        let edges = with_edges && horizontal;
        if edges {
            if looked_up_apps {
                self.apps_edge.on_lookup(info.apps_right);
            }
            info.apps_right = self.apps_edge.value();
        }
        let dpi = info.dpi;
        let now = now_unix();
        let (claude, codex) = self.displayed_statuses();
        self.items = build_items(
            &self.cfg,
            &self.hw,
            &claude,
            &codex,
            &self.claude_label,
            self.lang,
            now,
        );
        let margin = to_px(MARGIN_DIP, dpi);
        let key = FrameKey {
            items: self.items.clone(),
            taskbar: info.rect,
            dpi,
            available: available_width(&info, margin),
            right_available: if edges {
                right_gap_width(&info, margin)
            } else {
                0
            },
            tray_left: info.tray_left,
            light: self.light,
        };
        let covered = !info.visible || foreground_is_fullscreen(info.monitor);
        let secs = self.started.elapsed().as_secs();
        let explicit = is_explicit(self.cfg.placement);
        // The same frame is already on screen: only keep the strip above the taskbar (#86).
        // Its placement still counts toward settling.
        if !covered
            && self.frame.is_current(&key)
            && let Some(placed) = self.placed_last
        {
            if horizontal {
                self.placement.observe(placed, explicit, secs);
            }
            if let Placed::Preferred(side) | Placed::Fallback(side) = placed {
                info.side = side;
            }
            strip::show(hwnd, self);
            self.taskbar = Some(info);
            return;
        }
        let metrics = self.renderer.measure(&self.items, dpi);
        let kinds: Vec<_> = self
            .items
            .iter()
            .map(|i| i.kind)
            .zip(metrics.iter().copied())
            .collect();
        let (gap, sep, ellipsis) = (
            to_px(GAP, dpi),
            to_px(SEP_W, dpi),
            self.renderer.ellipsis_px(dpi),
        );
        let left_layout = layout(&kinds, key.available, gap, sep, ellipsis);
        if super::placement::needs_second_pass(
            horizontal,
            edges,
            !self.items.is_empty(),
            left_layout.width > 0,
        ) {
            // The left side just ran out of room and the right side was not read: the edge
            // pass that follows decides, so the strip neither blinks nor counts as hidden.
            self.left_fitted_last = Some(false);
            return;
        }
        // The right side has room only with both edges read on a horizontal taskbar.
        let right_layout = (edges && info.tray_left.is_some() && key.right_available > 0)
            .then(|| layout(&kinds, key.right_available, gap, sep, ellipsis))
            .filter(|l| l.width > 0);
        let order = side_order(self.cfg.placement);
        let placed = if horizontal {
            place(order, |side| match side {
                Side::Left => left_layout.width > 0,
                Side::Right => right_layout.is_some(),
            })
        } else if left_layout.width > 0 {
            // Vertical taskbar: the left logic only, as before.
            Placed::Preferred(Side::Left)
        } else {
            Placed::Hidden
        };
        if !self.items.is_empty() {
            self.left_fitted_last = Some(left_layout.width > 0);
        }
        // Covered states hide as before and say nothing about the placement. An empty strip
        // (every item turned off) has nothing to place. A vertical taskbar never notifies.
        if !covered && !self.items.is_empty() && horizontal {
            self.placement.observe(placed, explicit, secs);
        }
        let drawn = match placed {
            Placed::Preferred(side) | Placed::Fallback(side) => Some(side),
            Placed::Hidden => None,
        };
        // Hidden: the preferred side, for the hints.
        info.side = drawn.unwrap_or(order[0]);
        // Right side: flush with the tray.
        let chosen = match (drawn, right_layout, info.tray_left) {
            (Some(Side::Left), _, _) => Some((info.rect.left + margin, left_layout.clone())),
            (Some(Side::Right), Some(l), Some(tray_left)) => {
                Some((right_strip_left(tray_left, margin, l.width), l))
            }
            _ => None,
        };
        let shown = match chosen.filter(|_| !covered) {
            None => {
                strip::hide(hwnd, self);
                None
            }
            Some((left, l)) => {
                let pos = (left, info.rect.top);
                let size = (l.width, info.rect.height());
                let painted = self.renderer.paint(
                    hwnd,
                    pos,
                    size,
                    dpi,
                    &self.items,
                    &metrics,
                    &l,
                    self.light,
                );
                match hides_strip(painted.is_err()) {
                    StripAction::Show => {
                        self.frame.painted(key);
                        self.placed_last = Some(placed);
                        strip::show(hwnd, self);
                    }
                    StripAction::Hide => strip::hide(hwnd, self),
                }
                Some(l)
            }
        };
        self.layout = Some(shown.unwrap_or(left_layout));
        self.taskbar = Some(info);
    }

    /// Forces the next render to paint, e.g. after the taskbar or the display changed.
    pub fn invalidate_frame(&mut self) {
        self.frame.invalidate();
    }

    /// The monitor the strip's taskbar is on, from the last taskbar query.
    pub fn strip_monitor(&self) -> Option<taskbar::Rect> {
        self.taskbar.as_ref().map(|t| t.monitor)
    }

    /// Hints input from the last taskbar query.
    pub fn taskbar_hints(&self) -> TaskbarHints {
        match &self.taskbar {
            Some(t) => TaskbarHints {
                widgets_button_on: t.widgets_button_on,
                windows_10: t.windows_10,
                side: t.side,
            },
            None => TaskbarHints::default(),
        }
    }

    pub fn tooltip(&self) -> String {
        let hidden: Vec<ItemView> = match &self.layout {
            Some(l) if l.overflow => (0..self.items.len())
                .filter(|i| !l.visible.contains(i))
                .map(|i| self.items[i].clone())
                .collect(),
            _ => Vec::new(),
        };
        let hints = self.taskbar_hints();
        let (claude, codex) = self.displayed_statuses();
        let text = tooltip_text(
            self.lang,
            &claude,
            &codex,
            &hidden,
            hints,
            now_unix(),
            self.cfg.show_remaining,
        );
        let mut text = text;
        for (kind, enabled, status, keepalive) in [
            (
                Kind::Claude,
                self.cfg.keep_claude_login,
                &claude,
                &self.keepalive_claude,
            ),
            (
                Kind::Codex,
                self.cfg.keep_codex_login,
                &codex,
                &self.keepalive_codex,
            ),
        ] {
            if !enabled || *status == ProviderStatus::Disabled {
                continue;
            }
            let (key, when) = match keepalive.outcome() {
                Outcome::Renewed { at } => (Key::KeepAliveRenewedFmt, at),
                Outcome::StillExpired { tried_at } => (Key::KeepAliveTriedFmt, tried_at),
                Outcome::None => continue,
            };
            let line = fmt(t(self.lang, key), &format_local_hhmm(when));
            text.push_str(&format!("\n\n{}: {line}", provider_name(kind)));
        }
        if text.is_empty() { "CCT".into() } else { text }
    }
}

pub(crate) fn claude_provider() -> ClaudeProvider {
    ClaudeProvider {
        creds_path: ClaudeProvider::default_path(),
    }
}

pub(crate) fn codex_provider() -> CodexProvider {
    CodexProvider::new(CodexProvider::default_home())
}

/// Starts the limits threads (one per provider); results queue in the returned inbox and
/// `WM_APP_LIMITS` (carrying no data) tells the window to drain it.
pub fn spawn_worker(hwnd: HWND, control: Arc<Control>) -> Receiver<LimitResult> {
    let claude = claude_provider();
    let codex = codex_provider();
    let fetchers: Vec<(Kind, Fetcher)> = vec![
        (
            Kind::Claude,
            Box::new(move |now| claude.fetch().map(|u| (u, false, now))),
        ),
        (
            Kind::Codex,
            Box::new(move |now| codex.fetch_with_fallback(now)),
        ),
    ];
    let target = hwnd.0 as isize;
    let (_handle, inbox) = spawn_queued(control, fetchers, move || {
        // SAFETY: posting a data-free notification to our own window; failure is harmless.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(target as _)),
                strip::WM_APP_LIMITS,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
    inbox
}

pub fn run() -> i32 {
    match App::new() {
        Ok(app) => strip::run(app),
        Err(_) => 1,
    }
}

#[derive(Debug, PartialEq)]
enum StripAction {
    Hide,
    Show,
}

fn hides_strip(paint_error: bool) -> StripAction {
    if paint_error {
        StripAction::Hide
    } else {
        StripAction::Show
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_hides_strip() {
        assert_eq!(hides_strip(true), StripAction::Hide);
    }

    #[test]
    fn success_shows_strip() {
        assert_eq!(hides_strip(false), StripAction::Show);
    }
}
