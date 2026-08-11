use crate::lab::recorder::{RecordingObserver, ScenarioRecorder};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CoreNotification, CoreObserver};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

const EFFECT_QUEUE_CAPACITY: usize = 128;

/// Records every actor notification and separately queues effects that the
/// Lab platform adapter must execute. The queue is bounded so an actor cannot
/// outrun scenario execution without surfacing a real failure.
pub(super) struct LiveScenarioObserver {
    recorder: RecordingObserver,
    effects: SyncSender<CoreNotification>,
}

impl LiveScenarioObserver {
    pub(super) fn new(recorder: Arc<ScenarioRecorder>) -> (Self, Receiver<CoreNotification>) {
        let (effects, receiver) = mpsc::sync_channel(EFFECT_QUEUE_CAPACITY);
        (
            Self {
                recorder: RecordingObserver(recorder),
                effects,
            },
            receiver,
        )
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
        self.effects.try_send(notification).map_err(|queue_error| {
            let message = match queue_error {
                TrySendError::Full(_) => "Lab live-effect queue reached its bounded capacity",
                TrySendError::Disconnected(_) => "Lab live-effect consumer disconnected",
            };
            CoreError::new(
                CoreErrorCode::QueueOverflow,
                message,
                ErrorSeverity::Error,
                true,
                None,
            )
            .expect("static Lab effect-queue error text satisfies the core error contract")
        })
    }
}
