use super::{
    DESKTOP_PENDING_NOTIFICATION_CAPACITY, DesktopNotificationBuffer,
    DesktopNotificationSendError, DesktopNotificationSink,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CoreDiagnostic, CoreNotification, CoreObserver, CoreSnapshot, SnapshotRevision,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
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
        std::thread::yield_now();
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
fn errors_survive_snapshot_pressure() {
    let observer = DesktopNotificationBuffer::new();
    observer.on_notification(snapshot(1)).expect("snapshot 1");
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
    let delivered = sink.wait_for_len(2);
    assert_snapshot_revision(&delivered[0], 100);
    assert!(matches!(
        &delivered[1],
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
