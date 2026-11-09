//! User configuration persisted as JSON under %APPDATA%.

use crate::hw::parse::disk_parts;
use crate::model::ItemId;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "claude-codex-taskbar";
/// Folder used by v0.1.x, migrated once at startup.
pub const LEGACY_APP_DIR: &str = "widget-usage-windows";
const DEFAULT_NOTIFY_THRESHOLDS: [u32; 2] = [80, 95];

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Config {
    pub hw_interval_secs: u32,
    pub limits_interval_secs: u32,
    pub claude_enabled: bool,
    pub codex_enabled: bool,
    /// Keys from [`item_key`] of items the user turned off; normalized by [`Self::clamped`].
    pub hidden_items: Vec<String>,
    pub first_run_done: bool,
    /// Opt-in: run `claude auth status` when the Claude login has expired.
    pub keep_claude_login: bool,
    /// The user was already asked about `keep_claude_login`.
    pub keepalive_asked: bool,
    /// Opt-in: run `codex login status` when the Codex login has expired.
    pub keep_codex_login: bool,
    pub codex_keepalive_asked: bool,
    /// Opt-in notification delivery; the UI is a separate slice of #15.
    pub notify_near_limit: bool,
    /// Percent-used thresholds, normalized to unique ascending values in 1..=100.
    pub notify_thresholds: Vec<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hw_interval_secs: 1,
            limits_interval_secs: 120,
            claude_enabled: false,
            codex_enabled: false,
            hidden_items: Vec::new(),
            first_run_done: false,
            keep_claude_login: false,
            keepalive_asked: false,
            keep_codex_login: false,
            codex_keepalive_asked: false,
            notify_near_limit: false,
            notify_thresholds: DEFAULT_NOTIFY_THRESHOLDS.to_vec(),
        }
    }
}

impl Config {
    /// Missing or corrupt files yield the default config; this never fails.
    pub fn load_from(path: &Path) -> Config {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| {
                serde_json::from_str::<Config>(text.trim_start_matches('\u{feff}')).ok()
            })
            .unwrap_or_default()
            .clamped()
    }

    /// Writes atomically: temp file first, then rename over the target.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    pub fn clamped(mut self) -> Config {
        self.hw_interval_secs = self.hw_interval_secs.clamp(1, 5);
        self.limits_interval_secs = self.limits_interval_secs.clamp(60, 3600);
        clamp_notify_thresholds(&mut self.notify_thresholds);
        // Normalize v0.1/v0.2 disk keys once, including disks not currently connected.
        for key in &mut self.hidden_items {
            if let Some(instance) = key.strip_prefix("disk:")
                && disk_parts(instance).0.parse::<u32>().is_ok()
            {
                *key = disk_key(instance);
            }
        }
        let mut seen = HashSet::new();
        self.hidden_items.retain(|key| seen.insert(key.clone()));
        self
    }

    pub fn is_visible(&self, id: &ItemId) -> bool {
        let key = item_key(id);
        !self.hidden_items.contains(&key)
    }

    pub fn set_visible(&mut self, id: &ItemId, visible: bool) {
        let key = item_key(id);
        self.hidden_items.retain(|k| *k != key);
        if !visible {
            self.hidden_items.push(key);
        }
    }
}

fn clamp_notify_thresholds(thresholds: &mut Vec<u32>) {
    if thresholds.is_empty() {
        thresholds.extend(DEFAULT_NOTIFY_THRESHOLDS);
    }
    for threshold in thresholds.iter_mut() {
        *threshold = (*threshold).clamp(1, 100);
    }
    thresholds.sort_unstable();
    thresholds.dedup();
}

/// Disks with drive letters keep their identity when PDH disk indices change.
fn disk_key(instance: &str) -> String {
    let (index, letters) = disk_parts(instance);
    if letters.is_empty() {
        format!("disk:#{index}")
    } else {
        format!("disk:{letters}")
    }
}

pub fn item_key(id: &ItemId) -> String {
    match id {
        ItemId::Cpu => "cpu".into(),
        ItemId::Ram => "ram".into(),
        ItemId::Gpu => "gpu".into(),
        ItemId::Disk(instance) => disk_key(instance),
        ItemId::ClaudeFive => "claude5h".into(),
        ItemId::ClaudeWeek => "claudeweek".into(),
        ItemId::CodexFive => "codex5h".into(),
        ItemId::CodexWeek => "codexweek".into(),
    }
}

/// `%APPDATA%\claude-codex-taskbar\config.json`.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(APP_DIR).join("config.json")
}

#[cfg(test)]
mod notify_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wuw-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn config_roundtrip() {
        let path = temp_file("roundtrip");
        let cfg = Config {
            hw_interval_secs: 2,
            limits_interval_secs: 300,
            claude_enabled: true,
            codex_enabled: false,
            hidden_items: vec!["gpu".into()],
            first_run_done: true,
            keep_claude_login: true,
            keepalive_asked: true,
            keep_codex_login: true,
            codex_keepalive_asked: true,
            notify_near_limit: true,
            notify_thresholds: vec![75, 90],
        };
        cfg.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), cfg);
    }

    #[test]
    fn config_corrupt_file_gives_default() {
        let path = temp_file("corrupt");
        std::fs::write(&path, "{nope").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn config_missing_file_gives_default() {
        let path = temp_file("missing").with_file_name("absent.json");
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn config_missing_fields_use_defaults() {
        let path = temp_file("partial");
        std::fs::write(&path, r#"{"claude_enabled":true}"#).unwrap();
        let cfg = Config::load_from(&path);
        assert!(cfg.claude_enabled);
        assert_eq!(cfg.hw_interval_secs, 1);
        assert_eq!(cfg.limits_interval_secs, 120);
        assert!(!cfg.first_run_done);
        assert!(!cfg.keep_claude_login, "keep-alive is opt-in");
        assert!(!cfg.keepalive_asked);
        assert!(!cfg.keep_codex_login, "keep-alive is opt-in");
        assert!(!cfg.codex_keepalive_asked);
    }

    #[test]
    fn load_migrates_and_deduplicates_legacy_disk_keys_before_any_lookup() {
        let path = temp_file("migrate-disk-keys");
        std::fs::write(
            &path,
            r#"{"hidden_items":["disk:0 C:","gpu","disk:C:","disk:1 C:","disk:2","disk:#2","disk:7 X: Y:"]}"#,
        )
        .unwrap();
        let cfg = Config::load_from(&path);
        assert_eq!(
            cfg.hidden_items,
            ["disk:C:", "gpu", "disk:#2", "disk:X: Y:"]
        );

        // No disk enumeration or visibility toggle is needed, even for unplugged disks.
        cfg.save_to(&path).unwrap();
        let saved: Config = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.hidden_items, cfg.hidden_items);
        assert_eq!(Config::load_from(&path), cfg);
    }

    #[test]
    fn load_preserves_modern_and_unrecognized_keys() {
        let path = temp_file("modern-disk-keys");
        std::fs::write(
            &path,
            r#"{"hidden_items":["disk:C:","disk:C: D:","disk:#2","ram","disk:","disk:unknown","future-item"]}"#,
        )
        .unwrap();
        let cfg = Config::load_from(&path);
        assert_eq!(
            cfg.hidden_items,
            [
                "disk:C:",
                "disk:C: D:",
                "disk:#2",
                "ram",
                "disk:",
                "disk:unknown",
                "future-item"
            ]
        );
        assert_eq!(cfg.clone().clamped(), cfg);
    }

    #[test]
    fn config_clamps_intervals() {
        let c = |hw, lim| {
            Config {
                hw_interval_secs: hw,
                limits_interval_secs: lim,
                ..Config::default()
            }
            .clamped()
        };
        assert_eq!(c(0, 120).hw_interval_secs, 1);
        assert_eq!(c(9, 120).hw_interval_secs, 5);
        assert_eq!(c(1, 10).limits_interval_secs, 60);
        assert_eq!(c(3, 600).limits_interval_secs, 600);
        assert_eq!(c(3, 99_999).limits_interval_secs, 3600);
    }

    #[test]
    fn visibility_respects_hidden_items() {
        let cfg = Config {
            hidden_items: vec!["disk:D:".into()],
            ..Config::default()
        };
        assert!(!cfg.is_visible(&ItemId::Disk("1 D:".into())));
        assert!(cfg.is_visible(&ItemId::Disk("0 C:".into())));
        assert!(cfg.is_visible(&ItemId::Cpu));
    }

    #[test]
    fn disk_key_uses_letters() {
        for instance in ["0 C:", "1 C:"] {
            assert_eq!(item_key(&ItemId::Disk(instance.into())), "disk:C:");
        }
        assert_eq!(item_key(&ItemId::Disk("3 C: D:".into())), "disk:C: D:");
    }

    #[test]
    fn disk_key_without_letters_uses_index() {
        assert_eq!(item_key(&ItemId::Disk("2".into())), "disk:#2");
    }

    #[test]
    fn legacy_disk_key_still_matches() {
        let path = temp_file("legacy-disk-matches");
        std::fs::write(
            &path,
            r#"{"hidden_items":["disk:0 C:","disk:2 D: E:","disk:3"]}"#,
        )
        .unwrap();
        let cfg = Config::load_from(&path);
        assert!(!cfg.is_visible(&ItemId::Disk("1 C:".into())));
        assert!(!cfg.is_visible(&ItemId::Disk("4 D: E:".into())));
        assert!(!cfg.is_visible(&ItemId::Disk("3".into())));
        assert!(cfg.is_visible(&ItemId::Disk("0 F:".into())));
        assert!(cfg.is_visible(&ItemId::Disk("2 D:".into())));
        assert!(cfg.is_visible(&ItemId::Disk("4".into())));
    }

    #[test]
    fn hidden_disk_stays_hidden_after_reenumeration() {
        let mut cfg = Config::default();
        cfg.set_visible(&ItemId::Disk("0 C:".into()), false);
        assert!(!cfg.is_visible(&ItemId::Disk("1 C:".into())));
        assert!(cfg.is_visible(&ItemId::Disk("0 D:".into())));
        cfg.set_visible(&ItemId::Disk("2".into()), false);
        assert!(!cfg.is_visible(&ItemId::Disk("2".into())));
        assert!(cfg.is_visible(&ItemId::Disk("3".into())));
    }

    #[test]
    fn showing_legacy_disk_removes_all_matching_keys() {
        let mut cfg = Config {
            hidden_items: vec![
                "disk:0 C:".into(),
                "disk:1 C:".into(),
                "disk:C:".into(),
                "gpu".into(),
                "disk:2 D:".into(),
            ],
            ..Config::default()
        }
        .clamped();
        cfg.set_visible(&ItemId::Disk("3 C:".into()), true);
        assert!(cfg.is_visible(&ItemId::Disk("3 C:".into())));
        assert_eq!(cfg.hidden_items, ["gpu", "disk:D:"]);
    }

    #[test]
    fn hiding_legacy_disk_saves_one_stable_key() {
        let path = temp_file("legacy-disk");
        std::fs::write(&path, r#"{"hidden_items":["disk:0 C:","disk:C:","ram"]}"#).unwrap();
        let mut cfg = Config::load_from(&path);
        cfg.set_visible(&ItemId::Disk("1 C:".into()), false);
        cfg.set_visible(&ItemId::Disk("2 C:".into()), false);
        assert_eq!(cfg.hidden_items, ["ram", "disk:C:"]);
        cfg.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        assert!(!loaded.is_visible(&ItemId::Disk("3 C:".into())));
        assert!(!loaded.is_visible(&ItemId::Ram));
        assert!(loaded.is_visible(&ItemId::Disk("0 D:".into())));
    }

    #[test]
    fn item_keys_are_stable() {
        assert_eq!(item_key(&ItemId::Cpu), "cpu");
        assert_eq!(item_key(&ItemId::Ram), "ram");
        assert_eq!(item_key(&ItemId::Gpu), "gpu");
        assert_eq!(item_key(&ItemId::Disk("0 C:".into())), "disk:C:");
        assert_eq!(item_key(&ItemId::ClaudeFive), "claude5h");
        assert_eq!(item_key(&ItemId::ClaudeWeek), "claudeweek");
        assert_eq!(item_key(&ItemId::CodexFive), "codex5h");
        assert_eq!(item_key(&ItemId::CodexWeek), "codexweek");
    }
}
