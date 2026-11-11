//! *Check for updates* (#117): compares the running version with the latest GitHub release.
//!
//! Only when the user clicks the button in Settings: never at startup, on a timer or in the
//! background. One request to the public releases API through the usual client (HTTPS only, no
//! redirects, certificate checks on, 10 s), with no token and nothing about the user.

use crate::limits::FetchError;
use crate::limits::http::get_json;
use std::cmp::Ordering;

pub const LATEST_API: &str =
    "https://api.github.com/repos/chiarorosa/claude-codex-taskbar/releases/latest";
/// Opened in the browser when a newer version is out.
pub const RELEASE_PAGE: &str = "https://github.com/chiarorosa/claude-codex-taskbar/releases/latest";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    /// The newer version, as published (`0.5.0`, without the `v`).
    Available(String),
    /// Short, header-free reason, like the tooltip's.
    Failed(String),
}

/// `major.minor.patch`, and whether it carries a pre-release tag (`-rc.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Version {
    core: (u64, u64, u64),
    pre: bool,
}

/// `0.4.3`, `v0.4.3` or `v0.5.0-rc.1`; anything else is `None`.
fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let (core, pre) = match text.split_once('-') {
        Some((core, tag)) if !tag.is_empty() => (core, true),
        Some(_) => return None,
        None => (text, false),
    };
    let mut parts = core.split('.');
    let mut num = || -> Option<u64> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let core = (num()?, num()?, num()?);
    if parts.next().is_some() {
        return None;
    }
    Some(Version { core, pre })
}

/// A pre-release comes before the release with the same numbers; two pre-releases of the
/// same numbers count as equal (the tag is not compared).
fn compare(a: Version, b: Version) -> Ordering {
    a.core.cmp(&b.core).then(b.pre.cmp(&a.pre))
}

fn reason(err: &FetchError) -> String {
    match err {
        FetchError::Network(msg) | FetchError::Offline(msg) | FetchError::BadResponse(msg) => {
            msg.clone()
        }
        // 401/403 from GitHub: usually its hourly limit for anonymous requests.
        FetchError::Expired => "HTTP 403".into(),
        FetchError::RateLimited(_) => "HTTP 429".into(),
        FetchError::NoLogin | FetchError::NoSubscription => "unexpected error".into(),
    }
}

/// What the releases API answered, against the running version.
fn outcome(reply: Result<serde_json::Value, FetchError>, current: &str) -> Outcome {
    let release = match reply {
        Ok(release) => release,
        Err(err) => return Outcome::Failed(reason(&err)),
    };
    let Some(tag) = release["tag_name"].as_str() else {
        return Outcome::Failed("invalid release".into());
    };
    let (Some(latest), Some(running)) = (parse_version(tag), parse_version(current)) else {
        return Outcome::Failed("invalid version".into());
    };
    if compare(latest, running) == Ordering::Greater {
        Outcome::Available(tag.trim().trim_start_matches('v').to_string())
    } else {
        Outcome::UpToDate
    }
}

/// Asks GitHub once. Blocking (up to the client's 10 s): call it off the UI thread.
pub fn check() -> Outcome {
    outcome(get_json(LATEST_API, &[]), CURRENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tag(tag: &str) -> Result<serde_json::Value, FetchError> {
        Ok(json!({ "tag_name": tag, "name": "ignored" }))
    }

    #[test]
    fn newer_releases_are_offered_and_others_are_up_to_date() {
        let cases = [
            ("v0.4.3", Outcome::UpToDate),
            ("0.4.3", Outcome::UpToDate),
            ("v0.4.4", Outcome::Available("0.4.4".into())),
            ("v0.5.0", Outcome::Available("0.5.0".into())),
            ("v1.0.0", Outcome::Available("1.0.0".into())),
            ("v0.4.2", Outcome::UpToDate),
            ("v0.3.9", Outcome::UpToDate),
            // A pre-release of the next version is newer; of this one, older.
            ("v0.4.4-rc.1", Outcome::Available("0.4.4-rc.1".into())),
            ("v0.4.3-rc.1", Outcome::UpToDate),
            ("v0.4.10", Outcome::Available("0.4.10".into())),
        ];
        for (latest, want) in cases {
            assert_eq!(outcome(tag(latest), "0.4.3"), want, "{latest}");
        }
    }

    #[test]
    fn a_pre_release_build_sees_its_release() {
        assert_eq!(
            outcome(tag("v0.5.0"), "0.5.0-rc.1"),
            Outcome::Available("0.5.0".into())
        );
        assert_eq!(outcome(tag("v0.5.0-rc.2"), "0.5.0-rc.1"), Outcome::UpToDate);
    }

    #[test]
    fn garbage_is_an_error_never_up_to_date() {
        for bad in [
            "", "latest", "v1", "v1.2", "v1.2.3.4", "v1.2.x", "v-1.2.3", "v1.2.3-", "1..3",
        ] {
            assert_eq!(
                outcome(tag(bad), "0.4.3"),
                Outcome::Failed("invalid version".into()),
                "{bad:?}"
            );
        }
        for reply in [json!({}), json!({ "tag_name": 5 }), json!([])] {
            assert_eq!(
                outcome(Ok(reply), "0.4.3"),
                Outcome::Failed("invalid release".into())
            );
        }
    }

    #[test]
    fn errors_keep_a_short_reason() {
        let cases = [
            (
                FetchError::Offline("host not found".into()),
                "host not found",
            ),
            (FetchError::Network("timed out".into()), "timed out"),
            (FetchError::BadResponse("HTTP 404".into()), "HTTP 404"),
            (FetchError::Expired, "HTTP 403"),
            (FetchError::RateLimited(Some(1)), "HTTP 429"),
        ];
        for (err, want) in cases {
            assert_eq!(outcome(Err(err), "0.4.3"), Outcome::Failed(want.into()));
        }
    }

    #[test]
    fn the_running_version_parses() {
        assert!(parse_version(CURRENT).is_some(), "{CURRENT}");
    }

    #[test]
    #[ignore = "asks the real GitHub API"]
    fn live_check() {
        let got = check();
        assert!(!matches!(got, Outcome::Failed(_)), "{got:?}");
    }
}
