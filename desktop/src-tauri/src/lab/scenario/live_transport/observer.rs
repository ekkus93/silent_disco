use super::support::live_error;
use crate::dto::DesktopErrorDto;
use crate::lab::recorder::{RecordingObserver, ScenarioRecorder};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CoreNotification, CoreObserver};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

const EFFECT_QUEUE_CAPACITY: usize = 128;

/// Records every actor notification and separately queues effects that the
/// Lab platform adapter must execute. The queue is bounded so an actor cannot
/// outrun scenario execution without surfacing a real failure.
pub(in crate::lab::scenario) struct LiveScenarioObserver {
    recorder: RecordingObserver,
    effects: SyncSender<CoreNotification>,
    queue_full_error: CoreError,
    queue_disconnected_error: CoreError,
}

impl LiveScenarioObserver {
    /// Creates the bounded live-effect observer and its consumer.
    ///
    /// # Errors
    ///
    /// Returns a structured Lab error if the shared core rejects one of the
    /// prevalidated queue-failure error shapes.
    pub(in crate::lab::scenario) fn new(
        recorder: Arc<ScenarioRecorder>,
    ) -> Result<(Self, Receiver<CoreNotification>), DesktopErrorDto> {
        let queue_full_error = queue_error("Lab live-effect queue reached its bounded capacity")?;
        let queue_disconnected_error = queue_error("Lab live-effect consumer disconnected")?;
        let (effects, receiver) = mpsc::sync_channel(EFFECT_QUEUE_CAPACITY);
        Ok((
            Self {
                recorder: RecordingObserver(recorder),
                effects,
                queue_full_error,
                queue_disconnected_error,
            },
            receiver,
        ))
    }
}

impl CoreObserver for LiveScenarioObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        self.recorder.on_notification(notification.clone())?;
        if !matches!(
            notification,
            CoreNotification::Effect(_)
                | CoreNotification::TransportEffect(_)
                | CoreNotification::StorageEffect(_)
        ) {
            return Ok(());
        }
        self.effects
            .try_send(notification)
            .map_err(|queue_error| match queue_error {
                TrySendError::Full(_) => self.queue_full_error.clone(),
                TrySendError::Disconnected(_) => self.queue_disconnected_error.clone(),
            })
    }
}

fn queue_error(message: &'static str) -> Result<CoreError, DesktopErrorDto> {
    CoreError::new(
        CoreErrorCode::QueueOverflow,
        message,
        ErrorSeverity::Error,
        true,
        None,
    )
    .map_err(|error| live_error("observer_error_shape_invalid", &error.to_string()))
}
