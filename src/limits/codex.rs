//! Codex subscription limits, read with the CLI's existing ChatGPT login.
//!
//! `auth.json` is only ever read. When the HTTP endpoint fails, the installed `codex` CLI's
//! JSON-RPC `app-server` (`account/rateLimits/read`) is queried over stdio; if that also fails,
//! the newest `rate_limits` event the CLI wrote to its session logs is used if it has not reset.

use super::backoff::Backoff;
use super::http::get_json;
use super::keepalive::{CREATE_NO_WINDOW, candidate_dirs, resolve_cli, system_dir};
use super::terminal::Job;
use super::{FetchError, Provider, ProviderUsage, WindowUsage};
use crate::model::Kind;
use crate::timeutil::{now_unix, parse_rfc3339};
use serde_json::Value;
use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(5);
const APP_SERVER_REQUEST: &str = concat!(
    r#"{"method":"initialize","id":1,"params":{"clientInfo":{"name":"claude-codex-taskbar","title":"CCT","version":""#,
    env!("CARGO_PKG_VERSION"),
    r#""}}}"#,
    "\n",
    r#"{"method":"initialized","params":{}}"#,
    "\n",
    r#"{"method":"account/rateLimits/read","id":3,"params":{}}"#,
    "\n",
);

pub struct CodexCreds {
    pub access_token: String,
    pub account_id: Option<String>,
}

impl std::fmt::Debug for CodexCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexCreds")
            .field("access_token", &"<redacted>")
            .field(
                "account_id",
                &self.account_id.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

pub fn parse_codex_auth(text: &str) -> Result<CodexCreds, FetchError> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| FetchError::NoLogin)?;
    let token = v["tokens"]["access_token"]
        .as_str()
        .filter(|t| !t.is_empty());
    match token {
        Some(token) => Ok(CodexCreds {
            access_token: token.to_owned(),
            account_id: v["tokens"]["account_id"].as_str().map(str::to_owned),
        }),
        None if v["OPENAI_API_KEY"].as_str().is_some_and(|k| !k.is_empty()) => {
            Err(FetchError::NoSubscription)
        }
        None => Err(FetchError::NoLogin),
    }
}

fn window(pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowUsage> {
    Some(WindowUsage {
        used_pct: (pct? as f32).clamp(0.0, 100.0),
        resets_at,
    })
}

/// Reported duration takes precedence over the legacy primary/secondary ordering.
fn assign_window(
    usage: &mut ProviderUsage,
    reading: Option<WindowUsage>,
    seconds: Option<i64>,
    secondary: bool,
) {
    let slot = match seconds {
        Some(18000) => &mut usage.five_hour,
        Some(604800) => &mut usage.weekly,
        None if secondary => &mut usage.weekly,
        None => &mut usage.five_hour,
        Some(_) => return,
    };
    if reading.is_some() {
        *slot = reading;
    }
}

/// `now` resolves the relative `reset_after_seconds` field.
pub fn parse_codex_usage(v: &Value, now: i64) -> Result<ProviderUsage, FetchError> {
    let rl = v
        .get("rate_limit")
        .filter(|r| r.is_object())
        .ok_or_else(|| FetchError::BadResponse("unexpected usage format".into()))?;
    let parse = |w: &Value| {
        let resets_at = w["reset_at"]
            .as_i64()
            .or_else(|| w["reset_after_seconds"].as_i64().map(|s| now + s));
        window(w["used_percent"].as_f64(), resets_at)
    };
    let mut usage = ProviderUsage {
        five_hour: None,
        weekly: None,
        ..Default::default()
    };
    for (name, secondary) in [("primary_window", false), ("secondary_window", true)] {
        let w = &rl[name];
        assign_window(
            &mut usage,
            parse(w),
            w["limit_window_seconds"].as_i64(),
            secondary,
        );
    }
    Ok(usage)
}

/// Parses the JSON-RPC `account/rateLimits/read` response from `codex app-server`.
pub fn parse_app_server_rate_limits(v: &Value) -> Result<ProviderUsage, FetchError> {
    if v.get("error").is_some_and(|e| !e.is_null()) {
        return Err(FetchError::BadResponse("app-server error response".into()));
    }
    let result = v.get("result").unwrap_or(v);
    let rl = result
        .get("rateLimits")
        .or_else(|| result["rateLimitsByLimitId"].get("codex"))
        .filter(|r| r.is_object())
        .ok_or_else(|| FetchError::BadResponse("unexpected app-server format".into()))?;
    let parse = |w: &Value| window(w["usedPercent"].as_f64(), w["resetsAt"].as_i64());
    let mut usage = ProviderUsage {
        five_hour: None,
        weekly: None,
        ..Default::default()
    };
    for (name, secondary) in [("primary", false), ("secondary", true)] {
        let w = &rl[name];
        let seconds = w["windowDurationMins"]
            .as_i64()
            .map(|m| m.saturating_mul(60));
        assign_window(&mut usage, parse(w), seconds, secondary);
    }
    Ok(usage)
}

/// Runs `codex app-server --listen stdio://` from `cwd` inside a kill-on-close Job Object,
/// sends `account/rateLimits/read`, waits up to `timeout` for response `id: 3`, and always
/// kills the whole process tree and reaps the child.
pub fn run_app_server_rate_limits(
    cli: &Path,
    cwd: &Path,
    codex_home: &Path,
    timeout: Duration,
) -> Result<ProviderUsage, FetchError> {
    let mut child = Command::new(cli)
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(cwd)
        .env("CODEX_HOME", codex_home)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| FetchError::Network("app-server spawn failed".into()))?;
    let job = Job::assign(&child);

    let mut stdin = child.stdin.take();
    let Some(stdout) = child.stdout.take() else {
        drop(stdin);
        drop(job);
        let _ = child.kill();
        let _ = child.wait();
        return Err(FetchError::Network("app-server stdout unavailable".into()));
    };

    if let Some(s) = stdin.as_mut()
        && (s.write_all(APP_SERVER_REQUEST.as_bytes()).is_err() || s.flush().is_err())
    {
        drop(stdin);
        drop(job);
        let _ = child.kill();
        let _ = child.wait();
        return Err(FetchError::Network("app-server write failed".into()));
    }

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && v["id"].as_i64() == Some(3)
            {
                let _ = tx.send(parse_app_server_rate_limits(&v));
                return;
            }
        }
        let _ = tx.send(Err(FetchError::BadResponse(
            "app-server closed without rateLimits response".into(),
        )));
    });

    let result = match rx.recv_timeout(timeout) {
        Ok(res) => res,
        Err(_) => Err(FetchError::Network("app-server timed out".into())),
    };
    drop(stdin);
    drop(job);
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// Parses one session-log line; `None` unless it carries `payload.rate_limits`.
fn session_line_limits(line: &str) -> Option<(ProviderUsage, i64)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let rl = v["payload"].get("rate_limits").filter(|r| r.is_object())?;
    let at = v["timestamp"].as_str().and_then(parse_rfc3339)?;
    let parse = |w: &Value| {
        let resets_at = w["resets_at"]
            .as_i64()
            .or_else(|| w["resets_in_seconds"].as_i64().map(|s| at + s));
        window(w["used_percent"].as_f64(), resets_at)
    };
    let mut usage = ProviderUsage {
        five_hour: None,
        weekly: None,
        ..Default::default()
    };
    for (name, secondary) in [("primary", false), ("secondary", true)] {
        let w = &rl[name];
        let seconds = w["window_minutes"].as_i64().map(|m| m.saturating_mul(60));
        assign_window(&mut usage, parse(w), seconds, secondary);
    }
    Some((usage, at))
}

/// Older logs can't hold a live window: the weekly limit has reset since.
const SESSION_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;
const SESSION_MAX_FILES: usize = 20;
const SESSION_TAIL_BYTES: u64 = 256 * 1024;
/// `sessions/YYYY/MM/DD/*.jsonl`, plus one spare level.
const SESSION_MAX_DEPTH: usize = 4;

fn unix_mtime(meta: &std::fs::Metadata) -> Option<i64> {
    let modified = meta.modified().ok()?;
    let secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    i64::try_from(secs).ok()
}

/// Lists `*.jsonl` files modified at or after `cutoff`, from directory metadata only.
/// Symlinks are not followed.
fn collect_recent_sessions(dir: &Path, cutoff: i64, depth: usize, out: &mut Vec<(i64, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if depth > 0 {
                collect_recent_sessions(&path, cutoff, depth - 1, out);
            }
        } else if file_type.is_file() && path.extension().is_some_and(|x| x == "jsonl") {
            let mtime = entry.metadata().ok().as_ref().and_then(unix_mtime);
            if let Some(mtime) = mtime.filter(|&m| m >= cutoff) {
                out.push((mtime, path));
            }
        }
    }
}

/// Reads at most the last `max_bytes` of a file, without the partial line the cut leaves.
fn read_tail(path: &Path, max_bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity(usize::try_from(len - start).ok()?);
    file.take(max_bytes).read_to_end(&mut buf).ok()?;
    if start > 0 {
        let first_newline = buf.iter().position(|&b| b == b'\n')?;
        buf.drain(..=first_newline);
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Newest `rate_limits` line among recent session logs, scanning only each file's tail.
fn newest_session_limits(dir: &Path, now: i64) -> Option<(ProviderUsage, i64)> {
    let mut files = Vec::new();
    let cutoff = now.saturating_sub(SESSION_MAX_AGE_SECS);
    collect_recent_sessions(dir, cutoff, SESSION_MAX_DEPTH, &mut files);
    files.sort_unstable_by(|a, b| b.cmp(a));
    files.iter().take(SESSION_MAX_FILES).find_map(|(_, path)| {
        let tail = read_tail(path, SESSION_TAIL_BYTES)?;
        tail.lines()
            .rev()
            .filter(|l| l.contains("\"rate_limits\""))
            .find_map(session_line_limits)
    })
}

/// Last limits the CLI logged, with windows whose reset time has passed removed.
pub fn latest_session_limits(sessions_dir: &Path, now: i64) -> Option<(ProviderUsage, i64)> {
    let (mut usage, at) = newest_session_limits(sessions_dir, now)?;
    let still_valid = |w: &Option<WindowUsage>| {
        w.as_ref()
            .and_then(|w| w.resets_at)
            .is_some_and(|r| r > now)
    };
    if !still_valid(&usage.five_hour) {
        usage.five_hour = None;
    }
    if !still_valid(&usage.weekly) {
        usage.weekly = None;
    }
    if usage.five_hour.is_none() && usage.weekly.is_none() {
        return None;
    }
    Some((usage, at))
}

pub struct CodexProvider {
    pub home: PathBuf,
    app_server_backoff: Cell<Backoff>,
}

/// Result of a Codex fetch: usage, whether it came from session logs, and its timestamp.
pub type CodexFetch = (ProviderUsage, bool, i64);

impl CodexProvider {
    pub fn new(home: PathBuf) -> Self {
        CodexProvider {
            home,
            app_server_backoff: Cell::new(Backoff::default()),
        }
    }

    /// `%CODEX_HOME%`, or `%USERPROFILE%\.codex`.
    pub fn default_home() -> PathBuf {
        if let Some(home) = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) {
            return PathBuf::from(home);
        }
        let profile = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default();
        profile.join(".codex")
    }

    fn read_creds(&self) -> Result<CodexCreds, FetchError> {
        let text = std::fs::read_to_string(self.home.join("auth.json"))
            .map_err(|_| FetchError::NoLogin)?;
        parse_codex_auth(&text)
    }

    /// Falls back to `codex app-server` and then session logs only for transient failures,
    /// never for auth problems. With no network at all, only the session logs are tried. After an app-server failure that tier is skipped for a
    /// doubling back-off, reset by any live success.
    pub fn fetch_with_fallback(&self, now: i64) -> Result<CodexFetch, FetchError> {
        self.fetch_with_tiers(now, |now| self.fetch_at(now), || self.fetch_app_server())
    }

    fn fetch_with_tiers(
        &self,
        now: i64,
        http: impl FnOnce(i64) -> Result<ProviderUsage, FetchError>,
        app_server: impl FnOnce() -> Result<ProviderUsage, FetchError>,
    ) -> Result<CodexFetch, FetchError> {
        match http(now) {
            Ok(usage) => {
                self.app_server_backoff.set(Backoff::default());
                Ok((usage, false, now))
            }
            Err(
                err
                @ (FetchError::Network(_) | FetchError::BadResponse(_) | FetchError::Offline(_)),
            ) => {
                // The CLI needs the same network, so an offline error skips straight to the
                // session logs and leaves the back-off alone: the app-server wasn't tried (#106).
                let backoff = self.app_server_backoff.get();
                if !matches!(err, FetchError::Offline(_)) && backoff.allows(now) {
                    match app_server() {
                        Ok(usage) => {
                            self.app_server_backoff.set(Backoff::default());
                            return Ok((usage, false, now));
                        }
                        Err(_) => self.app_server_backoff.set(backoff.failed(now)),
                    }
                }
                latest_session_limits(&self.home.join("sessions"), now)
                    .map(|(usage, at)| (usage, true, at))
                    .ok_or(err)
            }
            Err(err) => Err(err),
        }
    }

    fn fetch_app_server(&self) -> Result<ProviderUsage, FetchError> {
        let dirs = candidate_dirs(Kind::Codex, |k| std::env::var_os(k));
        let cli = resolve_cli("codex", &dirs, Path::is_file)
            .ok_or_else(|| FetchError::Network("codex CLI not found".into()))?;
        let cwd = system_dir().ok_or_else(|| FetchError::Network("system32 not found".into()))?;
        run_app_server_rate_limits(&cli, &cwd, &self.home, APP_SERVER_TIMEOUT)
    }

    fn fetch_at(&self, now: i64) -> Result<ProviderUsage, FetchError> {
        let creds = self.read_creds()?;
        let auth = format!("Bearer {}", creds.access_token);
        let mut headers = vec![("Authorization", auth.as_str())];
        if let Some(id) = &creds.account_id {
            headers.push(("ChatGPT-Account-Id", id.as_str()));
        }
        parse_codex_usage(&get_json(USAGE_URL, &headers)?, now)
    }
}

impl Provider for CodexProvider {
    fn fetch(&self) -> Result<ProviderUsage, FetchError> {
        self.fetch_at(now_unix())
    }

    fn has_login(&self) -> bool {
        self.read_creds().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{FetchError, WindowUsage};
    use crate::timeutil::parse_rfc3339;
    use serde_json::json;
    use std::path::PathBuf;

    const AUTH: &str = include_str!("../../tests/fixtures/codex_auth.json");
    const USAGE: &str = include_str!("../../tests/fixtures/codex_usage.json");
    const APP_SERVER: &str = include_str!("../../tests/fixtures/codex_app_server.json");
    const SESSION: &str = include_str!("../../tests/fixtures/codex_session.jsonl");

    fn sessions_dir(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("wuw-codex-{}-{name}", std::process::id()))
            .join("sessions");
        let day = dir.join("2026").join("10").join("26");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-FAKE.jsonl"), content).unwrap();
        dir
    }

    #[test]
    fn auth_ok() {
        let c = parse_codex_auth(AUTH).unwrap();
        assert_eq!(c.access_token, "FAKE-ACCESS");
        assert_eq!(
            c.account_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000000")
        );
        assert!(!format!("{c:?}").contains("FAKE-ACCESS"));
    }

    #[test]
    fn auth_with_bom() {
        assert!(parse_codex_auth(&format!("\u{feff}{AUTH}")).is_ok());
    }

    #[test]
    fn auth_api_key_only_is_no_subscription() {
        let text = r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-FAKE","tokens":null}"#;
        assert_eq!(
            parse_codex_auth(text).unwrap_err(),
            FetchError::NoSubscription
        );
    }

    #[test]
    fn auth_empty_is_no_login() {
        assert_eq!(parse_codex_auth("").unwrap_err(), FetchError::NoLogin);
        assert_eq!(parse_codex_auth("{}").unwrap_err(), FetchError::NoLogin);
    }

    #[test]
    fn usage_fixture_parses() {
        let v: serde_json::Value = serde_json::from_str(USAGE).unwrap();
        let u = parse_codex_usage(&v, 0).unwrap();
        assert_eq!(
            u.five_hour,
            Some(WindowUsage {
                used_pct: 12.0,
                resets_at: Some(1793385254)
            })
        );
        assert_eq!(
            u.weekly,
            Some(WindowUsage {
                used_pct: 40.0,
                resets_at: Some(1793972054)
            })
        );
    }

    #[test]
    fn usage_primary_weekly_is_not_five_hour() {
        let v = json!({"rate_limit": {
            "primary_window": {"used_percent": 55, "limit_window_seconds": 604800, "reset_at": 1794267982i64},
            "secondary_window": null
        }});
        let u = parse_codex_usage(&v, 0).unwrap();
        assert_eq!(u.five_hour, None);
        assert_eq!(
            u.weekly,
            Some(WindowUsage {
                used_pct: 55.0,
                resets_at: Some(1794267982)
            })
        );
    }

    #[test]
    fn usage_window_duration_overrides_position() {
        let v = json!({"rate_limit": {
            "primary_window": {"used_percent": 55, "limit_window_seconds": 604800},
            "secondary_window": {"used_percent": 12, "limit_window_seconds": 18000}
        }});
        let u = parse_codex_usage(&v, 0).unwrap();
        assert_eq!(u.five_hour.unwrap().used_pct, 12.0);
        assert_eq!(u.weekly.unwrap().used_pct, 55.0);
    }

    #[test]
    fn usage_unknown_duration_is_not_mislabeled() {
        let v = json!({"rate_limit": {
            "primary_window": {"used_percent": 55, "limit_window_seconds": 3600}
        }});
        let u = parse_codex_usage(&v, 0).unwrap();
        assert_eq!(u.five_hour, None);
        assert_eq!(u.weekly, None);
    }

    #[test]
    fn session_primary_weekly_is_not_five_hour() {
        let line = r#"{"timestamp":"2026-10-26T15:56:19Z","payload":{"rate_limits":{"primary":{"used_percent":55,"window_minutes":10080,"resets_at":1794267982},"secondary":null}}}"#;
        let (u, _) = session_line_limits(line).unwrap();
        assert_eq!(u.five_hour, None);
        assert_eq!(
            u.weekly,
            Some(WindowUsage {
                used_pct: 55.0,
                resets_at: Some(1794267982)
            })
        );
    }

    #[test]
    fn missing_durations_preserve_both_positional_windows() {
        let api = json!({"rate_limit": {
            "primary_window": {"used_percent": 12, "reset_at": 1793035631},
            "secondary_window": {"used_percent": 55, "reset_at": 1793622431}
        }});
        let session = r#"{"timestamp":"2026-10-26T15:56:19Z","payload":{"rate_limits":{"primary":{"used_percent":12,"resets_at":1793035631},"secondary":{"used_percent":55,"resets_at":1793622431}}}}"#;
        let api = parse_codex_usage(&api, 0).unwrap();
        let (session, _) = session_line_limits(session).unwrap();
        assert_eq!(api, session);
        let five = api.five_hour.unwrap();
        let week = api.weekly.unwrap();
        assert_eq!(five.used_pct, 12.0);
        assert_eq!(five.resets_at, Some(1793035631));
        assert_eq!(week.used_pct, 55.0);
        assert_eq!(week.resets_at, Some(1793622431));
    }

    #[test]
    fn session_window_duration_overrides_position() {
        let line = r#"{"timestamp":"2026-10-26T15:56:19Z","payload":{"rate_limits":{"primary":{"used_percent":55,"window_minutes":10080,"resets_at":1793622431},"secondary":{"used_percent":12,"window_minutes":300,"resets_at":1793035631}}}}"#;
        let (usage, _) = session_line_limits(line).unwrap();
        let five = usage.five_hour.unwrap();
        let week = usage.weekly.unwrap();
        assert_eq!(five.used_pct, 12.0);
        assert_eq!(five.resets_at, Some(1793035631));
        assert_eq!(week.used_pct, 55.0);
        assert_eq!(week.resets_at, Some(1793622431));
    }

    #[test]
    fn usage_reset_after_seconds() {
        let v = json!({"rate_limit": {"primary_window": {"used_percent": 5, "reset_after_seconds": 600}}});
        let u = parse_codex_usage(&v, 1000).unwrap();
        assert_eq!(
            u.five_hour,
            Some(WindowUsage {
                used_pct: 5.0,
                resets_at: Some(1600)
            })
        );
        assert_eq!(u.weekly, None);
    }

    #[test]
    fn usage_without_rate_limit_is_bad_response() {
        assert!(matches!(
            parse_codex_usage(&json!({"plan_type": "free"}), 0),
            Err(FetchError::BadResponse(_))
        ));
    }

    #[test]
    fn session_fallback_picks_last_line() {
        let dir = sessions_dir("last", SESSION);
        let (u, at) = latest_session_limits(&dir, 1793000000).unwrap();
        assert_eq!(
            u.five_hour,
            Some(WindowUsage {
                used_pct: 44.0,
                resets_at: Some(1793035631)
            })
        );
        assert_eq!(
            u.weekly,
            Some(WindowUsage {
                used_pct: 7.0,
                resets_at: Some(1793622431)
            })
        );
        assert_eq!(at, parse_rfc3339("2026-10-26T15:56:19Z").unwrap());
    }

    #[test]
    fn session_fallback_drops_expired_window() {
        let dir = sessions_dir("expired", SESSION);
        let (u, _) = latest_session_limits(&dir, 1793100000).unwrap();
        assert_eq!(u.five_hour, None);
        assert!(u.weekly.is_some());
    }

    #[test]
    fn session_fallback_all_expired_is_none() {
        let dir = sessions_dir("allexpired", SESSION);
        assert!(latest_session_limits(&dir, 1799999999).is_none());
    }

    #[test]
    fn session_fallback_missing_dir_is_none() {
        assert!(
            latest_session_limits(&std::env::temp_dir().join("wuw-nope-sessions"), 0).is_none()
        );
    }

    #[test]
    fn session_fallback_resets_in_seconds() {
        let line = r#"{"timestamp":"2026-10-26T15:56:19Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":10.0,"window_minutes":300,"resets_in_seconds":100}}}}"#;
        let dir = sessions_dir("relative", line);
        let ts = parse_rfc3339("2026-10-26T15:56:19Z").unwrap();
        let (u, _) = latest_session_limits(&dir, ts).unwrap();
        assert_eq!(
            u.five_hour,
            Some(WindowUsage {
                used_pct: 10.0,
                resets_at: Some(ts + 100)
            })
        );
    }

    const SESSION_NOW: i64 = 1793000000;

    fn set_mtime(path: &Path, unix: i64) {
        let at = std::time::UNIX_EPOCH + Duration::from_secs(unix as u64);
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(at).unwrap();
    }

    fn day_dir(sessions: &Path) -> PathBuf {
        sessions.join("2026").join("10").join("26")
    }

    /// Session-log lines with no limits, at least `bytes` long in total.
    fn filler(bytes: usize) -> String {
        let line = format!(
            r#"{{"timestamp":"2026-10-26T15:58:00Z","type":"response_item","payload":{{"type":"message","text":"{}"}}}}"#,
            "x".repeat(1000)
        );
        let mut out = String::new();
        while out.len() < bytes {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    #[test]
    fn session_fallback_skips_old_files() {
        let dir = sessions_dir("old", SESSION);
        let file = day_dir(&dir).join("rollout-FAKE.jsonl");
        set_mtime(&file, SESSION_NOW - SESSION_MAX_AGE_SECS - 60);
        assert!(latest_session_limits(&dir, SESSION_NOW).is_none());

        set_mtime(&file, SESSION_NOW - SESSION_MAX_AGE_SECS + 60);
        assert!(latest_session_limits(&dir, SESSION_NOW).is_some());
    }

    #[test]
    fn session_fallback_finds_limits_near_the_end_of_a_large_file() {
        let content = format!("{}{SESSION}", filler(4 * SESSION_TAIL_BYTES as usize));
        let dir = sessions_dir("large-end", &content);
        let (u, _) = latest_session_limits(&dir, SESSION_NOW).unwrap();
        assert_eq!(u.five_hour.unwrap().used_pct, 44.0);
    }

    #[test]
    fn session_fallback_reads_only_the_tail_of_a_large_file() {
        let content = format!("{SESSION}{}", filler(2 * SESSION_TAIL_BYTES as usize));
        let dir = sessions_dir("large-start", &content);
        assert!(latest_session_limits(&dir, SESSION_NOW).is_none());
    }

    #[test]
    fn session_fallback_prefers_the_newest_file() {
        let dir = sessions_dir("newest", SESSION);
        let older = day_dir(&dir).join("rollout-FAKE.jsonl");
        let newer = day_dir(&dir).join("rollout-NEWER.jsonl");
        std::fs::write(&newer, SESSION.replace("44.0", "61.0")).unwrap();
        set_mtime(&older, SESSION_NOW - 120);
        set_mtime(&newer, SESSION_NOW - 60);
        let (u, _) = latest_session_limits(&dir, SESSION_NOW).unwrap();
        assert_eq!(u.five_hour.unwrap().used_pct, 61.0);
    }

    #[test]
    fn session_fallback_stops_after_max_files() {
        let dir = sessions_dir("max-files", SESSION);
        let day = day_dir(&dir);
        set_mtime(&day.join("rollout-FAKE.jsonl"), SESSION_NOW - 3600);
        for i in 0..SESSION_MAX_FILES {
            let path = day.join(format!("rollout-EMPTY-{i}.jsonl"));
            std::fs::write(&path, filler(1)).unwrap();
            set_mtime(&path, SESSION_NOW - i as i64);
        }
        assert!(latest_session_limits(&dir, SESSION_NOW).is_none());

        std::fs::remove_file(day.join("rollout-EMPTY-0.jsonl")).unwrap();
        assert!(latest_session_limits(&dir, SESSION_NOW).is_some());
    }

    #[test]
    fn app_server_fixture_parses() {
        let v: serde_json::Value = serde_json::from_str(APP_SERVER).unwrap();
        let u = parse_app_server_rate_limits(&v).unwrap();
        assert_eq!(
            u.five_hour,
            Some(WindowUsage {
                used_pct: 12.0,
                resets_at: Some(1793385254)
            })
        );
        assert_eq!(
            u.weekly,
            Some(WindowUsage {
                used_pct: 40.0,
                resets_at: Some(1793972054)
            })
        );
    }

    #[test]
    fn app_server_reversed_positions_follow_duration() {
        let v = json!({
            "id": 3,
            "result": {
                "rateLimits": {
                    "primary": {
                        "usedPercent": 55,
                        "windowDurationMins": 10080,
                        "resetsAt": 1793622431
                    },
                    "secondary": {
                        "usedPercent": 12,
                        "windowDurationMins": 300,
                        "resetsAt": 1793035631
                    }
                }
            }
        });
        let u = parse_app_server_rate_limits(&v).unwrap();
        assert_eq!(u.five_hour.unwrap().used_pct, 12.0);
        assert_eq!(u.weekly.unwrap().used_pct, 55.0);
    }

    #[test]
    fn app_server_missing_window_and_primary_weekly() {
        let v = json!({
            "id": 3,
            "result": {
                "rateLimits": {
                    "primary": {
                        "usedPercent": 55,
                        "windowDurationMins": 10080,
                        "resetsAt": 1794267982i64
                    },
                    "secondary": null
                }
            }
        });
        let u = parse_app_server_rate_limits(&v).unwrap();
        assert_eq!(u.five_hour, None);
        assert_eq!(
            u.weekly,
            Some(WindowUsage {
                used_pct: 55.0,
                resets_at: Some(1794267982)
            })
        );
    }

    #[test]
    fn app_server_unknown_duration_is_not_mislabeled() {
        let v = json!({
            "id": 3,
            "result": {
                "rateLimits": {
                    "primary": {
                        "usedPercent": 55,
                        "windowDurationMins": 60,
                        "resetsAt": 1794267982i64
                    },
                    "secondary": null
                }
            }
        });
        let u = parse_app_server_rate_limits(&v).unwrap();
        assert_eq!(u.five_hour, None);
        assert_eq!(u.weekly, None);
    }

    #[test]
    fn app_server_error_and_missing_payload_are_bad_response() {
        let err = json!({"id": 3, "error": {"code": -32601, "message": "Method not found"}});
        assert!(matches!(
            parse_app_server_rate_limits(&err),
            Err(FetchError::BadResponse(_))
        ));
        let empty = json!({"id": 3, "result": {}});
        assert!(matches!(
            parse_app_server_rate_limits(&empty),
            Err(FetchError::BadResponse(_))
        ));
    }

    #[test]
    fn app_server_runner_reads_id_3_after_notifications() {
        let base = std::env::temp_dir().join(format!("wuw-codex-app-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let shim = base.join("codex.cmd");
        let script = concat!(
            "@echo off\r\n",
            "> \"%~dp0args.txt\" echo cwd=%CD%;args=%*\r\n",
            "echo {\"id\":1,\"result\":{\"userAgent\":\"test\"}}\r\n",
            "echo {\"method\":\"remoteControl/status/changed\",\"params\":{}}\r\n",
            "echo {\"id\":3,\"result\":{\"rateLimits\":{\"primary\":{\"usedPercent\":25,\"windowDurationMins\":300,\"resetsAt\":1793385254},\"secondary\":{\"usedPercent\":60,\"windowDurationMins\":10080,\"resetsAt\":1793972054}}}}\r\n",
        );
        std::fs::write(&shim, script).unwrap();
        let cwd = system_dir().unwrap();

        let u = run_app_server_rate_limits(&shim, &cwd, &base, Duration::from_secs(3)).unwrap();
        let args_out = std::fs::read_to_string(base.join("args.txt")).unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(u.five_hour.unwrap().used_pct, 25.0);
        assert_eq!(u.weekly.unwrap().used_pct, 60.0);
        assert_eq!(
            args_out.trim().to_lowercase(),
            format!("cwd={};args=app-server --listen stdio://", cwd.display()).to_lowercase()
        );
    }

    #[test]
    fn app_server_runner_times_out_and_kills_hung_child() {
        let base = std::env::temp_dir().join(format!("wuw-codex-app-hang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let shim = base.join("codex.cmd");
        std::fs::write(&shim, "@echo off\r\n:hang\r\nset /p x=\r\ngoto hang\r\n").unwrap();
        let cwd = system_dir().unwrap();

        let res = run_app_server_rate_limits(&shim, &cwd, &base, Duration::from_millis(150));
        let _ = std::fs::remove_dir_all(&base);

        assert!(matches!(res, Err(FetchError::Network(_))));
    }

    #[test]
    fn app_server_runner_times_out_and_kills_process_tree() {
        if let Ok(pid_file) = std::env::var("CCT_APP_SERVER_GRANDCHILD_PID") {
            std::fs::write(pid_file, std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        let base = std::env::temp_dir().join(format!("wuw-codex-app-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let pid_file = base.join("grandchild.pid");
        let shim = base.join("codex.cmd");
        let exe = std::env::current_exe().unwrap();
        let script = format!(
            "@echo off\r\nset \"CCT_APP_SERVER_GRANDCHILD_PID={}\"\r\n\"{}\" --exact limits::codex::tests::app_server_runner_times_out_and_kills_process_tree\r\n",
            pid_file.display(),
            exe.display()
        );
        std::fs::write(&shim, script).unwrap();
        let cwd = system_dir().unwrap();

        let res = run_app_server_rate_limits(&shim, &cwd, &base, Duration::from_millis(500));
        let pid: u32 = std::fs::read_to_string(&pid_file)
            .expect("grandchild should write its PID before timeout")
            .trim()
            .parse()
            .unwrap();
        let _ = std::fs::remove_dir_all(&base);

        assert!(matches!(res, Err(FetchError::Network(_))));
        assert!(
            crate::limits::terminal::process_has_exited(pid, 2_000),
            "grandchild process {pid} should be killed with the job"
        );
    }

    #[test]
    fn app_server_weekly_only_replaces_five_hour_through_stale_and_reset() {
        use crate::limits::ProviderStatus;
        use crate::limits::state::{age_status, next_status};

        // 1. t=1000: successful status has both windows, with the 5h reset at 5000.
        let initial = ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: 12.0,
                resets_at: Some(5000),
            }),
            weekly: Some(WindowUsage {
                used_pct: 40.0,
                resets_at: Some(1400),
            }),
            ..Default::default()
        };
        let status = next_status(&ProviderStatus::Loading, Ok(initial), 1000);

        // 2. t=1100: weekly-only app-server result replaces status; old 5h is absent immediately.
        let weekly_only = json!({
            "id": 3,
            "result": {
                "rateLimits": {
                    "primary": {
                        "usedPercent": 55,
                        "windowDurationMins": 10080,
                        "resetsAt": 1400
                    },
                    "secondary": null
                }
            }
        });
        let status = next_status(&status, parse_app_server_rate_limits(&weekly_only), 1100);
        assert_eq!(
            status,
            ProviderStatus::Ok {
                usage: ProviderUsage {
                    five_hour: None,
                    weekly: Some(WindowUsage {
                        used_pct: 55.0,
                        resets_at: Some(1400),
                    }),
                    ..Default::default()
                },
                at: 1100,
            }
        );

        // 3. t=1200: transient Network error preserves only weekly as stale with at=1100.
        let err = FetchError::Network("timeout".into());
        let status = next_status(&status, Err(err.clone()), 1200);
        assert_eq!(
            status,
            ProviderStatus::Stale {
                usage: ProviderUsage {
                    five_hour: None,
                    weekly: Some(WindowUsage {
                        used_pct: 55.0,
                        resets_at: Some(1400),
                    }),
                    ..Default::default()
                },
                at: 1100,
                err: err.clone(),
            }
        );

        // 4. t=1400: aging with max_age=900 drops expired weekly without inventing a 0% reading.
        let aged = age_status(&status, 1400, 900);
        assert_eq!(
            aged,
            ProviderStatus::Stale {
                usage: ProviderUsage {
                    five_hour: None,
                    weekly: None,
                    ..Default::default()
                },
                at: 1100,
                err,
            }
        );
    }

    #[test]
    fn fallback_chain_prefers_http_then_app_server_then_session_logs() {
        let dir = sessions_dir("tiers", SESSION);
        let home = dir.parent().unwrap().to_path_buf();
        let provider = CodexProvider::new(home);
        let now = 1793000000;

        let sample = |pct: f32| ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: pct,
                resets_at: Some(1793385254),
            }),
            weekly: None,
            ..Default::default()
        };

        // 1. HTTP succeeds -> app-server is not invoked.
        let mut app_called = false;
        let (u, from_session, at) = provider
            .fetch_with_tiers(
                now,
                |_| Ok(sample(10.0)),
                || {
                    app_called = true;
                    Ok(sample(20.0))
                },
            )
            .unwrap();
        assert!(!app_called);
        assert!(!from_session);
        assert_eq!(at, now);
        assert_eq!(u.five_hour.unwrap().used_pct, 10.0);

        // 2. HTTP fails transiently, app-server succeeds -> live reading (from_session = false).
        let (u, from_session, at) = provider
            .fetch_with_tiers(
                now,
                |_| Err(FetchError::Network("http down".into())),
                || Ok(sample(20.0)),
            )
            .unwrap();
        assert!(!from_session);
        assert_eq!(at, now);
        assert_eq!(u.five_hour.unwrap().used_pct, 20.0);

        // 3. HTTP and app-server both fail -> falls back to session logs (from_session = true).
        let (u, from_session, at) = provider
            .fetch_with_tiers(
                now,
                |_| Err(FetchError::BadResponse("bad http".into())),
                || Err(FetchError::Network("app-server failed".into())),
            )
            .unwrap();
        assert!(from_session);
        assert_eq!(at, parse_rfc3339("2026-10-26T15:56:19Z").unwrap());
        assert_eq!(u.five_hour.unwrap().used_pct, 44.0);

        // 4. Auth errors never fall back to app-server or session logs.
        let mut app_called = false;
        let err = provider
            .fetch_with_tiers(
                now,
                |_| Err(FetchError::Expired),
                || {
                    app_called = true;
                    Ok(sample(20.0))
                },
            )
            .unwrap_err();
        assert!(!app_called);
        assert_eq!(err, FetchError::Expired);
    }

    fn live_sample() -> ProviderUsage {
        ProviderUsage {
            five_hour: Some(WindowUsage {
                used_pct: 10.0,
                resets_at: Some(1793385254),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn app_server_is_skipped_while_backing_off() {
        let dir = sessions_dir("backoff", SESSION);
        let provider = CodexProvider::new(dir.parent().unwrap().to_path_buf());
        let calls = Cell::new(0);
        let http_down = |_: i64| Err(FetchError::Network("http down".into()));
        let app_fails = || {
            calls.set(calls.get() + 1);
            Err(FetchError::Network("app-server failed".into()))
        };
        let fetch_at = |now: i64| provider.fetch_with_tiers(now, http_down, app_fails);
        let t0 = SESSION_NOW;

        let (_, from_session, _) = fetch_at(t0).unwrap();
        assert!(from_session);
        assert_eq!(calls.get(), 1);

        fetch_at(t0 + 5 * 60 - 1).unwrap();
        assert_eq!(calls.get(), 1, "skipped inside the first 5 min");

        let t1 = t0 + 5 * 60;
        fetch_at(t1).unwrap();
        assert_eq!(calls.get(), 2);
        fetch_at(t1 + 10 * 60 - 1).unwrap();
        assert_eq!(calls.get(), 2, "second failure doubles the wait");

        provider
            .fetch_with_tiers(t1 + 60, |_| Ok(live_sample()), app_fails)
            .unwrap();
        fetch_at(t1 + 61).unwrap();
        assert_eq!(calls.get(), 3, "an HTTP success resets the back-off");
    }

    #[test]
    fn offline_http_error_skips_the_app_server_without_touching_the_backoff() {
        let dir = sessions_dir("offline", SESSION);
        let provider = CodexProvider::new(dir.parent().unwrap().to_path_buf());
        let calls = Cell::new(0);
        let offline = |_: i64| Err(FetchError::Offline("host not found".into()));
        let app_fails = || {
            calls.set(calls.get() + 1);
            Err(FetchError::Network("app-server failed".into()))
        };
        let t0 = SESSION_NOW;

        let (_, from_session, _) = provider.fetch_with_tiers(t0, offline, app_fails).unwrap();
        assert!(from_session, "still falls back to the session logs");
        assert_eq!(calls.get(), 0, "no app-server spawn while offline");

        // Not counted as an app-server failure: the next transient error tries it right away.
        let http_down = |_: i64| Err(FetchError::Network("timed out".into()));
        provider
            .fetch_with_tiers(t0 + 1, http_down, app_fails)
            .unwrap();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn offline_without_session_logs_returns_the_offline_error() {
        let home =
            std::env::temp_dir().join(format!("wuw-codex-{}-offline-empty", std::process::id()));
        let provider = CodexProvider::new(home);
        let err = provider
            .fetch_with_tiers(
                SESSION_NOW,
                |_| Err(FetchError::Offline("could not connect".into())),
                || panic!("app-server must not run while offline"),
            )
            .unwrap_err();
        assert_eq!(err, FetchError::Offline("could not connect".into()));
    }

    #[test]
    fn app_server_success_resets_the_backoff() {
        let dir = sessions_dir("backoff-reset", SESSION);
        let provider = CodexProvider::new(dir.parent().unwrap().to_path_buf());
        let calls = Cell::new(0);
        let http_down = |_: i64| Err(FetchError::Network("http down".into()));
        let app_fails = || {
            calls.set(calls.get() + 1);
            Err(FetchError::Network("app-server failed".into()))
        };
        let t0 = SESSION_NOW;

        provider.fetch_with_tiers(t0, http_down, app_fails).unwrap();
        let t1 = t0 + 5 * 60;
        let (_, from_session, _) = provider
            .fetch_with_tiers(t1, http_down, || Ok(live_sample()))
            .unwrap();
        assert!(!from_session);

        provider
            .fetch_with_tiers(t1 + 1, http_down, app_fails)
            .unwrap();
        assert_eq!(calls.get(), 2);
        provider
            .fetch_with_tiers(t1 + 1 + 5 * 60, http_down, app_fails)
            .unwrap();
        assert_eq!(calls.get(), 3, "back to a 5 min wait, not 20");
    }

    #[test]
    #[ignore = "calls the real endpoint with the local login"]
    fn live_codex() {
        let p = CodexProvider::new(CodexProvider::default_home());
        let u = crate::limits::Provider::fetch(&p).expect("live fetch");
        assert!(u.five_hour.is_some() || u.weekly.is_some());
    }

    #[test]
    #[ignore = "runs the locally installed Codex CLI app-server"]
    fn live_codex_app_server() {
        let p = CodexProvider::new(CodexProvider::default_home());
        let u = p.fetch_app_server().expect("live app-server fetch");
        assert!(u.five_hour.is_some() || u.weekly.is_some());
    }
}
