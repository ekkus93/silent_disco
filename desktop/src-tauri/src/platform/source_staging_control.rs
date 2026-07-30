use crate::dto::DesktopErrorDto;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tauri::{AppHandle, Emitter, Runtime};

pub(crate) const SOURCE_STAGING_PROGRESS_EVENT: &str =
    "silent-disco://audio-source-staging-progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceStagingProgressDto {
    pub copied_bytes: u64,
    pub total_bytes: u64,
}

pub(crate) trait SourceStagingProgressSink: Send + Sync {
    fn emit(&self, progress: SourceStagingProgressDto) -> Result<(), DesktopErrorDto>;
}

pub(crate) struct TauriSourceStagingProgressSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriSourceStagingProgressSink<R> {
    #[must_use]
    pub(crate) fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> SourceStagingProgressSink for TauriSourceStagingProgressSink<R> {
    fn emit(&self, progress: SourceStagingProgressDto) -> Result<(), DesktopErrorDto> {
        self.app
            .emit(SOURCE_STAGING_PROGRESS_EVENT, progress)
            .map_err(|error| {
                staging_control_error(
                    "desktop.audio_source.progress_delivery_failed",
                    true,
                    &format!("audio source staging progress could not be delivered: {error}"),
                )
            })
    }
}

#[derive(Clone, Default)]
pub(crate) struct SourceStagingControl {
    inner: Arc<SourceStagingControlInner>,
}

#[derive(Default)]
struct SourceStagingControlInner {
    state: Mutex<SourceStagingState>,
    idle: Condvar,
    cancelled: AtomicBool,
}

#[derive(Default)]
struct SourceStagingState {
    active: bool,
}

impl SourceStagingControl {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn begin(&self) -> Result<SourceStagingOperation, DesktopErrorDto> {
        let mut state = self.inner.state.lock().map_err(|_| {
            staging_control_error(
                "desktop.audio_source.staging_state_poisoned",
                false,
                "audio source staging state mutex was poisoned",
            )
        })?;
        if state.active {
            return Err(staging_control_error(
                "desktop.audio_source.staging_in_progress",
                true,
                "another audio source staging operation is already active",
            ));
        }
        self.inner.cancelled.store(false, Ordering::Release);
        state.active = true;
        Ok(SourceStagingOperation {
            control: self.clone(),
            finished: false,
        })
    }

    #[must_use]
    pub(crate) fn cancel(&self) -> bool {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.state.lock().map_or(true, |state| state.active)
    }

    pub(crate) fn cancel_and_wait(&self) -> Result<(), DesktopErrorDto> {
        self.inner.cancelled.store(true, Ordering::Release);
        let mut state = self.inner.state.lock().map_err(|_| {
            staging_control_error(
                "desktop.audio_source.staging_state_poisoned",
                false,
                "audio source staging state mutex was poisoned",
            )
        })?;
        while state.active {
            state = self.inner.idle.wait(state).map_err(|_| {
                staging_control_error(
                    "desktop.audio_source.staging_state_poisoned",
                    false,
                    "audio source staging state mutex was poisoned while waiting for cancellation",
                )
            })?;
        }
        Ok(())
    }

    fn finish(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.active = false;
            self.inner.idle.notify_all();
        }
    }
}

pub(crate) struct SourceStagingOperation {
    control: SourceStagingControl,
    finished: bool,
}

impl SourceStagingOperation {
    #[must_use]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.control.inner.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn ensure_not_cancelled(&self) -> Result<(), DesktopErrorDto> {
        if self.is_cancelled() {
            Err(staging_control_error(
                "desktop.audio_source.staging_cancelled",
                true,
                "audio source staging was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for SourceStagingOperation {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            self.control.finish();
        }
    }
}

fn staging_control_error(code: &str, retryable: bool, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(code, "audio_source", "error", retryable, message)
}

#[cfg(test)]
mod tests {
    use super::SourceStagingControl;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn overlapping_operations_are_rejected_and_cancel_is_explicit() {
        let control = SourceStagingControl::new();
        let operation = control.begin().expect("begin");
        assert!(control.begin().is_err());
        assert!(control.cancel());
        assert!(operation.is_cancelled());
        drop(operation);
        assert!(!control.cancel());
    }

    #[test]
    fn cancel_and_wait_blocks_until_the_operation_finishes() {
        let control = SourceStagingControl::new();
        let operation = control.begin().expect("begin");
        let waiter = control.clone();
        let (sent, received) = mpsc::channel();
        let thread = thread::spawn(move || {
            waiter.cancel_and_wait().expect("cancel and wait");
            sent.send(()).expect("signal");
        });

        assert!(received.recv_timeout(Duration::from_millis(20)).is_err());
        assert!(operation.is_cancelled());
        drop(operation);
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("waiter completed");
        thread.join().expect("join");
    }
}
