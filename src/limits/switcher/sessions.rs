//! Running Claude Code sessions, from the records it keeps in `~/.claude/sessions/<pid>.json`.
//!
//! Used only to ask before switching while a task is running (#53). Switching never waits on
//! or blocks because of a session: the request in flight finishes on the old token and the
//! next one uses the new account.

use serde_json::Value;
use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Pids of live sessions with a task underway: `"busy"`, or `"waiting"`, which Claude Code
/// writes instead of `"busy"` while a turn waits on the user (a permission prompt, a
/// question, a sandbox request, a dialog). Answering it resumes the task on whatever login
/// is live, so it counts as running. Dead pids and unreadable records are ignored.
pub fn busy_sessions(records: &[String], is_alive: impl Fn(u32) -> bool) -> Vec<u32> {
    records
        .iter()
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .filter(|r| {
            matches!(
                r.get("status").and_then(Value::as_str),
                Some("busy" | "waiting")
            )
        })
        .filter_map(|r| r.get("pid").and_then(Value::as_u64))
        .filter_map(|pid| u32::try_from(pid).ok())
        .filter(|&pid| is_alive(pid))
        .collect()
}

/// The raw session records. A missing directory means no sessions.
pub fn read_session_records(home: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home.join(".claude").join("sessions")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect()
}

pub fn is_process_alive(pid: u32) -> bool {
    // SAFETY: the handle is checked before use and closed exactly once.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        alive
    }
}

/// Live Claude Code tasks right now, running or waiting on the user.
pub fn busy_now(home: &Path) -> Vec<u32> {
    busy_sessions(&read_session_records(home), is_process_alive)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pid: u32, status: &str) -> String {
        format!(r#"{{"pid":{pid},"sessionId":"s","cwd":"C:/x","status":"{status}"}}"#)
    }

    #[test]
    fn reports_live_sessions_that_are_running_or_waiting_on_the_user() {
        let records = vec![
            rec(10, "busy"),
            rec(11, "idle"),
            rec(12, "waiting"),
            rec(13, "busy"),
            rec(14, "waiting"),
            rec(15, "something-new"),
            "{torn".to_string(),
            r#"{"status":"busy"}"#.to_string(),
        ];
        assert_eq!(
            busy_sessions(&records, |pid| pid != 13 && pid != 14),
            vec![10, 12]
        );
    }

    #[test]
    fn a_task_paused_on_a_permission_prompt_counts_as_running() {
        // Claude Code writes "waiting" instead of "busy" while a turn waits for an answer.
        let record = r#"{"pid":20,"sessionId":"s","cwd":"C:/x","status":"waiting","waitingFor":"permission prompt"}"#;
        assert_eq!(busy_sessions(&[record.to_string()], |_| true), vec![20]);
    }

    #[test]
    fn a_missing_directory_means_no_sessions() {
        let home = crate::limits::switcher::fsio::temp_dir("sessions-none");
        assert!(read_session_records(&home).is_empty());
        assert!(busy_now(&home).is_empty());
    }

    #[test]
    fn this_process_is_alive_and_pid_zero_is_not_a_session() {
        assert!(is_process_alive(std::process::id()));
        assert!(!is_process_alive(u32::MAX - 1));
    }
}
