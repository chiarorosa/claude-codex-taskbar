//! Opt-in "keep login active": when a provider's token has expired, run the official CLI's
//! status command (`claude auth status`, `codex login status`; neither sends a prompt) so
//! the CLI itself renews its own login. This app still never writes credentials files.

use super::{FetchError, ProviderStatus};
use crate::model::Kind;
use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

/// Minimum spacing between CLI runs.
pub const MIN_INTERVAL_SECS: i64 = 15 * 60;
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    StillExpired { tried_at: i64 },
    Renewed { at: i64 },
}

#[derive(Debug)]
pub struct KeepAlive {
    last_attempt: Option<i64>,
    outcome: Outcome,
}

impl Default for KeepAlive {
    fn default() -> Self {
        KeepAlive {
            last_attempt: None,
            outcome: Outcome::None,
        }
    }
}

impl KeepAlive {
    pub fn should_run(&self, enabled: bool, status: &ProviderStatus, now: i64) -> bool {
        enabled
            && *status == ProviderStatus::Failed(FetchError::Expired)
            && self
                .last_attempt
                .is_none_or(|t| now - t >= MIN_INTERVAL_SECS)
    }

    pub fn started(&mut self, now: i64) {
        self.last_attempt = Some(now);
    }

    /// Tracks whether the login recovered after the last CLI run.
    pub fn observe(&mut self, status: &ProviderStatus, now: i64) {
        let Some(tried_at) = self.last_attempt else {
            return;
        };
        match status {
            ProviderStatus::Ok { .. } if !matches!(self.outcome, Outcome::Renewed { .. }) => {
                self.outcome = Outcome::Renewed { at: now };
            }
            ProviderStatus::Failed(FetchError::Expired) => {
                self.outcome = Outcome::StillExpired { tried_at };
            }
            _ => {}
        }
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
}

/// The CLI command that checks (and, if needed, renews) a provider's login.
pub fn status_command(kind: Kind) -> Option<&'static [&'static str]> {
    match kind {
        Kind::Claude => Some(&["claude", "auth", "status"]),
        Kind::Codex => Some(&["codex", "login", "status"]),
        Kind::Hardware => None,
    }
}

/// Providers to offer keep-alive for: logged in and not asked before.
pub fn invitation_kinds(
    claude_asked: bool,
    codex_asked: bool,
    claude_login: bool,
    codex_login: bool,
) -> Vec<Kind> {
    [
        (Kind::Claude, !claude_asked && claude_login),
        (Kind::Codex, !codex_asked && codex_login),
    ]
    .into_iter()
    .filter_map(|(kind, offer)| offer.then_some(kind))
    .collect()
}

/// Finds `name.exe`, then `name.cmd`, in each directory in order and returns the first match.
///
/// Only absolute directories are searched: an empty `PATH` entry, `.`, or a relative,
/// root-relative (`\tools`) or drive-relative (`C:tools`) entry would resolve against the
/// current directory, where a planted `claude.cmd` could otherwise be picked up.
pub fn resolve_cli(
    name: &str,
    dirs: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    dirs.iter()
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| ["exe", "cmd"].map(|ext| dir.join(format!("{name}.{ext}"))))
        .find(|path| exists(path))
}

/// Directories to search for a provider's CLI: its known install locations (native installer,
/// then the npm global shim), followed by `PATH`. Unset variables are skipped.
pub fn candidate_dirs(kind: Kind, env: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let native = match kind {
        Kind::Claude => env("USERPROFILE").map(|d| PathBuf::from(d).join(r".local\bin")),
        Kind::Codex => {
            env("LOCALAPPDATA").map(|d| PathBuf::from(d).join(r"Programs\OpenAI\Codex\bin"))
        }
        Kind::Hardware => None,
    };
    let npm = env("APPDATA").map(|d| PathBuf::from(d).join("npm"));
    let path = env("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    native.into_iter().chain(npm).chain(path).collect()
}

/// The status command for an already resolved CLI, run from a trusted directory.
///
/// A `.cmd` shim is not wrapped by hand: for batch files the standard library runs
/// `<GetSystemDirectoryW>\cmd.exe /e:ON /v:OFF /d /c` with the script path quoted and the
/// arguments escaped, so `cmd.exe` comes from System32 and registry `AutoRun` is skipped.
fn login_command(cli: &Path, args: &[&str], cwd: &Path) -> Command {
    let mut command = Command::new(cli);
    command.args(args).current_dir(cwd);
    command
}

/// `C:\Windows\System32`: not writable by the user, so a safe working directory.
pub(crate) fn system_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    // SAFETY: the buffer is valid for its whole length; the call writes at most that much.
    let len = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    (len > 0 && len < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// Runs the provider's status command hidden and waits for it. The CLI is resolved to an
/// absolute path first; if it cannot be found, keep-alive is skipped. Output is discarded (it
/// contains account details).
pub fn run_login_status(kind: Kind) -> bool {
    let Some((name, args)) = status_command(kind).and_then(|c| c.split_first()) else {
        return false;
    };
    let dirs = candidate_dirs(kind, |k| std::env::var_os(k));
    let (Some(cli), Some(cwd)) = (resolve_cli(name, &dirs, Path::is_file), system_dir()) else {
        return false;
    };
    login_command(&cli, args, &cwd)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{FetchError, ProviderStatus, ProviderUsage};

    fn ok() -> ProviderStatus {
        ProviderStatus::Ok {
            usage: ProviderUsage {
                five_hour: None,
                weekly: None,
                ..Default::default()
            },
            at: 0,
        }
    }
    const EXPIRED: ProviderStatus = ProviderStatus::Failed(FetchError::Expired);

    #[test]
    fn runs_only_when_enabled_and_expired() {
        let k = KeepAlive::default();
        assert!(k.should_run(true, &EXPIRED, 1000));
        assert!(!k.should_run(false, &EXPIRED, 1000));
        assert!(!k.should_run(true, &ok(), 1000));
        assert!(!k.should_run(true, &ProviderStatus::Failed(FetchError::NoLogin), 1000));
    }

    #[test]
    fn at_most_once_per_15_minutes() {
        let mut k = KeepAlive::default();
        k.started(1000);
        assert!(!k.should_run(true, &EXPIRED, 1000 + 899));
        assert!(k.should_run(true, &EXPIRED, 1000 + 900));
    }

    #[test]
    fn records_renewal_when_ok_follows_attempt() {
        let mut k = KeepAlive::default();
        k.observe(&ok(), 500);
        assert_eq!(k.outcome(), Outcome::None);
        k.started(1000);
        k.observe(&EXPIRED, 1010);
        assert_eq!(k.outcome(), Outcome::StillExpired { tried_at: 1000 });
        k.observe(&ok(), 1020);
        assert_eq!(k.outcome(), Outcome::Renewed { at: 1020 });
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore = "runs the locally installed Claude Code CLI"]
    fn live_claude_auth_status_runs_hidden() {
        assert!(super::run_login_status(crate::model::Kind::Claude));
    }

    #[test]
    #[ignore = "runs the locally installed Codex CLI"]
    fn live_codex_login_status_runs_hidden() {
        assert!(super::run_login_status(crate::model::Kind::Codex));
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::model::Kind;

    #[test]
    fn each_provider_uses_its_own_status_command() {
        assert_eq!(
            status_command(Kind::Claude),
            Some(&["claude", "auth", "status"][..])
        );
        assert_eq!(
            status_command(Kind::Codex),
            Some(&["codex", "login", "status"][..])
        );
        assert_eq!(status_command(Kind::Hardware), None);
    }

    #[test]
    fn invitation_covers_only_logged_in_providers_not_yet_asked() {
        assert_eq!(
            invitation_kinds(false, false, true, true),
            [Kind::Claude, Kind::Codex]
        );
        assert_eq!(invitation_kinds(true, false, true, true), [Kind::Codex]);
        assert_eq!(invitation_kinds(false, false, false, true), [Kind::Codex]);
        assert!(invitation_kinds(true, true, true, true).is_empty());
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::model::Kind;
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    /// `exists` backed by a fixed set of files.
    fn files(list: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = list.iter().map(|s| p(s)).collect();
        move |path| set.contains(path)
    }

    #[test]
    fn resolver_returns_absolute_exe() {
        let dirs = [p(r"C:\tools")];
        assert_eq!(
            resolve_cli("claude", &dirs, files(&[r"C:\tools\claude.exe"])),
            Some(p(r"C:\tools\claude.exe"))
        );
    }

    #[test]
    fn resolver_prefers_exe_over_cmd_in_same_dir() {
        let dirs = [p(r"C:\npm")];
        let exists = files(&[r"C:\npm\claude.cmd", r"C:\npm\claude.exe"]);
        assert_eq!(
            resolve_cli("claude", &dirs, exists),
            Some(p(r"C:\npm\claude.exe"))
        );
    }

    #[test]
    fn resolver_falls_back_to_cmd_shim() {
        let dirs = [p(r"C:\npm")];
        assert_eq!(
            resolve_cli("codex", &dirs, files(&[r"C:\npm\codex.cmd"])),
            Some(p(r"C:\npm\codex.cmd"))
        );
    }

    #[test]
    fn resolver_keeps_directory_order() {
        let dirs = [p(r"C:\first"), p(r"C:\second")];
        let exists = files(&[r"C:\first\claude.cmd", r"C:\second\claude.exe"]);
        assert_eq!(
            resolve_cli("claude", &dirs, exists),
            Some(p(r"C:\first\claude.cmd"))
        );
    }

    #[test]
    fn resolver_ignores_current_and_relative_dirs() {
        // An empty PATH entry, ".", a relative dir, a root-relative and a drive-relative dir
        // all resolve against the current directory: never search them.
        let dirs = [p(""), p("."), p("bin"), p(r"\tools"), p(r"C:tools")];
        assert_eq!(resolve_cli("claude", &dirs, |_| true), None);
    }

    #[test]
    fn resolver_ignores_other_extensions() {
        let dirs = [p(r"C:\tools")];
        let exists = files(&[
            r"C:\tools\claude",
            r"C:\tools\claude.bat",
            r"C:\tools\claude.com",
            r"C:\tools\claude.ps1",
        ]);
        assert_eq!(resolve_cli("claude", &dirs, exists), None);
    }

    #[test]
    fn resolver_returns_none_when_not_found() {
        let dirs = [p(r"C:\tools"), p(r"C:\npm")];
        assert_eq!(resolve_cli("codex", &dirs, files(&[])), None);
    }

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn claude_known_dirs_come_before_path() {
        let e = env(&[
            ("USERPROFILE", r"C:\Users\u"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("PATH", r"C:\a;.;C:\b"),
        ]);
        assert_eq!(
            candidate_dirs(Kind::Claude, e),
            [
                p(r"C:\Users\u\.local\bin"),
                p(r"C:\Users\u\AppData\Roaming\npm"),
                p(r"C:\a"),
                p("."),
                p(r"C:\b"),
            ]
        );
    }

    #[test]
    fn codex_known_dirs_come_before_path() {
        let e = env(&[
            ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("PATH", r"C:\a"),
        ]);
        assert_eq!(
            candidate_dirs(Kind::Codex, e),
            [
                p(r"C:\Users\u\AppData\Local\Programs\OpenAI\Codex\bin"),
                p(r"C:\Users\u\AppData\Roaming\npm"),
                p(r"C:\a"),
            ]
        );
    }

    #[test]
    fn missing_env_vars_are_skipped() {
        assert!(candidate_dirs(Kind::Claude, env(&[])).is_empty());
        assert_eq!(
            candidate_dirs(Kind::Codex, env(&[("PATH", r"C:\a")])),
            [p(r"C:\a")]
        );
    }

    #[test]
    fn codex_npm_shim_wins_over_windowsapps_on_path() {
        let e = env(&[
            ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("PATH", r"C:\Users\u\AppData\Local\Microsoft\WindowsApps"),
        ]);
        let dirs = candidate_dirs(Kind::Codex, e);
        let exists = files(&[
            r"C:\Users\u\AppData\Roaming\npm\codex.cmd",
            r"C:\Users\u\AppData\Local\Microsoft\WindowsApps\codex.exe",
        ]);
        assert_eq!(
            resolve_cli("codex", &dirs, exists),
            Some(p(r"C:\Users\u\AppData\Roaming\npm\codex.cmd"))
        );
    }

    #[test]
    fn login_command_runs_resolved_path_from_trusted_dir() {
        let cmd = login_command(
            Path::new(r"C:\npm\claude.cmd"),
            &["auth", "status"],
            Path::new(r"C:\Windows\System32"),
        );
        assert_eq!(cmd.get_program(), r"C:\npm\claude.cmd");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["auth", "status"]);
        assert_eq!(
            cmd.get_current_dir(),
            Some(Path::new(r"C:\Windows\System32"))
        );
    }

    #[test]
    fn system_dir_is_absolute_system32() {
        let dir = system_dir().expect("GetSystemDirectoryW");
        assert!(dir.is_absolute(), "{dir:?}");
        assert!(dir.join("cmd.exe").is_file(), "{dir:?}");
        assert!(dir.ends_with("System32") || dir.ends_with("system32"));
    }

    /// Real spawn of a `.cmd` shim in a directory whose name needs quoting and contains cmd
    /// metacharacters: the shim records its working directory and arguments.
    #[test]
    fn cmd_shim_runs_with_trusted_dir_and_exact_args() {
        let base = std::env::temp_dir().join(format!(
            "cct-keepalive-test-{} a&b ^c (d)",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let shim = base.join("claude.cmd");
        std::fs::write(
            &shim,
            "@echo off\r\n> \"%~dp0out.txt\" echo cwd=%CD%;args=%*\r\n",
        )
        .unwrap();
        let cwd = system_dir().unwrap();

        let status = login_command(&shim, &["auth", "status"], &cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        let out = std::fs::read_to_string(base.join("out.txt")).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert!(status.success());
        assert_eq!(
            out.trim().to_lowercase(),
            format!("cwd={};args=auth status", cwd.display()).to_lowercase()
        );
    }
}
