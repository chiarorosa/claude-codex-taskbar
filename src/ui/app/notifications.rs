//! Classify each worker result before asking the pure notification policy for events.

use super::App;
use crate::limits::ProviderStatus;
use crate::limits::worker::LimitResult;
use crate::model::Kind;
use crate::timeutil::now_unix;
use crate::ui::notifications::{Notice, classify_limit_result};

impl App {
    pub fn on_limits(&mut self, r: LimitResult) -> Vec<Notice> {
        if r.kind == Kind::Claude {
            // A /login outside CCT changes the live account; catch it with each refresh.
            self.refresh_claude_label();
        }
        let now = now_unix();
        let status = match r.kind {
            Kind::Claude => &mut self.claude,
            Kind::Codex => &mut self.codex,
            Kind::Hardware => return Vec::new(),
        };
        if *status == ProviderStatus::Disabled {
            return Vec::new();
        }
        *status = classify_limit_result(status, r.result, now);
        let notices = self.notifications.observe(&self.cfg, r.kind, status, now);
        self.keep_login_active(r.kind, now);
        notices
    }
}
