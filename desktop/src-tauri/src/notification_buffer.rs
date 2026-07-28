use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CoreNotification, CoreObserver, CoreSnapshot, SnapshotRevision};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const DESKTOP_PENDING_NOTIFICATION_CAPACITY: usize = 64;

/// Identifies one frontend notification subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DesktopNotificationSubscriptionId(u64);

impl DesktopNotificationSubscriptionId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Native delivery boundary used by the bounded desktop notification dispatcher.
pub trait DesktopNotificationSink: Send + Sync + 'static {
    /// Delivers one source-ordered core notification.
    ///
    /// # Errors
    ///
    /// Returns a bounded description when the receiving channel is unavailable.
    fn send(&self, notification: CoreNotification) -> Result<(), DesktopNotificationSendError>;
}

/// Bounded native-channel delivery failure. The message must not contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotificationSendError {
    message: String,
}

impl DesktopNotificationSendError {
    /// Creates a bounded delivery error.
    ///
    /// # Errors
    ///
    /// Rejects blank, control-containing, or oversized messages.
    pub fn new(message: impl Into<String>) -> Result<Self, CoreError> {
        let message = message.into();
        if message.trim().is_empty()
            || message.trim() != message
            || message.len() > 256
            || message.chars().any(char::is_control)
        {
            return Err(bridge_error(
                CoreErrorCode::InvalidArgument,
                "desktop notification send error message is invalid",
            ));
        }
        Ok(Self { message })
    }
}

impl std::fmt::Display for DesktopNotificationSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DesktopNotificationSendError {}

#[derive(Debug, Default)]
struct NotificationState {
    latest_snapshot: Option<CoreSnapshot>,
    pending: VecDeque<CoreNotification>,
    active_subscription: Option<DesktopNotificationSubscriptionId>,
    next_subscription_id: u64,
    delivery_failure: Option<CoreError>,
    closed: bool,
}

#[derive(Debug, Default)]
struct NotificationShared {
    state: Mutex<NotificationState>,
    available: Condvar,
}

struct SubscriptionWorker {
    id: DesktopNotificationSubscriptionId,
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

/// Bounded bridge-side notification staging and native subscription dispatcher.
///
/// Snapshots are coalesced to the latest revision. Effects, errors, and diagnostics are
/// retained in FIFO order and fail the core observer visibly if the bounded queue fills.
/// One explicitly identified subscriber is active at a time; replacement stops and joins
/// the prior worker before the new subscription is published.
#[derive(Default)]
pub struct DesktopNotificationBuffer {
    shared: Arc<NotificationShared>,
    worker: Mutex<Option<SubscriptionWorker>>,
    #[cfg(test)]
    fail_initial_notification: bool,
}

impl DesktopNotificationBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    #[must_use]
    pub fn failing_initial_notification() -> Self {
        let mut buffer = Self::default();
        buffer.fail_initial_notification = true;
        buffer
    }

    /// Waits for the actor's first delivered snapshot.
    ///
    /// # Errors
    ///
    /// Returns a fatal bridge error on observer failure, poisoned synchronization, or timeout.
    pub fn wait_for_initial_snapshot(&self, timeout: Duration) -> Result<CoreSnapshot, CoreError> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| state_poisoned_error())?;

        loop {
            if let Some(error) = &state.delivery_failure {
                return Err(error.clone());
            }
            if state.closed {
                return Err(bridge_error(
                    CoreErrorCode::WorkerStopped,
                    "desktop notification bridge is closed",
                ));
            }
            if let Some(snapshot) = &state.latest_snapshot {
                return Ok(snapshot.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(initial_snapshot_timeout_error());
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next, wait_result) = self
                .shared
                .available
                .wait_timeout(state, remaining)
                .map_err(|_| state_poisoned_error())?;
            state = next;
            if wait_result.timed_out() && state.latest_snapshot.is_none() {
                return Err(initial_snapshot_timeout_error());
            }
        }
    }

    /// Replaces the active subscriber after stopping and joining its worker.
    ///
    /// The current authoritative snapshot is always the first notification delivered to the
    /// new subscriber. Pending effects, errors, and diagnostics remain source ordered.
    ///
    /// # Errors
    ///
    /// Returns a visible error for a failed bridge, closed bridge, identifier exhaustion,
    /// poisoned synchronization, thread-start failure, or prior worker join failure.
    pub fn attach_sink(
        &self,
        sink: Arc<dyn DesktopNotificationSink>,
    ) -> Result<DesktopNotificationSubscriptionId, CoreError> {
        let mut worker_slot = self.worker.lock().map_err(|_| worker_poisoned_error())?;
        stop_and_join(worker_slot.take(), &self.shared)?;

        let id = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| state_poisoned_error())?;
            ensure_bridge_available(&state)?;
            if state.latest_snapshot.is_none() {
                return Err(bridge_error(
                    CoreErrorCode::InvalidStateTransition,
                    "desktop notification bridge has no authoritative snapshot",
                ));
            }
            let next = state.next_subscription_id.checked_add(1).ok_or_else(|| {
                bridge_error(
                    CoreErrorCode::ResourceLimitExceeded,
                    "desktop notification subscription identifier exhausted",
                )
            })?;
            let id = DesktopNotificationSubscriptionId(next);
            state.next_subscription_id = next;
            state.active_subscription = Some(id);
            id
        };

        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let shared = Arc::clone(&self.shared);
        let join = thread::Builder::new()
            .name(format!("silent-disco-desktop-notifications-{}", id.get()))
            .spawn(move || run_subscription_worker(&shared, id, &worker_stop, sink.as_ref()))
            .map_err(|_| {
                clear_active_subscription(&self.shared, id);
                bridge_error(
                    CoreErrorCode::WorkerStopped,
                    "could not start desktop notification subscription worker",
                )
            })?;

        *worker_slot = Some(SubscriptionWorker { id, stop, join });
        self.shared.available.notify_all();
        Ok(id)
    }

    /// Returns a previously recorded asynchronous channel-delivery failure.
    ///
    /// # Errors
    ///
    /// Returns a fatal bridge error when synchronization state is poisoned.
    pub fn delivery_failure(&self) -> Result<Option<CoreError>, CoreError> {
        self.shared
            .state
            .lock()
            .map(|state| state.delivery_failure.clone())
            .map_err(|_| state_poisoned_error())
    }

    /// Stops and joins the active subscription worker.
    ///
    /// No later observer notification or subscription attachment is accepted.
    ///
    /// # Errors
    ///
    /// Returns a visible error if synchronization is poisoned or the worker panicked.
    pub fn shutdown(&self) -> Result<(), CoreError> {
        let mut worker_slot = self.worker.lock().map_err(|_| worker_poisoned_error())?;
        {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| state_poisoned_error())?;
            state.closed = true;
            state.active_subscription = None;
        }
        self.shared.available.notify_all();
        stop_and_join(worker_slot.take(), &self.shared)
    }

    #[cfg(test)]
    fn pending_len(&self) -> Result<usize, CoreError> {
        self.shared
            .state
            .lock()
            .map(|state| state.pending.len())
            .map_err(|_| state_poisoned_error())
    }
}

impl CoreObserver for DesktopNotificationBuffer {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        #[cfg(test)]
        if self.fail_initial_notification {
            return Err(bridge_error(
                CoreErrorCode::FfiCallbackFailed,
                "injected desktop notification observer setup failure",
            ));
        }

        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| state_poisoned_error())?;
        ensure_bridge_available(&state)?;
        match notification {
            CoreNotification::Snapshot(snapshot) => {
                let replace = state
                    .latest_snapshot
                    .as_ref()
                    .is_none_or(|current| snapshot.revision > current.revision);
                if replace {
                    state.latest_snapshot = Some(snapshot);
                    self.shared.available.notify_all();
                }
                Ok(())
            }
            other => {
                if state.pending.len() >= DESKTOP_PENDING_NOTIFICATION_CAPACITY {
                    return Err(bridge_error(
                        CoreErrorCode::QueueOverflow,
                        "desktop pending notification queue is full",
                    ));
                }
                state.pending.push_back(other);
                self.shared.available.notify_all();
                Ok(())
            }
        }
    }
}

impl Drop for DesktopNotificationBuffer {
    fn drop(&mut self) {
        let clean = self.worker.get_mut().is_ok_and(|worker| worker.is_none());
        assert!(
            clean || thread::panicking(),
            "DesktopNotificationBuffer dropped with an active subscription worker"
        );
    }
}

fn run_subscription_worker(
    shared: &NotificationShared,
    id: DesktopNotificationSubscriptionId,
    stop: &AtomicBool,
    sink: &dyn DesktopNotificationSink,
) {
    let mut delivered_snapshot = None;
    loop {
        let next = wait_for_next(shared, id, stop, delivered_snapshot).unwrap_or_else(|error| {
            record_worker_failure(shared, id, error);
            None
        });
        let Some(next) = next else {
            return;
        };
        let snapshot_revision = notification_snapshot_revision(&next);
        if sink.send(next).is_err() {
            record_delivery_failure(shared, id);
            return;
        }
        if let Some(revision) = snapshot_revision {
            delivered_snapshot = Some(revision);
        }
    }
}

fn wait_for_next(
    shared: &NotificationShared,
    id: DesktopNotificationSubscriptionId,
    stop: &AtomicBool,
    delivered_snapshot: Option<SnapshotRevision>,
) -> Result<Option<CoreNotification>, CoreError> {
    let mut state = shared.state.lock().map_err(|_| state_poisoned_error())?;
    loop {
        if stop.load(Ordering::Acquire)
            || state.closed
            || state.active_subscription != Some(id)
            || state.delivery_failure.is_some()
        {
            return Ok(None);
        }

        if delivered_snapshot.is_none()
            && let Some(snapshot) = newer_snapshot(&state, delivered_snapshot)
        {
            return Ok(Some(CoreNotification::Snapshot(snapshot)));
        }
        if let Some(notification) = state.pending.pop_front() {
            return Ok(Some(notification));
        }
        if let Some(snapshot) = newer_snapshot(&state, delivered_snapshot) {
            return Ok(Some(CoreNotification::Snapshot(snapshot)));
        }

        state = shared
            .available
            .wait(state)
            .map_err(|_| state_poisoned_error())?;
    }
}

fn newer_snapshot(
    state: &NotificationState,
    delivered: Option<SnapshotRevision>,
) -> Option<CoreSnapshot> {
    state
        .latest_snapshot
        .as_ref()
        .filter(|snapshot| delivered.is_none_or(|revision| snapshot.revision > revision))
        .cloned()
}

fn notification_snapshot_revision(notification: &CoreNotification) -> Option<SnapshotRevision> {
    match notification {
        CoreNotification::Snapshot(snapshot) => Some(snapshot.revision),
        CoreNotification::Effect(_)
        | CoreNotification::Error(_)
        | CoreNotification::Diagnostic(_) => None,
    }
}

fn record_delivery_failure(shared: &NotificationShared, id: DesktopNotificationSubscriptionId) {
    record_worker_failure(
        shared,
        id,
        bridge_error(
            CoreErrorCode::FfiCallbackFailed,
            "desktop notification channel send failed",
        ),
    );
}

fn record_worker_failure(
    shared: &NotificationShared,
    id: DesktopNotificationSubscriptionId,
    error: CoreError,
) {
    let mut state = shared
        .state
        .lock()
        .expect("desktop notification worker failure state was poisoned");
    if state.active_subscription == Some(id) {
        state.delivery_failure = Some(error);
        state.active_subscription = None;
        shared.available.notify_all();
    }
}

fn stop_and_join(
    worker: Option<SubscriptionWorker>,
    shared: &NotificationShared,
) -> Result<(), CoreError> {
    let Some(worker) = worker else {
        return Ok(());
    };
    worker.stop.store(true, Ordering::Release);
    clear_active_subscription(shared, worker.id);
    shared.available.notify_all();
    worker.join.join().map_err(|_| {
        bridge_error(
            CoreErrorCode::ShutdownFailed,
            "desktop notification subscription worker panicked",
        )
    })
}

fn clear_active_subscription(shared: &NotificationShared, id: DesktopNotificationSubscriptionId) {
    if let Ok(mut state) = shared.state.lock()
        && state.active_subscription == Some(id)
    {
        state.active_subscription = None;
    }
}

fn ensure_bridge_available(state: &NotificationState) -> Result<(), CoreError> {
    if let Some(error) = &state.delivery_failure {
        return Err(error.clone());
    }
    if state.closed {
        return Err(bridge_error(
            CoreErrorCode::WorkerStopped,
            "desktop notification bridge is closed",
        ));
    }
    Ok(())
}

fn initial_snapshot_timeout_error() -> CoreError {
    bridge_error(
        CoreErrorCode::FfiCallbackFailed,
        "timed out waiting for the initial authoritative core snapshot",
    )
}

fn state_poisoned_error() -> CoreError {
    bridge_error(
        CoreErrorCode::FfiCallbackFailed,
        "desktop notification state was poisoned",
    )
}

fn worker_poisoned_error() -> CoreError {
    bridge_error(
        CoreErrorCode::FfiCallbackFailed,
        "desktop notification worker state was poisoned",
    )
}

fn bridge_error(code: CoreErrorCode, message: &'static str) -> CoreError {
    match CoreError::new(code, message, ErrorSeverity::Fatal, false, None) {
        Ok(error) => error,
        Err(error) => panic!("invalid static desktop notification error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DESKTOP_PENDING_NOTIFICATION_CAPACITY, DesktopNotificationBuffer,
        DesktopNotificationSendError, DesktopNotificationSink,
    };
    use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
    use silent_disco_core::runtime::{
        CoreDiagnostic, CoreNotification, CoreObserver, CoreSnapshot, SnapshotRevision,
    };
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

    fn snapshot(revision: u64) -> CoreNotification {
        CoreNotification::Snapshot(CoreSnapshot {
            revision: SnapshotRevision::new(revision),
            ..CoreSnapshot::default()
        })
    }

    fn diagnostic(name: &str) -> CoreNotification {
        CoreNotification::Diagnostic(CoreDiagnostic::new(name, Vec::new()).expect("diagnostic"))
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
        let CoreNotification::Snapshot(snapshot) = &delivered[0] else {
            panic!("initial notification must be a snapshot");
        };
        assert_eq!(snapshot.revision.get(), 3);
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
        assert!(matches!(&delivered[0], CoreNotification::Snapshot(_)));
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
        let CoreNotification::Snapshot(snapshot) = &delivered[0] else {
            panic!("initial notification must be a snapshot");
        };
        assert_eq!(snapshot.revision.get(), 100);
        assert!(matches!(
            &delivered[1],
            CoreNotification::Diagnostic(value) if value.name == "must-deliver"
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
                .on_notification(CoreNotification::Error(
                    CoreError::new(
                        CoreErrorCode::PlatformOperationFailed,
                        format!("failure {index}"),
                        ErrorSeverity::Error,
                        false,
                        None,
                    )
                    .expect("valid test error"),
                ))
                .expect("bounded notification accepted");
        }

        let error = observer
            .on_notification(CoreNotification::Error(
                CoreError::new(
                    CoreErrorCode::PlatformOperationFailed,
                    "overflow",
                    ErrorSeverity::Error,
                    false,
                    None,
                )
                .expect("valid test error"),
            ))
            .expect_err("overflow must fail");
        assert_eq!(error.code, CoreErrorCode::QueueOverflow);
        observer.shutdown().expect("shutdown");
    }

    #[test]
    fn asynchronous_send_failure_becomes_visible_bridge_failure() {
        let observer = DesktopNotificationBuffer::new();
        observer.on_notification(snapshot(8)).expect("snapshot");
        observer
            .attach_sink(Arc::new(RecordingSink::failing()))
            .expect("attach failing sink");

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if observer
                .delivery_failure()
                .expect("failure state")
                .is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "delivery failure was not recorded"
            );
            std::thread::yield_now();
        }
        let error = observer
            .on_notification(diagnostic("after-failure"))
            .expect_err("observer must surface channel failure");
        assert_eq!(error.code, CoreErrorCode::FfiCallbackFailed);
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
}
