//! Crash-safe file writes for the switcher.
//!
//! A write goes to a temp file next to the target and is then renamed over it, so a reader
//! (Claude Code watches its files) never sees a truncated file. Windows Defender and the search
//! indexer briefly open fresh files, which makes the rename fail with a sharing violation; those
//! failures are retried a few times before giving up.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const RETRIES: u32 = 10;
const PAUSE: Duration = Duration::from_millis(50);

/// Writes `bytes` to `path` atomically.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(
        path,
        bytes,
        |from, to| std::fs::rename(from, to),
        RETRIES,
        PAUSE,
    )
}

/// [`atomic_write`] with an injectable rename, for tests.
pub fn atomic_write_with(
    path: &Path,
    bytes: &[u8],
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    retries: u32,
    pause: Duration,
) -> io::Result<()> {
    let tmp = temp_sibling(path);
    std::fs::write(&tmp, bytes)?;
    let mut attempt = 0;
    loop {
        match rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) if is_transient(&e) && attempt < retries => {
                attempt += 1;
                std::thread::sleep(pause);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
}

/// Access denied, sharing violation and lock violation: what a scanner holding the file
/// open produces.
fn is_transient(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

fn temp_sibling(path: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".cct-tmp-{}-{nanos}", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cct-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sharing_violation() -> io::Error {
        io::Error::from_raw_os_error(32)
    }

    #[test]
    fn writes_and_replaces_without_leaving_temp_files() {
        let dir = temp_dir("fsio");
        let p = dir.join("a.json");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn retries_a_transient_sharing_violation() {
        let dir = temp_dir("fsio-retry");
        let p = dir.join("a.json");
        let mut calls = 0;
        atomic_write_with(
            &p,
            b"x",
            |from, to| {
                calls += 1;
                if calls < 3 {
                    Err(sharing_violation())
                } else {
                    std::fs::rename(from, to)
                }
            },
            5,
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
    }

    #[test]
    fn gives_up_after_bounded_retries_and_keeps_the_original() {
        let dir = temp_dir("fsio-giveup");
        let p = dir.join("a.json");
        std::fs::write(&p, b"original").unwrap();
        let mut calls = 0;
        let err = atomic_write_with(
            &p,
            b"new",
            |_, _| {
                calls += 1;
                Err(sharing_violation())
            },
            3,
            Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(32));
        assert_eq!(calls, 4);
        assert_eq!(std::fs::read(&p).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "temp file removed"
        );
    }

    #[test]
    fn a_non_transient_error_is_not_retried() {
        let dir = temp_dir("fsio-fatal");
        let mut calls = 0;
        let _ = atomic_write_with(
            &dir.join("a"),
            b"x",
            |_, _| {
                calls += 1;
                Err(io::Error::from_raw_os_error(2))
            },
            5,
            Duration::ZERO,
        );
        assert_eq!(calls, 1);
    }
}
