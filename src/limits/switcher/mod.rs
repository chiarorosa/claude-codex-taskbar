//! Switch Claude Code between saved accounts (#53).
//!
//! This is the only module allowed to write Claude Code's credentials, and only on an explicit
//! user action. See AGENTS.md and issue #53 for the rules.

pub mod lock;
