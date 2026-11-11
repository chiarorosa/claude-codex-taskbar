//! "Start with Windows": the per-user Run key for the portable `.exe`, or the package's
//! startup task when CCT runs from its MSIX package (#5), where a Run entry is ignored.

use std::io;
use std::path::{Path, PathBuf};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "claude-codex-taskbar";
/// Run value written by v0.1.x.
const LEGACY_VALUE_NAME: &str = "widget-usage-windows";

pub fn autostart_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

pub fn value_matches(value: &str, exe: &Path) -> bool {
    value.eq_ignore_ascii_case(&autostart_value(exe))
}

fn current_exe() -> io::Result<PathBuf> {
    std::env::current_exe()
}

fn read_value_named(name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buf/size describe a valid buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn is_enabled() -> bool {
    if crate::packaged::is_packaged() {
        crate::packaged::startup_task_enabled()
    } else {
        run_key_enabled()
    }
}

pub fn enable() -> io::Result<()> {
    if crate::packaged::is_packaged() {
        crate::packaged::enable_startup_task()
    } else {
        enable_run_key()
    }
}

pub fn disable() -> io::Result<()> {
    if crate::packaged::is_packaged() {
        crate::packaged::disable_startup_task()
    } else {
        delete_value(VALUE_NAME)
    }
}

/// True only when the Run entry points at this executable.
fn run_key_enabled() -> bool {
    match (read_value_named(VALUE_NAME), current_exe()) {
        (Some(value), Ok(exe)) => value_matches(&value, &exe),
        _ => false,
    }
}

fn enable_run_key() -> io::Result<()> {
    let value: Vec<u16> = autostart_value(&current_exe()?)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: value is a NUL-terminated UTF-16 buffer of the given byte length.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(VALUE_NAME),
            REG_SZ.0,
            Some(value.as_ptr().cast()),
            (value.len() * 2) as u32,
        )
    }
    .ok()
    .map_err(io::Error::other)
}

fn delete_value(name: &str) -> io::Result<()> {
    // SAFETY: plain registry call.
    unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(name),
        )
    }
    .ok()
    .map_err(io::Error::other)
}

/// If v0.1.x registered "Start with Windows", move that entry to the new name and exe.
/// Portable build only: inside the package the delete would never stick, and the startup
/// task would be turned on again at every launch.
pub fn migrate_legacy() {
    if !crate::packaged::is_packaged()
        && read_value_named(LEGACY_VALUE_NAME).is_some()
        && delete_value(LEGACY_VALUE_NAME).is_ok()
    {
        let _ = enable_run_key();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn autostart_value_quotes_path() {
        assert_eq!(
            autostart_value(Path::new(r"C:\a b\x.exe")),
            r#""C:\a b\x.exe""#
        );
    }

    #[test]
    fn value_matches_is_case_insensitive() {
        let exe = Path::new(r"C:\Apps\Widget.exe");
        assert!(value_matches(r#""c:\apps\widget.exe""#, exe));
        assert!(!value_matches(r#""C:\Other\widget.exe""#, exe));
    }
}
