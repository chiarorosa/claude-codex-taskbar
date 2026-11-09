//! Pure threshold decisions for issue #15; notification delivery belongs to the UI.

use super::{ProviderStatus, WindowUsage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitWindow {
    FiveHour,
    Weekly,
}

/// A crossed percentage; the caller supplies the provider and notification text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub window: LimitWindow,
    pub threshold: u32,
}

/// One provider's high-water percentages, retained across failed or missing readings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossingState {
    five_hour: Option<WindowUsage>,
    weekly: Option<WindowUsage>,
}

/// Returns new history and crossings from an `Ok` reading, without changing inputs.
/// Keep the returned history per provider and pass normalized `Config::notify_thresholds`.
/// The first reading seeds a baseline; e.g. a later 70% -> 97% emits 80 and 95 once.
pub fn crossings(
    prev: &CrossingState,
    curr: &ProviderStatus,
    thresholds: &[u32],
) -> (CrossingState, Vec<Event>) {
    let mut next = prev.clone();
    let mut events = Vec::new();
    let ProviderStatus::Ok { usage, .. } = curr else {
        return (next, events);
    };
    for (window, history, reading) in [
        (LimitWindow::FiveHour, &mut next.five_hour, &usage.five_hour),
        (LimitWindow::Weekly, &mut next.weekly, &usage.weekly),
    ] {
        window_crossings(history, reading, window, thresholds, &mut events);
    }
    (next, events)
}

fn window_crossings(
    history: &mut Option<WindowUsage>,
    reading: &Option<WindowUsage>,
    window: LimitWindow,
    thresholds: &[u32],
    events: &mut Vec<Event>,
) {
    let Some((before, after)) = advance_window(history, reading) else {
        return;
    };
    events.extend(
        thresholds
            .iter()
            .copied()
            .filter(|&t| before < t as f32 && after >= t as f32)
            .map(|threshold| Event { window, threshold }),
    );
}

fn advance_window(
    history: &mut Option<WindowUsage>,
    reading: &Option<WindowUsage>,
) -> Option<(f32, f32)> {
    let curr = reading.as_ref().filter(|w| w.used_pct.is_finite())?;
    let Some(prev) = history else {
        *history = Some(curr.clone());
        return None;
    };
    // Missing reset metadata cannot prove that a quota window rolled over (#15).
    // Providers' estimated/rounded timestamps can drift between polls (PR #73).
    let reset =
        matches!((prev.resets_at, curr.resets_at), (Some(a), Some(b)) if b.saturating_sub(a) > 60);
    let before = if reset { 0.0 } else { prev.used_pct };
    *prev = WindowUsage {
        // Keep the peak so dips cannot rearm a threshold in the same window.
        used_pct: before.max(curr.used_pct),
        resets_at: curr.resets_at.or(prev.resets_at),
    };
    Some((before, curr.used_pct))
}

#[cfg(test)]
mod tests;
