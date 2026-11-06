//! Claude Code's own advisory locks, held while the switcher touches its files (#54).
//!
//! Claude Code uses the npm `proper-lockfile` package: the lock is a **directory** whose
//! `mkdir` is the mutex, and its holder touches the directory's mtime every few seconds. A lock
//! whose mtime is older than its staleness window belongs to a dead holder and may be taken
//! over. Claude Code refreshes its OAuth token under `~/.claude/.oauth_refresh.lock`, then
//! `~/.claude.lock` (both stale after 60 s), and writes `~/.claude.json` under
//! `~/.claude.json.lock` (stale after 10 s). Holding all three while switching means a token
//! refresh never interleaves with a swap.

use std::fs::OpenOptions;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// Staleness of Claude Code's two credential-refresh locks.
pub const CREDENTIALS_STALE: Duration = Duration::from_secs(60);
/// Staleness of Claude Code's `~/.claude.json` lock.
pub const CONFIG_STALE: Duration = Duration::from_secs(10);
/// How often a held lock's mtime is refreshed (Claude Code uses 5 s; we go a little faster).
pub const TOUCH_EVERY: Duration = Duration::from_secs(3);
/// Default time to wait for each lock.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(9);
/// A takeover directory older than this belongs to a waiter that died mid-takeover.
const TAKEOVER_STALE: Duration = Duration::from_secs(5);

/// One lock directory and how long it may go untouched before it counts as abandoned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockSpec {
    pub dir: PathBuf,
    pub stale: Duration,
}

/// Claude Code's locks for the user profile `home`, in the order they must be taken.
pub fn claude_lock_specs(home: &Path) -> [LockSpec; 3] {
    let spec = |dir: PathBuf, stale| LockSpec { dir, stale };
    [
        spec(
            home.join(".claude").join(".oauth_refresh.lock"),
            CREDENTIALS_STALE,
        ),
        spec(home.join(".claude.lock"), CREDENTIALS_STALE),
        spec(home.join(".claude.json.lock"), CONFIG_STALE),
    ]
}

#[derive(Debug, PartialEq, Eq)]
pub enum LockError {
    /// The lock stayed held by a live holder for the whole timeout.
    Timeout(PathBuf),
    Io(String),
}

/// A held lock directory. Dropping it stops the toucher and removes the directory.
#[derive(Debug)]
pub struct DirLock {
    dir: PathBuf,
    stop: Option<Sender<()>>,
    toucher: Option<JoinHandle<()>>,
}

impl DirLock {
    /// Takes the lock, waiting up to `timeout` and taking over a stale one.
    pub fn acquire(
        spec: &LockSpec,
        timeout: Duration,
        touch_every: Duration,
    ) -> Result<Self, LockError> {
        if let Some(parent) = spec.dir.parent() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let start = Instant::now();
        let mut rng = Jitter::seeded();
        loop {
            match std::fs::create_dir(&spec.dir) {
                Ok(()) => break,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_err(e)),
            }
            if start.elapsed() > timeout {
                return Err(LockError::Timeout(spec.dir.clone()));
            }
            match age_of(&spec.dir) {
                // Released between our mkdir and stat: retry at once.
                None => continue,
                Some(age) if age > spec.stale => {
                    if !remove_if_still_stale(&spec.dir, spec.stale) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    continue;
                }
                Some(_) => std::thread::sleep(Duration::from_millis(250 + rng.next() % 250)),
            }
        }
        let (stop, rx) = mpsc::channel::<()>();
        let dir = spec.dir.clone();
        let toucher = std::thread::spawn(move || {
            while let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(touch_every) {
                if set_dir_mtime(&dir, SystemTime::now()).is_err() {
                    return; // lock removed under us; nothing left to keep alive
                }
            }
        });
        Ok(DirLock {
            dir: spec.dir.clone(),
            stop: Some(stop),
            toucher: Some(toucher),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        drop(self.stop.take()); // disconnects the channel, waking the toucher
        if let Some(t) = self.toucher.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_dir(&self.dir);
    }
}

fn io_err(e: io::Error) -> LockError {
    LockError::Io(e.to_string())
}

/// Time since `dir` was last touched, or `None` if it is gone.
fn age_of(dir: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(dir).ok()?.modified().ok()?;
    Some(
        SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default(),
    )
}

/// Removes `dir` if it is still stale, serialized through a sibling `.takeover` directory so
/// two waiters that saw the same dead holder can't both act (the second would delete the
/// first one's fresh lock). True if this call removed it.
fn remove_if_still_stale(dir: &Path, stale: Duration) -> bool {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".takeover");
    let takeover = dir.with_file_name(name);
    if std::fs::create_dir(&takeover).is_err() {
        if age_of(&takeover).is_some_and(|a| a > TAKEOVER_STALE) {
            let _ = std::fs::remove_dir(&takeover); // its owner died mid-takeover
        }
        return false;
    }
    // Re-check inside: the waiter before us may already have replaced the dead lock.
    let removed = age_of(dir).is_some_and(|a| a > stale) && std::fs::remove_dir(dir).is_ok();
    let _ = std::fs::remove_dir(&takeover);
    removed
}

/// Sets a directory's modification time (opening a directory needs backup semantics).
pub(crate) fn set_dir_mtime(dir: &Path, to: SystemTime) -> io::Result<()> {
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)?
        .set_modified(to)
}

/// Tiny xorshift for retry jitter; no crate needed.
struct Jitter(u64);

impl Jitter {
    fn seeded() -> Self {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Jitter(nanos | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// All of Claude Code's locks, held together. Released in reverse order.
#[derive(Debug)]
pub struct ClaudeLocks {
    held: Vec<DirLock>,
}

impl ClaudeLocks {
    /// Takes every lock in Claude Code's order. On failure the ones already taken are released.
    pub fn acquire_all(home: &Path, timeout: Duration) -> Result<Self, LockError> {
        let mut locks = ClaudeLocks { held: Vec::new() };
        for spec in claude_lock_specs(home) {
            locks
                .held
                .push(DirLock::acquire(&spec, timeout, TOUCH_EVERY)?);
        }
        Ok(locks)
    }

    /// Lock directories currently held, in acquisition order.
    pub fn dirs(&self) -> Vec<&Path> {
        self.held.iter().map(DirLock::dir).collect()
    }
}

impl Drop for ClaudeLocks {
    fn drop(&mut self) {
        while let Some(lock) = self.held.pop() {
            drop(lock);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::time::{Instant, SystemTime};

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cct-lock-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    fn spec(home: &Path, stale_secs: u64) -> LockSpec {
        LockSpec {
            dir: home.join("x.lock"),
            stale: Duration::from_secs(stale_secs),
        }
    }

    fn age_dir(dir: &Path, by: Duration) {
        set_dir_mtime(dir, SystemTime::now() - by).unwrap();
    }

    #[test]
    fn specs_follow_claude_codes_order_and_staleness() {
        let home = Path::new(r"C:\Users\u");
        let specs = claude_lock_specs(home);
        assert_eq!(
            specs[0].dir,
            home.join(".claude").join(".oauth_refresh.lock")
        );
        assert_eq!(specs[1].dir, home.join(".claude.lock"));
        assert_eq!(specs[2].dir, home.join(".claude.json.lock"));
        assert_eq!(specs[0].stale, CREDENTIALS_STALE);
        assert_eq!(specs[1].stale, CREDENTIALS_STALE);
        assert_eq!(specs[2].stale, CONFIG_STALE);
    }

    #[test]
    fn acquire_creates_the_directory_and_drop_removes_it() {
        let home = temp_home("basic");
        let s = spec(&home, 60);
        let lock = DirLock::acquire(&s, Duration::from_millis(200), TOUCH_EVERY).unwrap();
        assert!(s.dir.is_dir());
        drop(lock);
        assert!(!s.dir.exists());
    }

    #[test]
    fn a_fresh_lock_blocks_until_timeout() {
        let home = temp_home("fresh");
        let s = spec(&home, 60);
        std::fs::create_dir(&s.dir).unwrap();
        let start = Instant::now();
        let err = DirLock::acquire(&s, Duration::from_millis(600), TOUCH_EVERY).unwrap_err();
        assert_eq!(err, LockError::Timeout(s.dir.clone()));
        assert!(start.elapsed() >= Duration::from_millis(600));
        assert!(s.dir.is_dir(), "a live holder's lock must never be removed");
    }

    #[test]
    fn a_lock_just_inside_its_window_is_not_taken() {
        let home = temp_home("young");
        let s = spec(&home, 60);
        std::fs::create_dir(&s.dir).unwrap();
        age_dir(&s.dir, Duration::from_secs(50));
        assert!(DirLock::acquire(&s, Duration::from_millis(300), TOUCH_EVERY).is_err());
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let home = temp_home("stale");
        let s = spec(&home, 60);
        std::fs::create_dir(&s.dir).unwrap();
        age_dir(&s.dir, Duration::from_secs(120));
        let lock = DirLock::acquire(&s, Duration::from_secs(2), TOUCH_EVERY).unwrap();
        let age = SystemTime::now()
            .duration_since(std::fs::metadata(&s.dir).unwrap().modified().unwrap())
            .unwrap_or_default();
        assert!(
            age < Duration::from_secs(5),
            "taken-over lock must look fresh"
        );
        drop(lock);
    }

    #[test]
    fn two_waiters_on_one_stale_lock_never_hold_it_together() {
        let home = temp_home("race");
        let s = spec(&home, 60);
        std::fs::create_dir(&s.dir).unwrap();
        age_dir(&s.dir, Duration::from_secs(120));
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (s, barrier) = (s.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let lock = DirLock::acquire(&s, Duration::from_secs(5), TOUCH_EVERY).ok()?;
                    let held_from = Instant::now();
                    std::thread::sleep(Duration::from_millis(300));
                    let held_until = Instant::now();
                    drop(lock);
                    Some((held_from, held_until))
                })
            })
            .collect();
        // Both may win in turn (the second after the first releases); timing under a loaded
        // test run decides that. What must never happen is both holding it at the same time.
        let held: Vec<_> = handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect();
        assert!(!held.is_empty(), "the stale lock was never taken over");
        if let [(a_from, a_until), (b_from, b_until)] = held[..] {
            assert!(
                a_until <= b_from || b_until <= a_from,
                "both waiters held the lock at the same time"
            );
        }
    }

    #[test]
    fn the_toucher_keeps_a_held_lock_fresh() {
        let home = temp_home("touch");
        let s = spec(&home, 60);
        let lock =
            DirLock::acquire(&s, Duration::from_millis(200), Duration::from_millis(100)).unwrap();
        age_dir(&s.dir, Duration::from_secs(40));
        std::thread::sleep(Duration::from_millis(400));
        let age = SystemTime::now()
            .duration_since(std::fs::metadata(&s.dir).unwrap().modified().unwrap())
            .unwrap_or_default();
        assert!(
            age < Duration::from_secs(5),
            "toucher did not refresh mtime: {age:?}"
        );
        drop(lock);
    }

    #[test]
    fn a_panic_while_holding_releases_the_lock() {
        let home = temp_home("panic");
        let s = spec(&home, 60);
        let s2 = s.clone();
        let result = std::panic::catch_unwind(move || {
            let _lock = DirLock::acquire(&s2, Duration::from_millis(200), TOUCH_EVERY).unwrap();
            panic!("boom");
        });
        assert!(result.is_err());
        assert!(!s.dir.exists());
    }

    #[test]
    fn acquire_all_takes_all_three_in_order_and_releases_them() {
        let home = temp_home("all");
        let locks = ClaudeLocks::acquire_all(&home, Duration::from_millis(500)).unwrap();
        let expected: Vec<PathBuf> = claude_lock_specs(&home)
            .into_iter()
            .map(|s| s.dir)
            .collect();
        assert_eq!(
            locks.dirs(),
            expected.iter().map(PathBuf::as_path).collect::<Vec<_>>()
        );
        assert!(expected.iter().all(|d| d.is_dir()));
        drop(locks);
        assert!(expected.iter().all(|d| !d.exists()));
    }

    #[test]
    fn acquire_all_releases_earlier_locks_when_a_later_one_times_out() {
        let home = temp_home("partial");
        let specs = claude_lock_specs(&home);
        std::fs::create_dir(&specs[2].dir).unwrap(); // Claude Code is writing ~/.claude.json
        let err = ClaudeLocks::acquire_all(&home, Duration::from_millis(400)).unwrap_err();
        assert_eq!(err, LockError::Timeout(specs[2].dir.clone()));
        assert!(!specs[0].dir.exists());
        assert!(!specs[1].dir.exists());
        assert!(specs[2].dir.is_dir());
    }
}
