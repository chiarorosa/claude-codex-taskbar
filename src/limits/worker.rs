//! Background threads, one per provider, that fetch limits and hand results to the UI thread.

use super::ratelimit::RateLimit;
use super::{FetchError, ProviderUsage};
use crate::model::Kind;
use crate::timeutil::now_unix;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Fetches one provider at `now`: usage, whether it came from session logs, and its time.
pub type Fetcher = Box<dyn Fn(i64) -> Result<(ProviderUsage, bool, i64), FetchError> + Send>;

#[derive(Debug)]
pub struct LimitResult {
    pub kind: Kind,
    pub result: Result<(ProviderUsage, bool, i64), FetchError>,
}

/// Shared knobs the UI thread uses to steer the per-provider worker threads.
pub struct Control {
    interval_secs: AtomicU32,
    claude_on: AtomicBool,
    codex_on: AtomicBool,
    quit: AtomicBool,
    /// Bumped by `account_changed`, so that provider's thread drops a rate-limit wait that
    /// belonged to the previous login (#87).
    claude_login: AtomicU64,
    codex_login: AtomicU64,
    /// Bumped by every `wake`. Each provider thread remembers the last value it acted on,
    /// so one wake reaches every thread: a shared `bool` reset by the first thread to wake
    /// could be cleared before the other thread saw it (#89).
    wake_gen: Mutex<u64>,
    cv: Condvar,
}

impl Control {
    pub fn new(interval_secs: u32) -> Arc<Control> {
        Arc::new(Control {
            interval_secs: AtomicU32::new(interval_secs),
            claude_on: AtomicBool::new(false),
            codex_on: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            claude_login: AtomicU64::new(0),
            codex_login: AtomicU64::new(0),
            wake_gen: Mutex::new(0),
            cv: Condvar::new(),
        })
    }

    fn flag(&self, kind: Kind) -> Option<&AtomicBool> {
        match kind {
            Kind::Claude => Some(&self.claude_on),
            Kind::Codex => Some(&self.codex_on),
            Kind::Hardware => None,
        }
    }

    pub fn set_enabled(&self, kind: Kind, on: bool) {
        if let Some(flag) = self.flag(kind) {
            flag.store(on, Ordering::Relaxed);
        }
    }

    pub fn is_enabled(&self, kind: Kind) -> bool {
        self.flag(kind).is_some_and(|f| f.load(Ordering::Relaxed))
    }

    pub fn set_interval(&self, secs: u32) {
        self.interval_secs.store(secs, Ordering::Relaxed);
    }

    /// Fetch now instead of waiting for the interval; wakes every provider thread.
    pub fn wake(&self) {
        let mut wake_gen = self.wake_gen.lock().unwrap_or_else(|e| e.into_inner());
        *wake_gen = wake_gen.wrapping_add(1);
        self.cv.notify_all();
    }

    pub fn stop(&self) {
        self.quit.store(true, Ordering::Relaxed);
        self.wake();
    }

    /// A different login is live for `kind` (a Claude account switch): fetch it now, even
    /// while the previous login is waiting out a rate limit.
    pub fn account_changed(&self, kind: Kind) {
        if let Some(login) = self.login(kind) {
            login.fetch_add(1, Ordering::Relaxed);
        }
        self.wake();
    }

    fn login(&self, kind: Kind) -> Option<&AtomicU64> {
        match kind {
            Kind::Claude => Some(&self.claude_login),
            Kind::Codex => Some(&self.codex_login),
            Kind::Hardware => None,
        }
    }

    fn login_gen(&self, kind: Kind) -> u64 {
        self.login(kind).map_or(0, |l| l.load(Ordering::Relaxed))
    }

    fn interval(&self) -> u32 {
        self.interval_secs.load(Ordering::Relaxed)
    }

    fn quitting(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }

    /// The wake generation right now, so a thread only reacts to wakes that come later.
    fn current_gen(&self) -> u64 {
        *self.wake_gen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sleeps for the interval, or until a wake newer than `seen` (or `stop`). A wake that
    /// arrived while the caller was still fetching returns at once, as it did with a single
    /// worker; several such wakes count as one. Updates `seen` to the generation acted on.
    fn wait(&self, seen: &mut u64) {
        let timeout = Duration::from_secs(self.interval_secs.load(Ordering::Relaxed).into());
        let guard = self.wake_gen.lock().unwrap_or_else(|e| e.into_inner());
        let (wake_gen, _) = self
            .cv
            .wait_timeout_while(guard, timeout, |g| *g == *seen && !self.quitting())
            .unwrap_or_else(|e| e.into_inner());
        *seen = *wake_gen;
    }
}

/// The worker threads, one per provider. Dropping it detaches them, as with a
/// [`JoinHandle`]; call [`Control::stop`] and then [`Workers::join`] to end them.
pub struct Workers(Vec<JoinHandle<()>>);

impl Workers {
    /// Waits for every provider thread to finish; the first panic, if any, is returned
    /// after all threads have been joined.
    pub fn join(self) -> std::thread::Result<()> {
        let mut outcome = Ok(());
        for handle in self.0 {
            let joined = handle.join();
            if outcome.is_ok() {
                outcome = joined;
            }
        }
        outcome
    }
}

/// Fetches every enabled provider right away, then every interval or on `wake`. Each
/// provider runs on its own thread with its own wait, so a slow fetch (for example the
/// Codex app-server fallback waiting for its timeout) never holds up the other provider
/// (#89). Each provider is still fetched exactly as often as with a single worker, and a
/// disabled provider's thread sleeps without fetching.
pub fn spawn_with(
    control: Arc<Control>,
    fetchers: Vec<(Kind, Fetcher)>,
    post: impl Fn(LimitResult) + Send + Sync + 'static,
) -> Workers {
    let post = Arc::new(post);
    // Taken before any thread starts, so every wake after this call is honored by every
    // thread, even one the OS has not scheduled yet (as with the single worker).
    let start_gen = control.current_gen();
    let handles = fetchers
        .into_iter()
        .map(|(kind, fetch)| {
            let control = control.clone();
            let post = post.clone();
            std::thread::spawn(move || {
                let mut seen = start_gen;
                let mut limit = RateLimit::default();
                let mut login = control.login_gen(kind);
                while !control.quitting() {
                    if control.login_gen(kind) != login {
                        login = control.login_gen(kind);
                        limit = RateLimit::default();
                    }
                    let now = now_unix();
                    // While rate-limited, skip the provider entirely: the error already on
                    // screen says when the next try is (#87).
                    if control.is_enabled(kind) && limit.allows(now) {
                        let mut result = fetch(now);
                        limit.record(&mut result, now, control.interval().into());
                        post(LimitResult { kind, result });
                    }
                    control.wait(&mut seen);
                }
            })
        })
        .collect();
    Workers(handles)
}

/// Like [`spawn_with`], but results wait in a queue the UI drains when it can; `notify`
/// only signals that something arrived, so no data travels inside window messages.
pub fn spawn_queued(
    control: Arc<Control>,
    fetchers: Vec<(Kind, Fetcher)>,
    notify: impl Fn() + Send + Sync + 'static,
) -> (Workers, Receiver<LimitResult>) {
    let (tx, rx) = channel();
    let handle = spawn_with(control, fetchers, move |result| {
        if tx.send(result).is_ok() {
            notify();
        }
    });
    (handle, rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{FetchError, ProviderUsage};
    use crate::model::Kind;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    fn fake(pct: f32) -> Fetcher {
        Box::new(move |now| {
            Ok((
                ProviderUsage {
                    five_hour: Some(crate::limits::WindowUsage {
                        used_pct: pct,
                        resets_at: None,
                    }),
                    weekly: None,
                    ..Default::default()
                },
                false,
                now,
            ))
        })
    }

    #[test]
    fn worker_posts_and_respects_wake() {
        let control = Control::new(3600);
        control.set_enabled(Kind::Claude, true);
        let (tx, rx) = mpsc::channel();
        let handle = spawn_with(
            control.clone(),
            vec![(Kind::Claude, fake(10.0))],
            move |r| {
                tx.send(r).unwrap();
            },
        );
        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("initial fetch");
        assert_eq!(first.kind, Kind::Claude);
        assert!(first.result.is_ok());
        control.wake();
        rx.recv_timeout(Duration::from_secs(2))
            .expect("fetch after wake");
        control.stop();
        handle.join().unwrap();
    }

    #[test]
    fn disabled_provider_is_not_fetched() {
        let control = Control::new(3600);
        control.set_enabled(Kind::Codex, false);
        control.set_enabled(Kind::Claude, true);
        let failing: Fetcher = Box::new(|_| Err(FetchError::NoLogin));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_with(
            control.clone(),
            vec![(Kind::Codex, failing), (Kind::Claude, fake(1.0))],
            move |r| tx.send(r).unwrap(),
        );
        let r = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(r.kind, Kind::Claude);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        control.stop();
        handle.join().unwrap();
    }

    /// Counts its calls, then succeeds.
    fn counting(calls: Arc<AtomicUsize>) -> Fetcher {
        let ok = fake(1.0);
        Box::new(move |now| {
            calls.fetch_add(1, Ordering::SeqCst);
            ok(now)
        })
    }

    /// Blocks until the test sends on (or drops) the other end of `gate`, then succeeds.
    fn gated(gate: mpsc::Receiver<()>) -> Fetcher {
        let ok = fake(5.0);
        Box::new(move |now| {
            let _ = gate.recv();
            ok(now)
        })
    }

    #[test]
    fn each_wake_fetches_each_enabled_provider_once() {
        let control = Control::new(3600);
        control.set_enabled(Kind::Claude, true);
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_with(
            control.clone(),
            vec![
                (Kind::Claude, counting(claude_calls.clone())),
                (Kind::Codex, counting(codex_calls.clone())),
            ],
            move |r| {
                let _ = tx.send(r);
            },
        );
        let next = || {
            rx.recv_timeout(Duration::from_secs(2))
                .expect("result")
                .kind
        };
        assert_eq!(next(), Kind::Claude);
        for _ in 0..2 {
            control.wake();
            assert_eq!(next(), Kind::Claude);
        }
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(claude_calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            codex_calls.load(Ordering::SeqCst),
            0,
            "a disabled provider is never fetched"
        );

        // Enabling Codex and waking fetches each provider exactly once more: one wake
        // reaches both threads.
        control.set_enabled(Kind::Codex, true);
        control.wake();
        let kinds = [next(), next()];
        assert!(kinds.contains(&Kind::Claude) && kinds.contains(&Kind::Codex));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(claude_calls.load(Ordering::SeqCst), 4);
        assert_eq!(codex_calls.load(Ordering::SeqCst), 1);
        control.stop();
        handle.join().unwrap();
    }

    /// `blocked` is stuck mid-fetch (like Codex waiting for the app-server timeout) while
    /// `free` keeps answering, including the refresh a wake asks for (#89).
    fn blocked_provider_does_not_delay(blocked: Kind, free: Kind) {
        let control = Control::new(3600);
        control.set_enabled(blocked, true);
        control.set_enabled(free, true);
        let (release, gate) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let handle = spawn_with(
            control.clone(),
            vec![(blocked, gated(gate)), (free, fake(1.0))],
            move |r| {
                let _ = tx.send(r);
            },
        );
        let next = |what: &str| rx.recv_timeout(Duration::from_secs(2)).expect(what).kind;

        assert_eq!(next("first fetch of the free provider"), free);
        control.wake();
        assert_eq!(next("free provider refreshed on wake"), free);
        // One fetch per wake, and nothing from the provider that is still blocked.
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());

        release.send(()).unwrap();
        assert_eq!(next("blocked provider's result once released"), blocked);
        // The wake that arrived mid-fetch is honored once afterwards, as with one worker.
        release.send(()).unwrap();
        assert_eq!(
            next("blocked provider's refresh for the pending wake"),
            blocked
        );
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());

        control.stop();
        drop(release);
        handle.join().unwrap();
    }

    #[test]
    fn blocked_codex_does_not_delay_claude() {
        blocked_provider_does_not_delay(Kind::Codex, Kind::Claude);
    }

    #[test]
    fn blocked_claude_does_not_delay_codex() {
        blocked_provider_does_not_delay(Kind::Claude, Kind::Codex);
    }

    #[test]
    fn a_rate_limited_provider_waits_even_on_wake_until_its_login_changes() {
        let control = Control::new(3600);
        control.set_enabled(Kind::Claude, true);
        control.set_enabled(Kind::Codex, true);
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let calls = claude_calls.clone();
        let limited: Fetcher = Box::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(FetchError::RateLimited(None))
        });
        let (tx, rx) = mpsc::channel();
        let started = now_unix();
        let handle = spawn_with(
            control.clone(),
            vec![(Kind::Claude, limited), (Kind::Codex, fake(1.0))],
            move |r| {
                let _ = tx.send(r);
            },
        );
        let mut first = Vec::new();
        for _ in 0..2 {
            first.push(
                rx.recv_timeout(Duration::from_secs(2))
                    .expect("first fetch"),
            );
        }
        let claude = first.iter().find(|r| r.kind == Kind::Claude).unwrap();
        // No Retry-After: one interval, and the posted error says until when.
        let Err(FetchError::RateLimited(Some(until))) = claude.result else {
            panic!("{:?}", claude.result);
        };
        assert!(
            (started + 3600..=started + 3602).contains(&until),
            "{until}"
        );

        // Refresh now still refreshes Codex, but leaves the rate-limited Claude alone.
        control.wake();
        let r = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("codex refresh");
        assert_eq!(r.kind, Kind::Codex);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(claude_calls.load(Ordering::SeqCst), 1);

        // A Claude account switch is a different login: fetched right away. Codex keeps its
        // own schedule apart from the wake.
        control.account_changed(Kind::Claude);
        let mut kinds = Vec::new();
        for _ in 0..2 {
            kinds.push(
                rx.recv_timeout(Duration::from_secs(2))
                    .expect("after switch")
                    .kind,
            );
        }
        assert!(kinds.contains(&Kind::Claude) && kinds.contains(&Kind::Codex));
        assert_eq!(claude_calls.load(Ordering::SeqCst), 2);
        control.stop();
        handle.join().unwrap();
    }

    #[test]
    fn stop_ends_every_provider_thread() {
        let control = Control::new(3600);
        control.set_enabled(Kind::Claude, true);
        let (tx, rx) = mpsc::channel();
        let handle = spawn_with(
            control.clone(),
            vec![(Kind::Claude, fake(1.0)), (Kind::Codex, fake(2.0))],
            move |r| {
                let _ = tx.send(r);
            },
        );
        rx.recv_timeout(Duration::from_secs(2))
            .expect("initial fetch");
        control.stop();
        handle.join().unwrap();
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use crate::limits::{ProviderUsage, WindowUsage};
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    #[test]
    fn results_queue_until_drained() {
        let control = Control::new(3600);
        control.set_enabled(Kind::Claude, true);
        control.set_enabled(Kind::Codex, true);
        let ok = |pct: f32| -> Fetcher {
            Box::new(move |now| {
                Ok((
                    ProviderUsage {
                        five_hour: Some(WindowUsage {
                            used_pct: pct,
                            resets_at: None,
                        }),
                        weekly: None,
                        ..Default::default()
                    },
                    false,
                    now,
                ))
            })
        };
        let notified = Arc::new(AtomicUsize::new(0));
        let n = notified.clone();
        let (handle, inbox) = spawn_queued(
            control.clone(),
            vec![(Kind::Claude, ok(1.0)), (Kind::Codex, ok(2.0))],
            move || {
                n.fetch_add(1, Ordering::SeqCst);
            },
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while notified.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // Nothing was consumed while "busy"; both results are still waiting. The providers
        // run on their own threads, so either may arrive first.
        let kinds: Vec<Kind> = inbox.try_iter().map(|r| r.kind).collect();
        assert_eq!(kinds.len(), 2);
        assert!(kinds.contains(&Kind::Claude) && kinds.contains(&Kind::Codex));
        control.stop();
        handle.join().unwrap();
    }
}
