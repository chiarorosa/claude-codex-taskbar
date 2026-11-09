//! Optional quota alerts, independent of the native notification transport.

pub mod shell;

use crate::config::Config;
use crate::i18n::{Key, Lang, t};
use crate::limits::notify::{CrossingState, Event, LimitWindow, crossings};
use crate::limits::state::{STALE_SECS, age_status, next_status};
use crate::limits::{FetchError, ProviderStatus, ProviderUsage};
use crate::model::{Kind, provider_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: Kind,
    pub event: Event,
}

/// Keeps successful session-log fallback separate from fresh endpoint/CLI readings.
/// For example, Ok((usage, true, at)) remains FromSession and cannot notify.
pub fn classify_limit_result(
    previous: &ProviderStatus,
    result: Result<(ProviderUsage, bool, i64), FetchError>,
    now: i64,
) -> ProviderStatus {
    match result {
        Ok((usage, true, at)) => ProviderStatus::FromSession { usage, at },
        Ok((usage, false, at)) => next_status(previous, Ok(usage), at),
        Err(err) => next_status(previous, Err(err), now),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notifications {
    claude: CrossingState,
    codex: CrossingState,
}

impl Notifications {
    /// Observes every result, even with alerts off, so enabling cannot replay old crossings.
    /// For example, a first 70% reading followed by 97% returns the 80/95 events.
    pub fn observe(
        &mut self,
        cfg: &Config,
        kind: Kind,
        status: &ProviderStatus,
        now: i64,
    ) -> Vec<Notice> {
        let history = match kind {
            Kind::Claude => &mut self.claude,
            Kind::Codex => &mut self.codex,
            Kind::Hardware => return Vec::new(),
        };
        let max_age = STALE_SECS.max(2 * i64::from(cfg.limits_interval_secs));
        let current = age_status(status, now, max_age);
        let (next, events) = crossings(history, &current, &cfg.notify_thresholds);
        *history = next;
        if !cfg.notify_near_limit || !provider_enabled(cfg, kind) {
            return Vec::new();
        }
        events
            .into_iter()
            .map(|event| Notice { kind, event })
            .collect()
    }
}

fn provider_enabled(cfg: &Config, kind: Kind) -> bool {
    match kind {
        Kind::Claude => cfg.claude_enabled,
        Kind::Codex => cfg.codex_enabled,
        Kind::Hardware => false,
    }
}

/// Groups a batch into the highest crossed threshold per window; e.g. 80/95 becomes 95.
pub fn balloon_text(lang: Lang, cfg: &Config, notices: &[Notice]) -> Option<String> {
    if !cfg.notify_near_limit {
        return None;
    }
    let mut lines = Vec::new();
    for kind in [Kind::Claude, Kind::Codex]
        .into_iter()
        .filter(|k| provider_enabled(cfg, *k))
    {
        let mut windows = Vec::new();
        for (window, label) in [
            (LimitWindow::FiveHour, Key::NotifyFiveHour),
            (LimitWindow::Weekly, Key::SettingsWeekSuffix),
        ] {
            let highest = notices
                .iter()
                .filter(|n| n.kind == kind && n.event.window == window)
                .map(|n| n.event.threshold)
                .max();
            if let Some(threshold) = highest {
                windows.push(format!("{} {threshold}%", t(lang, label)));
            }
        }
        if !windows.is_empty() {
            lines.push(format!("{}: {}", provider_name(kind), windows.join(" · ")));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BalloonCommand {
    Add,
    Version,
    Show,
    ArmCleanup,
}

/// Thin application-owned boundary; native calls must run outside App/RefCell borrows.
pub trait NotificationSink {
    fn accepts(&self) -> bool;
    fn send(&self, command: BalloonCommand, id: u16, title: &str, body: &str) -> bool;
    fn remove(&self, id: u16);
}

/// Sends one transient balloon; a revoked owner or failed operation removes its icon.
/// For example, opt-out during Add prevents Version and Show from running.
pub fn send_balloon(
    sink: &impl NotificationSink,
    id: u16,
    title: &str,
    body: &str,
    current: impl Fn() -> bool,
) -> bool {
    for command in [
        BalloonCommand::Add,
        BalloonCommand::Version,
        BalloonCommand::Show,
        BalloonCommand::ArmCleanup,
    ] {
        // A shell query or send can pump messages and revoke the user's opt-in.
        if !sink.accepts() || !current() || !sink.send(command, id, title, body) {
            sink.remove(id);
            return false;
        }
    }
    if !sink.accepts() || !current() {
        sink.remove(id);
        return false;
    }
    true
}

#[derive(Default)]
pub struct BalloonLife {
    last: u16,
    active: Option<u16>,
    shown: bool,
}

impl BalloonLife {
    /// Current temporary icon, e.g. None after cancellation or a terminal callback.
    pub fn active_id(&self) -> Option<u16> {
        self.active
    }

    pub fn begin(&mut self) -> (Option<u16>, u16) {
        self.last = self.last.wrapping_add(1).max(1);
        self.shown = false;
        (self.active.replace(self.last), self.last)
    }

    /// Records the shell's SHOW confirmation only for the active icon.
    pub fn mark_shown(&mut self, id: u16) -> bool {
        if !self.is_current(id) {
            return false;
        }
        self.shown = true;
        true
    }

    /// Distinguishes an actual balloon from one discarded by the shell.
    pub fn is_shown(&self, id: u16) -> bool {
        self.is_current(id) && self.shown
    }

    pub fn is_current(&self, id: u16) -> bool {
        self.active == Some(id)
    }

    pub fn finish(&mut self, id: u16) -> bool {
        if !self.is_current(id) {
            return false;
        }
        self.active = None;
        true
    }

    pub fn cancel(&mut self) -> Option<u16> {
        self.active.take()
    }
}

/// Copies complete Unicode scalars with room for NUL, e.g. "a💡" fits in four units.
pub fn wide_array<const N: usize>(text: &str) -> [u16; N] {
    let mut out = [0; N];
    let mut pos = 0;
    for ch in text.chars() {
        let mut pair = [0; 2];
        let units = ch.encode_utf16(&mut pair);
        if pos + units.len() > N.saturating_sub(1) {
            break;
        }
        out[pos..pos + units.len()].copy_from_slice(units);
        pos += units.len();
    }
    out
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;
