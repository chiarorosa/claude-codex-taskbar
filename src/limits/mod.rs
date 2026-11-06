//! Subscription limit types shared by the Claude and Codex providers.

pub mod claude;
pub mod codex;
pub mod http;
pub mod keepalive;
pub mod notify;
pub mod reset;
pub mod state;
pub mod switcher;
pub mod terminal;
pub mod worker;

/// Usage of one limit window (5-hour or weekly).
#[derive(Clone, Debug, PartialEq)]
pub struct WindowUsage {
    /// Percent used, already clamped to 0–100.
    pub used_pct: f32,
    /// Unix seconds when the window resets, if known.
    pub resets_at: Option<i64>,
}

/// Claude models with independently reported weekly limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Opus,
    Sonnet,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderUsage {
    pub five_hour: Option<WindowUsage>,
    pub weekly: Option<WindowUsage>,
    /// Only model windows the provider actually reported, in display order.
    pub weekly_by_model: Vec<(ModelKind, WindowUsage)>,
    /// True only when the provider explicitly enables extra usage.
    pub extra_usage_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FetchError {
    NoLogin,
    Expired,
    NoSubscription,
    Network(String),
    BadResponse(String),
}

/// What the strip shows for one provider.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderStatus {
    Disabled,
    Loading,
    Ok {
        usage: ProviderUsage,
        at: i64,
    },
    /// Latest fetch failed; showing data fetched at `at` (less than 15 min old).
    Stale {
        usage: ProviderUsage,
        at: i64,
        err: FetchError,
    },
    /// Data read from the CLI's own session log written at `at`.
    FromSession {
        usage: ProviderUsage,
        at: i64,
    },
    Failed(FetchError),
}

pub trait Provider: Send {
    fn fetch(&self) -> Result<ProviderUsage, FetchError>;
    fn has_login(&self) -> bool;
}
