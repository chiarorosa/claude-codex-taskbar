use super::*;
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct FakeNotificationSink {
    calls: RefCell<Vec<BalloonCommand>>,
    removed: RefCell<Vec<u16>>,
    payloads: RefCell<Vec<(u16, String, String)>>,
    fail_at: Option<BalloonCommand>,
    revoke_at: Option<BalloonCommand>,
    blocked: bool,
    valid: Cell<bool>,
}

impl NotificationSink for FakeNotificationSink {
    fn accepts(&self) -> bool {
        !self.blocked
    }
    fn send(&self, command: BalloonCommand, id: u16, title: &str, body: &str) -> bool {
        self.calls.borrow_mut().push(command);
        self.payloads
            .borrow_mut()
            .push((id, title.into(), body.into()));
        if self.revoke_at == Some(command) {
            self.valid.set(false);
        }
        self.fail_at != Some(command)
    }
    fn remove(&self, id: u16) {
        self.removed.borrow_mut().push(id);
    }
}

fn sink() -> FakeNotificationSink {
    FakeNotificationSink {
        valid: Cell::new(true),
        ..Default::default()
    }
}

#[test]
fn shell_protocol_sends_owned_payload_then_arms_cleanup() {
    let shell = sink();
    assert!(send_balloon(&shell, 7, "Title", "Body", || shell
        .valid
        .get()));
    assert_eq!(
        *shell.calls.borrow(),
        [
            BalloonCommand::Add,
            BalloonCommand::Version,
            BalloonCommand::Show,
            BalloonCommand::ArmCleanup
        ]
    );
    assert_eq!(
        *shell.payloads.borrow(),
        vec![(7, "Title".into(), "Body".into()); 4]
    );
    assert_eq!(*shell.removed.borrow(), Vec::<u16>::new());
}

#[test]
fn blocked_or_revoked_delivery_never_adds_an_icon() {
    for (blocked, valid) in [(true, true), (false, false)] {
        let shell = FakeNotificationSink {
            blocked,
            valid: Cell::new(valid),
            ..Default::default()
        };
        assert!(!send_balloon(&shell, 7, "Title", "Body", || shell
            .valid
            .get()));
        assert_eq!(*shell.calls.borrow(), vec![]);
        assert_eq!(*shell.removed.borrow(), [7]);
    }
}

#[test]
fn failures_and_reentrant_opt_out_stop_every_remaining_operation() {
    let operations = [
        BalloonCommand::Add,
        BalloonCommand::Version,
        BalloonCommand::Show,
        BalloonCommand::ArmCleanup,
    ];
    for (index, op) in operations.into_iter().enumerate() {
        for revoke in [false, true] {
            let shell = FakeNotificationSink {
                fail_at: (!revoke).then_some(op),
                revoke_at: revoke.then_some(op),
                ..sink()
            };
            assert!(!send_balloon(&shell, 7, "Title", "Body", || shell
                .valid
                .get()));
            assert_eq!(
                *shell.calls.borrow(),
                operations[..=index],
                "{op:?}, revoke={revoke}"
            );
            assert_eq!(*shell.removed.borrow(), [7]);
        }
    }
}

#[test]
fn old_callbacks_cannot_remove_a_replacement_balloon() {
    let mut life = BalloonLife::default();
    let (old, first) = life.begin();
    assert_eq!(old, None);
    assert!(life.is_current(first));
    assert!(!life.is_shown(first));
    assert!(life.mark_shown(first));
    assert!(life.is_shown(first));
    let (old, second) = life.begin();
    assert_eq!(old, Some(first));
    assert_ne!(first, second);
    assert!(!life.mark_shown(first));
    assert!(!life.is_shown(second));
    assert!(!life.finish(first));
    assert!(life.is_current(second));
    assert!(life.finish(second));
    assert_eq!(life.cancel(), None);
}

#[test]
fn cancel_and_wrapped_ids_preserve_lifetime_invariants() {
    let mut life = BalloonLife {
        last: u16::MAX,
        active: None,
        ..Default::default()
    };
    assert_eq!(life.begin(), (None, 1));
    assert_eq!(life.cancel(), Some(1));
    assert_eq!(life.cancel(), None);
    assert!(!life.is_current(1));
    assert!(!life.finish(1));
    assert_eq!(life.begin(), (None, 2));
}

#[test]
fn busy_delivery_is_discarded_without_replaying_consumed_crossings() {
    let cfg = super::tests::enabled();
    let mut policy = Notifications::default();
    policy.observe(
        &cfg,
        Kind::Claude,
        &super::tests::reading(70.0, 1_000_000),
        1_000_000,
    );
    let batch = policy.observe(
        &cfg,
        Kind::Claude,
        &super::tests::reading(97.0, 1_000_000),
        1_000_000,
    );
    let body = balloon_text(Lang::En, &cfg, &batch).unwrap();
    assert_eq!(body, "Claude Code: 5h 95% · weekly 95%");
    let shell = FakeNotificationSink {
        blocked: true,
        ..sink()
    };
    assert!(!send_balloon(&shell, 7, "Title", &body, || true));
    assert_eq!(*shell.calls.borrow(), vec![]);
    assert_eq!(*shell.removed.borrow(), [7]);
    let later = policy.observe(
        &cfg,
        Kind::Claude,
        &super::tests::reading(97.0, 1_000_001),
        1_000_001,
    );
    assert_eq!(later, vec![]);
    assert_eq!(balloon_text(Lang::En, &cfg, &later), None);
}

#[test]
fn classified_result_batch_reaches_the_shell_once_with_only_fresh_data() {
    let cfg = super::tests::enabled();
    let mut policy = Notifications::default();
    let now = 1_000_000;
    let mut batch = Vec::new();
    for kind in [Kind::Claude, Kind::Codex] {
        let previous = super::tests::reading(70.0, now);
        policy.observe(&cfg, kind, &previous, now);
        let ProviderStatus::Ok { usage, .. } = super::tests::reading(97.0, now) else {
            unreachable!()
        };
        let fallback = classify_limit_result(&previous, Ok((usage.clone(), true, now)), now);
        assert_eq!(policy.observe(&cfg, kind, &fallback, now), vec![]);
        let fresh = classify_limit_result(&previous, Ok((usage, false, now)), now);
        batch.extend(policy.observe(&cfg, kind, &fresh, now));
    }
    let body = balloon_text(Lang::En, &cfg, &batch).unwrap();
    assert_eq!(
        body,
        "Claude Code: 5h 95% · weekly 95%\nCodex: 5h 95% · weekly 95%"
    );
    let shell = sink();
    assert!(send_balloon(
        &shell,
        7,
        t(Lang::En, Key::NotifyTitle),
        &body,
        || true
    ));
    assert_eq!(
        shell
            .calls
            .borrow()
            .iter()
            .filter(|&&op| op == BalloonCommand::Show)
            .count(),
        1
    );
    assert_eq!(
        shell.payloads.borrow()[2],
        (7, "CCT — Usage limits".into(), body)
    );
}
