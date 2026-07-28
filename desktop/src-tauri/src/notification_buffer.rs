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
    failed_delivery: Option<CoreNotification>,
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
    /// new subscriber. A prior channel failure is cleared only after the old worker is joined.
    /// The failed non-snapshot notification and later pending notifications remain ordered.
    ///
    /// # Errors
    ///
    /// Returns a visible error for a closed bridge, identifier exhaustion, poisoned
    /// synchronization, thread-start failure, or prior worker join failure.
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
            ensure_bridge_open(&state)?;
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
            state.delivery_failure = None;
            state.active_subscription = Some(id);
            id
        };

        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let shared = Arc::clone(&self.shared);
        let join = thread::Builder::new()
            .name(format!("silent-disco-desktop-notifications-{}", id.get()))
            .spawn(move || run_subscription_worker(&shared, id, &worker_stop, sink.as_ref()))
            .map_err(|_| worker_start_error_after_cleanup(&self.shared, id))?;

        *worker_slot = Some(SubscriptionWorker { id, stop, join });
        self.shared.available.notify_all();
        Ok(id)
    }

    /// Returns the latest asynchronous channel-delivery failure.
    ///
    /// The failure remains visible until a replacement subscription is installed. It does not
    /// stop the core observer from retaining later authoritative notifications for that reload.
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

    /// Stops and joins the active subscription worker and closes the observer.
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
        ensure_bridge_open(&state)?;
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
        if let Err(error) = sink.send(next.clone()) {
            record_delivery_failure(shared, id, next, &error);
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
        if stop.load(Ordering::Acquire) || state.closed || state.active_subscription != Some(id) {
            return Ok(None);
        }

        if delivered_snapshot.is_none()
            && let Some(snapshot) = newer_snapshot(&state, delivered_snapshot)
        {
            return Ok(Some(CoreNotification::Snapshot(snapshot)));
        }
        if let Some(notification) = state.failed_delivery.take() {
            return Ok(Some(notification));
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

fn record_delivery_failure(
    shared: &NotificationShared,
    id: DesktopNotificationSubscriptionId,
    notification: CoreNotification,
    error: &DesktopNotificationSendError,
) {
    let mut state = shared
        .state
        .lock()
        .expect("desktop notification delivery failure state was poisoned");
    if state.active_subscription != Some(id) {
        return;
    }
    if !matches!(&notification, CoreNotification::Snapshot(_)) {
        assert!(
            state.failed_delivery.replace(notification).is_none(),
            "desktop notification dispatcher retained more than one failed delivery"
        );
    }
    state.delivery_failure = Some(channel_send_error(error));
    state.active_subscription = None;
    shared.available.notify_all();
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
    clear_active_subscription(shared, worker.id)?;
    shared.available.notify_all();
    worker.join.join().map_err(|_| {
        bridge_error(
            CoreErrorCode::ShutdownFailed,
            "desktop notification subscription worker panicked",
        )
    })
}

fn worker_start_error_after_cleanup(
    shared: &NotificationShared,
    id: DesktopNotificationSubscriptionId,
) -> CoreError {
    match clear_active_subscription(shared, id) {
        Ok(()) => bridge_error(
            CoreErrorCode::WorkerStopped,
            "could not start desktop notification subscription worker",
        ),
        Err(error) => error,
    }
}

fn clear_active_subscription(
    shared: &NotificationShared,
    id: DesktopNotificationSubscriptionId,
) -> Result<(), CoreError> {
    let mut state = shared.state.lock().map_err(|_| state_poisoned_error())?;
    if state.active_subscription == Some(id) {
        state.active_subscription = None;
    }
    Ok(())
}

fn ensure_bridge_open(state: &NotificationState) -> Result<(), CoreError> {
    if state.closed {
        return Err(bridge_error(
            CoreErrorCode::WorkerStopped,
            "desktop notification bridge is closed",
        ));
    }
    Ok(())
}

fn channel_send_error(error: &DesktopNotificationSendError) -> CoreError {
    match CoreError::new(
        CoreErrorCode::FfiCallbackFailed,
        format!("desktop notification channel send failed: {error}"),
        ErrorSeverity::Fatal,
        true,
        None,
    ) {
        Ok(error) => error,
        Err(error) => panic!("invalid bounded desktop notification channel error: {error}"),
    }
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
#[path = "notification_buffer_tests.rs"]
mod tests;
