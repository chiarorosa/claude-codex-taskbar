//! Item identifiers and visual helpers shared by the UI and config.

/// One cell of the strip. `Disk` carries the PDH instance name (e.g. `"0 C:"`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemId {
    Cpu,
    Ram,
    Gpu,
    Disk(String),
    ClaudeFive,
    ClaudeWeek,
    CodexFive,
    CodexWeek,
}

/// Which group an item belongs to; drives colors and the separator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Hardware,
    Claude,
    Codex,
}

/// Provider display name, e.g. Claude becomes "Claude Code"; hardware has no subscription.
pub(crate) fn provider_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "Claude Code",
        Kind::Codex => "Codex",
        Kind::Hardware => "",
    }
}

pub const COLOR_HARDWARE: u32 = 0x4cc2ff;
pub const COLOR_CLAUDE: u32 = 0xd97757;
pub const COLOR_CODEX: u32 = 0x3b82f6;
pub const COLOR_WARN: u32 = 0xe8a33d;
pub const COLOR_CRITICAL: u32 = 0xe5484d;
pub const COLOR_ERROR: u32 = 0x5a5a5a;

/// Bar fill color as 0xRRGGBB; `None` means no value (error state).
pub fn bar_color(kind: Kind, pct: Option<f32>) -> u32 {
    match pct {
        None => COLOR_ERROR,
        Some(p) if p >= 90.0 => COLOR_CRITICAL,
        Some(p) if p >= 75.0 => COLOR_WARN,
        Some(_) => match kind {
            Kind::Hardware => COLOR_HARDWARE,
            Kind::Claude => COLOR_CLAUDE,
            Kind::Codex => COLOR_CODEX,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_names_are_shared_by_tooltips_and_notifications() {
        assert_eq!(provider_name(Kind::Claude), "Claude Code");
        assert_eq!(provider_name(Kind::Codex), "Codex");
        assert_eq!(provider_name(Kind::Hardware), "");
    }

    #[test]
    fn bar_colors_follow_thresholds() {
        assert_eq!(bar_color(Kind::Hardware, Some(10.0)), 0x4cc2ff);
        assert_eq!(bar_color(Kind::Claude, Some(74.9)), 0xd97757);
        assert_eq!(bar_color(Kind::Codex, Some(50.0)), 0x3b82f6, "Codex blue");
        assert_eq!(bar_color(Kind::Codex, Some(75.0)), 0xe8a33d);
        assert_eq!(bar_color(Kind::Claude, Some(90.0)), 0xe5484d);
        assert_eq!(bar_color(Kind::Hardware, Some(100.0)), 0xe5484d);
        assert_eq!(bar_color(Kind::Codex, None), 0x5a5a5a);
    }
}
