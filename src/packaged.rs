//! Running from the MSIX package (#5): detection, and the package's startup task, which
//! replaces the Run key there ("Start with Windows").

use std::io;
use std::sync::OnceLock;
use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::core::HSTRING;

/// `TaskId` of the `windows.startupTask` extension in `packaging/AppxManifest.xml`.
pub const STARTUP_TASK_ID: &str = "CCTStartup";

/// Whether this process has a package identity, i.e. was started from the MSIX package.
pub fn is_packaged() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut len = 0u32;
        // SAFETY: with a zero length and no buffer, the call only reports the needed length.
        has_identity(unsafe { GetCurrentPackageFullName(&mut len, None) })
    })
}

/// A process with a package gets "buffer too small" for a zero-length query; one without
/// gets `APPMODEL_ERROR_NO_PACKAGE`.
fn has_identity(status: WIN32_ERROR) -> bool {
    status == ERROR_INSUFFICIENT_BUFFER || status == ERROR_SUCCESS
}

/// On is on, whether the user or a policy turned it on.
fn is_on(state: StartupTaskState) -> bool {
    state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
}

/// Runs a WinRT call on its own thread, so the UI thread's apartment never waits on it.
fn on_worker<T: Send + 'static>(
    call: impl FnOnce() -> windows::core::Result<T> + Send + 'static,
) -> io::Result<T> {
    std::thread::spawn(call)
        .join()
        .map_err(|_| io::Error::other("startup task call panicked"))?
        .map_err(io::Error::other)
}

fn task() -> windows::core::Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))?.join()
}

pub fn startup_task_enabled() -> bool {
    on_worker(|| task()?.State()).is_ok_and(is_on)
}

/// Turns the startup task on. When the user turned it off in Task Manager or in *Settings →
/// Apps → Startup*, only they can turn it back on there, so this reports an error.
pub fn enable_startup_task() -> io::Result<()> {
    let state = on_worker(|| task()?.RequestEnableAsync()?.join())?;
    if is_on(state) {
        Ok(())
    } else {
        Err(io::Error::other(
            "turned off in Windows startup settings; turn it on there",
        ))
    }
}

pub fn disable_startup_task() -> io::Result<()> {
    on_worker(|| task()?.Disable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;

    #[test]
    fn identity_is_read_from_the_package_name_query() {
        assert!(has_identity(ERROR_INSUFFICIENT_BUFFER));
        assert!(has_identity(ERROR_SUCCESS));
        assert!(!has_identity(APPMODEL_ERROR_NO_PACKAGE));
    }

    #[test]
    fn startup_task_is_on_when_the_user_or_a_policy_enabled_it() {
        assert!(is_on(StartupTaskState::Enabled));
        assert!(is_on(StartupTaskState::EnabledByPolicy));
        for off in [
            StartupTaskState::Disabled,
            StartupTaskState::DisabledByUser,
            StartupTaskState::DisabledByPolicy,
        ] {
            assert!(!is_on(off), "{off:?}");
        }
    }

    #[test]
    fn the_manifest_declares_the_startup_task_this_code_asks_for() {
        let manifest = include_str!("../packaging/AppxManifest.xml");
        assert!(manifest.contains(&format!(r#"TaskId="{STARTUP_TASK_ID}""#)));
        // Off until the user turns it on, like the Run key of the portable build.
        assert!(manifest.contains(r#"Enabled="false""#));
        assert_eq!(
            manifest
                .matches(r#"Executable="claude-codex-taskbar.exe""#)
                .count(),
            2,
            "the app and its startup task start the same exe"
        );
        assert!(manifest.contains(r#"<rescap:Capability Name="runFullTrust" />"#));
    }

    #[test]
    fn tests_and_the_portable_exe_run_without_a_package() {
        assert!(!is_packaged());
    }
}
