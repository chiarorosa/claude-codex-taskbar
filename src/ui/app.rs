//! Application state and wiring: config, sampler, limits worker and the strip window.

use super::layout::{Layout, layout};
use super::notifications::{Notice, Notifications};
use super::render::{GAP, Renderer, SEP_W, to_px};
use super::strip;
use super::taskbar::{
    self, IconCache, TaskbarInfo, available_width, hides_for_fullscreen, is_fullscreen,
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

/// Strip offset from the taskbar's left edge, in DIPs.
const MARGIN_DIP: f32 = 8.0;

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
    items: Vec<ItemView>,
    layout: Option<Layout>,
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
            items: Vec::new(),
            layout: None,
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
        self.cfg = cfg;
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

    /// Asks for a fresh UI Automation icon lookup (icons may have moved).
    pub fn invalidate_icons(&mut self) {
        self.icons.invalidate();
    }

    pub fn tick_icons(&mut self) {
        self.icons.tick();
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

    /// Recomputes position and layout from a fresh taskbar query, then paints or hides.
    /// The query runs outside the `App` borrow because UI Automation can pump messages.
    pub fn render(&mut self, hwnd: HWND, info: Option<TaskbarInfo>, looked_up_icons: bool) {
        let Some(mut info) = info else {
            strip::hide(hwnd, self);
            return;
        };
        if looked_up_icons {
            self.icons.on_lookup(info.icons_left);
        }
        info.icons_left = self.icons.value();
        let dpi = info.dpi;
        let (claude, codex) = self.displayed_statuses();
        self.items = build_items(
            &self.cfg,
            &self.hw,
            &claude,
            &codex,
            &self.claude_label,
            self.lang,
        );
        let metrics = self.renderer.measure(&self.items, dpi);
        let margin = to_px(MARGIN_DIP, dpi);
        let kinds: Vec<_> = self
            .items
            .iter()
            .map(|i| i.kind)
            .zip(metrics.iter().copied())
            .collect();
        let l = layout(
            &kinds,
            available_width(&info, margin),
            to_px(GAP, dpi),
            to_px(SEP_W, dpi),
            self.renderer.ellipsis_px(dpi),
        );
        let hide = !info.visible || l.width == 0 || foreground_is_fullscreen(info.monitor);
        if hide {
            strip::hide(hwnd, self);
        } else {
            let pos = (info.rect.left + margin, info.rect.top);
            let size = (l.width, info.rect.height());
            let painted =
                self.renderer
                    .paint(hwnd, pos, size, dpi, &self.items, &metrics, &l, self.light);
            let action = hides_strip(painted.is_err());
            match action {
                StripAction::Show => strip::show(hwnd, self),
                StripAction::Hide => strip::hide(hwnd, self),
            }
        }
        self.layout = Some(l);
        self.taskbar = Some(info);
    }

    /// The monitor the strip's taskbar is on, from the last taskbar query.
    pub fn strip_monitor(&self) -> Option<taskbar::Rect> {
        self.taskbar.as_ref().map(|t| t.monitor)
    }

    /// Hints input from the last taskbar query.
    pub fn taskbar_hints(&self) -> TaskbarHints {
        match &self.taskbar {
            Some(t) => TaskbarHints {
                left_aligned: t.left_aligned,
                widgets_button_on: t.widgets_button_on,
                windows_10: t.windows_10,
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
        let text = tooltip_text(self.lang, &claude, &codex, &hidden, hints, now_unix());
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
    CodexProvider {
        home: CodexProvider::default_home(),
    }
}

/// Starts the limits thread; results queue in the returned inbox and `WM_APP_LIMITS`
/// (carrying no data) tells the window to drain it.
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
