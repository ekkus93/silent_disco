use super::{
    DESKTOP_PENDING_NOTIFICATION_CAPACITY, DesktopNotificationBuffer, DesktopNotificationSendError,
    DesktopNotificationSink, DesktopNotificationSubscriptionId, NotificationShared,
    record_delivery_failure, record_worker_failure,
};
use silent_disco_core::domain::OperationId;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CoreDiagnostic, CoreNotification, CoreObserver, CoreSnapshot, PlatformEffect,
    PlatformEffectRequest, SnapshotRevision,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Default)]
struct RecordingState {
    notifications: Mutex<Vec<CoreNotification>>,
    available: Condvar,
}

#[derive(Clone, Default)]
struct RecordingSink {
    state: Arc<RecordingState>,
    fail: bool,
}

impl RecordingSink {
    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    fn wait_for_len(&self, expected: usize) -> Vec<CoreNotification> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut notifications = self.state.notifications.lock().expect("recording lock");
        while notifications.len() < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "notification delivery timed out");
            let (next, wait) = self
                .state
                .available
                .wait_timeout(notifications, remaining)
                .expect("recording wait");
            notifications = next;
            assert!(!wait.timed_out(), "notification delivery timed out");
        }
        notifications.clone()
    }

    fn len(&self) -> usize {
        self.state
            .notifications
            .lock()
            .expect("recording lock")
            .len()
    }
}

impl DesktopNotificationSink for RecordingSink {
    fn send(&self, notification: CoreNotification) -> Result<(), DesktopNotificationSendError> {
        if self.fail {
            return Err(
                DesktopNotificationSendError::new("injected channel failure")
                    .expect("valid test failure"),
            );
        }
        let mut notifications = self.state.notifications.lock().expect("recording lock");
        notifications.push(notification);
        self.state.available.notify_all();
        Ok(())
    }
}

#[derive(Default)]
struct FailAfterInitialSink {
    sends: AtomicUsize,
}

impl DesktopNotificationSink for FailAfterInitialSink {
    fn send(&self, _notification: CoreNotification) -> Result<(), DesktopNotificationSendError> {
        if self.sends.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(());
        }
        Err(
            DesktopNotificationSendError::new("injected reload channel failure")
                .expect("valid test failure"),
        )
    }
}

#[derive(Default)]
struct BlockingState {
    sends: usize,
    blocked: bool,
    released: bool,
}

#[derive(Default)]
struct BlockingSink {
    state: Mutex<BlockingState>,
    available: Condvar,
}

impl BlockingSink {
    fn wait_until_blocked(&self) {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut state = self.state.lock().expect("blocking lock");
        while !state.blocked {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "notification send did not block");
            let (next, wait) = self
                .available
                .wait_timeout(state, remaining)
                .expect("blocking wait");
            state = next;
            assert!(!wait.timed_out(), "notification send did not block");
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().expect("blocking lock");
        state.released = true;
        self.available.notify_all();
    }
}

impl DesktopNotificationSink for BlockingSink {
    fn send(&self, _notification: CoreNotification) -> Result<(), DesktopNotificationSendError> {
        let mut state = self.state.lock().expect("blocking lock");
        state.sends += 1;
        if state.sends == 2 {
            state.blocked = true;
            self.available.notify_all();
            while !state.released {
                state = self.available.wait(state).expect("blocking wait");
            }
        }
        Ok(())
    }
}

fn snapshot(revision: u64) -> CoreNotification {
    CoreNotification::Snapshot(CoreSnapshot {
        revision: SnapshotRevision::new(revision),
        ..CoreSnapshot::default()
    })
}

fn diagnostic(name: &str) -> CoreNotification {
    CoreNotification::Diagnostic(CoreDiagnostic::new(name, Vec::new()).expect("diagnostic"))
}

fn platform_error(message: &str) -> CoreNotification {
    CoreNotification::Error(
        CoreError::new(
            CoreErrorCode::PlatformOperationFailed,
            message,
            ErrorSeverity::Error,
            false,
            None,
        )
        .expect("valid test error"),
    )
}

fn platform_effect(operation_id: &str) -> CoreNotification {
    CoreNotification::Effect(
        PlatformEffect::new(
            OperationId::new(operation_id).expect("operation ID"),
            PlatformEffectRequest::StopAdvertising,
        )
        .expect("platform effect"),
    )
}

fn wait_for_delivery_failure(observer: &DesktopNotificationBuffer) -> CoreError {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(error) = observer.delivery_failure().expect("failure state") {
            return error;
        }
        assert!(
            Instant::now() < deadline,
            "delivery failure was not recorded"
        );
        thread::yield_now();
    }
}

fn assert_snapshot_revision(notification: &CoreNotification, expected: u64) {
    let CoreNotification::Snapshot(snapshot) = notification else {
        panic!("notification must be a snapshot");
    };
    assert_eq!(snapshot.revision.get(), expected);
}

#[test]
fn coalesces_snapshots_and_delivers_initial_snapshot() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(1)).expect("snapshot 1");
    observer.on_notification(snapshot(3)).expect("snapshot 3");
    observer
        .on_notification(snapshot(2))
        .expect("stale snapshot");

    let sink = RecordingSink::default();
    observer
        .attach_sink(Arc::new(sink.clone()))
        .expect("attach subscriber");
    let delivered = sink.wait_for_len(1);
    assert_snapshot_revision(&delivered[0], 3);
    assert_eq!(observer.pending_len().expect("pending length"), 0);
    observer.shutdown().expect("shutdown");
}

#[test]
fn live_snapshots_are_delivered_in_monotonic_revision_order() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(1)).expect("snapshot 1");
    let sink = RecordingSink::default();
    observer
        .attach_sink(Arc::new(sink.clone()))
        .expect("attach subscriber");
    sink.wait_for_len(1);

    observer.on_notification(snapshot(2)).expect("snapshot 2");
    sink.wait_for_len(2);
    observer
        .on_notification(snapshot(1))
        .expect("stale live snapshot");
    observer.on_notification(snapshot(3)).expect("snapshot 3");
    let delivered = sink.wait_for_len(3);

    assert_snapshot_revision(&delivered[0], 1);
    assert_snapshot_revision(&delivered[1], 2);
    assert_snapshot_revision(&delivered[2], 3);
    observer.shutdown().expect("shutdown");
}

#[test]
fn pending_non_snapshots_remain_ordered_after_initial_snapshot() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(4)).expect("snapshot");
    observer
        .on_notification(diagnostic("first"))
        .expect("first diagnostic");
    observer
        .on_notification(diagnostic("second"))
        .expect("second diagnostic");

    let sink = RecordingSink::default();
    observer
        .attach_sink(Arc::new(sink.clone()))
        .expect("attach subscriber");
    let delivered = sink.wait_for_len(3);
    assert_snapshot_revision(&delivered[0], 4);
    assert!(matches!(
        &delivered[1],
        CoreNotification::Diagnostic(value) if value.name == "first"
    ));
    assert!(matches!(
        &delivered[2],
        CoreNotification::Diagnostic(value) if value.name == "second"
    ));
    observer.shutdown().expect("shutdown");
}

#[test]
fn pending_events_are_not_starved_by_snapshot_coalescing() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(1)).expect("snapshot 1");
    observer
        .on_notification(diagnostic("must-deliver"))
        .expect("diagnostic");
    for revision in 2..=100 {
        observer
            .on_notification(snapshot(revision))
            .expect("coalesced snapshot");
    }

    let sink = RecordingSink::default();
    observer
        .attach_sink(Arc::new(sink.clone()))
        .expect("attach subscriber");
    let delivered = sink.wait_for_len(2);
    assert_snapshot_revision(&delivered[0], 100);
    assert!(matches!(
        &delivered[1],
        CoreNotification::Diagnostic(value) if value.name == "must-deliver"
    ));
    observer.shutdown().expect("shutdown");
}

#[test]
fn effects_and_errors_survive_snapshot_pressure() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(1)).expect("snapshot 1");
    observer
        .on_notification(platform_effect("operation-1"))
        .expect("platform effect");
    observer
        .on_notification(platform_error("must-deliver-error"))
        .expect("platform error");
    for revision in 2..=100 {
        observer
            .on_notification(snapshot(revision))
            .expect("coalesced snapshot");
    }

    let sink = RecordingSink::default();
    observer
        .attach_sink(Arc::new(sink.clone()))
        .expect("attach subscriber");
    let delivered = sink.wait_for_len(3);
    assert_snapshot_revision(&delivered[0], 100);
    assert!(matches!(&delivered[1], CoreNotification::Effect(_)));
    assert!(matches!(
        &delivered[2],
        CoreNotification::Error(error) if error.message == "must-deliver-error"
    ));
    observer.shutdown().expect("shutdown");
}

#[test]
fn replacement_joins_old_worker_before_new_subscription_returns() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(5)).expect("snapshot");
    let first = RecordingSink::default();
    observer
        .attach_sink(Arc::new(first.clone()))
        .expect("first attach");
    first.wait_for_len(1);

    let second = RecordingSink::default();
    let first_len = first.len();
    let second_id = observer
        .attach_sink(Arc::new(second.clone()))
        .expect("replacement attach");
    assert!(second_id.get() >= 2);
    second.wait_for_len(1);
    observer
        .on_notification(diagnostic("new-subscriber-only"))
        .expect("diagnostic");
    second.wait_for_len(2);
    assert_eq!(first.len(), first_len);
    observer.shutdown().expect("shutdown");
}

#[test]
fn non_snapshot_queue_overflow_is_visible() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(0)).expect("snapshot");
    for index in 0..DESKTOP_PENDING_NOTIFICATION_CAPACITY {
        observer
            .on_notification(platform_error(&format!("failure {index}")))
            .expect("bounded notification accepted");
    }

    let error = observer
        .on_notification(platform_error("overflow"))
        .expect_err("overflow must fail");
    assert_eq!(error.code, CoreErrorCode::QueueOverflow);
    observer.shutdown().expect("shutdown");
}

#[test]
fn asynchronous_send_failure_becomes_visible_bridge_state() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(8)).expect("snapshot");
    observer
        .attach_sink(Arc::new(RecordingSink::failing()))
        .expect("attach failing sink");

    let error = wait_for_delivery_failure(&observer);
    assert_eq!(error.code, CoreErrorCode::FfiCallbackFailed);
    assert!(error.message.contains("injected channel failure"));
    observer.shutdown().expect("shutdown");
}

#[test]
fn frontend_reload_reattaches_and_redelivers_failed_notification() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(8)).expect("snapshot");
    observer
        .on_notification(diagnostic("retry-after-reload"))
        .expect("diagnostic");
    observer
        .attach_sink(Arc::new(FailAfterInitialSink::default()))
        .expect("attach failing sink");

    let failure = wait_for_delivery_failure(&observer);
    assert!(failure.message.contains("injected reload channel failure"));
    observer.on_notification(snapshot(9)).expect("new snapshot");
    observer
        .on_notification(diagnostic("after-failure"))
        .expect("post-failure diagnostic");

    let replacement = RecordingSink::default();
    observer
        .attach_sink(Arc::new(replacement.clone()))
        .expect("reattach after reload");
    let delivered = replacement.wait_for_len(3);
    assert_snapshot_revision(&delivered[0], 9);
    assert!(matches!(
        &delivered[1],
        CoreNotification::Diagnostic(value) if value.name == "retry-after-reload"
    ));
    assert!(matches!(
        &delivered[2],
        CoreNotification::Diagnostic(value) if value.name == "after-failure"
    ));
    assert!(
        observer
            .delivery_failure()
            .expect("failure state")
            .is_none()
    );
    observer.shutdown().expect("shutdown");
}

#[test]
fn shutdown_joins_worker_while_notification_send_is_pending() {
    let observer = Arc::new(DesktopNotificationBuffer::new());
    observer.on_notification(snapshot(9)).expect("snapshot");
    observer
        .on_notification(diagnostic("pending-at-shutdown"))
        .expect("pending diagnostic");
    let sink = Arc::new(BlockingSink::default());
    observer
        .attach_sink(sink.clone())
        .expect("attach blocking sink");
    sink.wait_until_blocked();

    let shutdown_observer = Arc::clone(&observer);
    let shutdown = thread::spawn(move || shutdown_observer.shutdown());
    sink.release();

    shutdown
        .join()
        .expect("shutdown thread")
        .expect("shutdown result");
}

#[test]
fn shutdown_rejects_later_notifications_and_attachments() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(9)).expect("snapshot");
    let sink = RecordingSink::default();
    observer
        .attach_sink(Arc::new(sink.clone()))
        .expect("attach");
    sink.wait_for_len(1);
    observer.shutdown().expect("shutdown");

    assert!(observer.on_notification(snapshot(10)).is_err());
    assert!(
        observer
            .attach_sink(Arc::new(RecordingSink::default()))
            .is_err()
    );
}

/// Poisons `shared.state` the same way a real panic-while-holding-the-lock
/// would: locks it on a spawned thread and panics before unlocking.
fn poison_state(shared: &Arc<NotificationShared>) {
    let poisoning = Arc::clone(shared);
    let handle = thread::spawn(move || {
        let _guard = poisoning.state.lock().expect("acquire lock to poison it");
        panic!("deliberately poisoning the notification state mutex for a test");
    });
    assert!(handle.join().is_err(), "poisoning thread must panic");
}

/// Block 44 audit fix: `record_delivery_failure` used to `.expect()` the
/// state lock and panic a background subscription-worker thread if it was
/// already poisoned by an earlier, unrelated panic. That added a second,
/// redundant failure on top of one already independently visible to every
/// other caller through `state_poisoned_error()` -- confirms it now simply
/// returns instead of panicking.
#[test]
fn record_delivery_failure_does_not_panic_on_an_already_poisoned_state() {
    let shared = Arc::new(NotificationShared::default());
    poison_state(&shared);

    let error = DesktopNotificationSendError::new("channel closed").expect("valid message");
    record_delivery_failure(
        &shared,
        DesktopNotificationSubscriptionId(1),
        snapshot(1),
        &error,
    );
    // Reaching this line at all is the assertion: the call above must not
    // have panicked despite the already-poisoned lock.
}

/// Same guarantee as the above, for `record_worker_failure`'s own lock use.
#[test]
fn record_worker_failure_does_not_panic_on_an_already_poisoned_state() {
    let shared = Arc::new(NotificationShared::default());
    poison_state(&shared);

    let error = CoreError::new(
        CoreErrorCode::FfiCallbackFailed,
        "worker failed",
        ErrorSeverity::Fatal,
        false,
        None,
    )
    .expect("valid test error");
    record_worker_failure(&shared, DesktopNotificationSubscriptionId(1), error);
}
