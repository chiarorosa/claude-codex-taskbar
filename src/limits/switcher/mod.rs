//! Switch Claude Code between saved accounts (#53).
//!
//! This is the only module allowed to write Claude Code's credentials, and only on an explicit
//! user action. See AGENTS.md and issue #53 for the rules.

pub mod fsio;
pub mod lock;
pub mod sessions;
pub mod store;
pub mod switch;
pub mod vault;

/// Credential text. `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Self {
        Secret(text)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn secret_debug_redacts() {
        let s = super::Secret::new("sk-ant-oat01-FAKE".into());
        assert!(!format!("{s:?}").contains("FAKE"));
    }
}
