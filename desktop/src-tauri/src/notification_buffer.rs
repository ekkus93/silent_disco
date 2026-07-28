use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CoreNotification, CoreObserver, CoreSnapshot};
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub const DESKTOP_PENDING_NOTIFICATION_CAPACITY: usize = 64;

#[derive(Debug, Default)]
struct NotificationState {
    latest_snapshot: Option<CoreSnapshot>,
    pending: VecDeque<CoreNotification>,
}

/// Bounded bridge-side notification staging used before the frontend channel is attached.
///
/// Snapshots are coalesced to the latest revision. Effects, errors, and diagnostics are
/// retained in FIFO order and fail the core observer visibly if the bounded queue fills.
#[derive(Debug, Default)]
pub struct DesktopNotificationBuffer {
    state: Mutex<NotificationState>,
    snapshot_available: Condvar,
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
        Self {
            fail_initial_notification: true,
            ..Self::default()
        }
    }

    /// Waits for the actor's first delivered snapshot.
    ///
    /// # Errors
    ///
    /// Returns a fatal bridge error on poisoned synchronization or timeout.
    pub fn wait_for_initial_snapshot(&self, timeout: Duration) -> Result<CoreSnapshot, CoreError> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .map_err(|_| bridge_error("desktop notification state was poisoned"))?;

        loop {
            if let Some(snapshot) = &state.latest_snapshot {
                return Ok(snapshot.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(bridge_error(
                    "timed out waiting for the initial authoritative core snapshot",
                ));
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next, wait_result) = self
                .snapshot_available
                .wait_timeout(state, remaining)
                .map_err(|_| bridge_error("desktop notification state was poisoned"))?;
            state = next;
            if wait_result.timed_out() && state.latest_snapshot.is_none() {
                return Err(bridge_error(
                    "timed out waiting for the initial authoritative core snapshot",
                ));
            }
        }
    }

    #[cfg(test)]
    fn pending_len(&self) -> Result<usize, CoreError> {
        self.state
            .lock()
            .map(|state| state.pending.len())
            .map_err(|_| bridge_error("desktop notification state was poisoned"))
    }
}

impl CoreObserver for DesktopNotificationBuffer {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        #[cfg(test)]
        if self.fail_initial_notification {
            return Err(bridge_error(
                "injected desktop notification observer setup failure",
            ));
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| bridge_error("desktop notification state was poisoned"))?;
        match notification {
            CoreNotification::Snapshot(snapshot) => {
                let replace = state
                    .latest_snapshot
                    .as_ref()
                    .is_none_or(|current| snapshot.revision > current.revision);
                if replace {
                    state.latest_snapshot = Some(snapshot);
                    self.snapshot_available.notify_all();
                }
                Ok(())
            }
            other => {
                if state.pending.len() >= DESKTOP_PENDING_NOTIFICATION_CAPACITY {
                    return Err(core_error(
                        CoreErrorCode::QueueOverflow,
                        "desktop pending notification queue is full",
                    ));
                }
                state.pending.push_back(other);
                Ok(())
            }
        }
    }
}

fn bridge_error(message: &'static str) -> CoreError {
    core_error(CoreErrorCode::FfiCallbackFailed, message)
}

fn core_error(code: CoreErrorCode, message: &'static str) -> CoreError {
    match CoreError::new(code, message, ErrorSeverity::Fatal, false, None) {
        Ok(error) => error,
        Err(error) => panic!("invalid static desktop notification error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{DESKTOP_PENDING_NOTIFICATION_CAPACITY, DesktopNotificationBuffer};
    use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
    use silent_disco_core::runtime::{CoreNotification, CoreObserver, CoreSnapshot};
    use std::time::Duration;

    #[test]
    fn coalesces_snapshots_and_delivers_initial_snapshot() {
        let observer = DesktopNotificationBuffer::new();
        observer
            .on_notification(CoreNotification::Snapshot(CoreSnapshot::default()))
            .expect("accept initial snapshot");
        let snapshot = observer
            .wait_for_initial_snapshot(Duration::from_millis(10))
            .expect("initial snapshot");
        assert_eq!(snapshot.revision.get(), 0);
        assert_eq!(observer.pending_len().expect("pending length"), 0);
    }

    #[test]
    fn non_snapshot_queue_overflow_is_visible() {
        let observer = DesktopNotificationBuffer::new();
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
    }
}
