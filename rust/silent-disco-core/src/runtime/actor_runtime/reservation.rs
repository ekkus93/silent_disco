//! Bounded producer-to-actor reservation acknowledgment.
use super::{ActorMessage, CoreActorHandle, core_error, worker_stopped};
use crate::domain::StreamId;
use crate::error::{CoreError, CoreErrorCode, ErrorSeverity};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, TrySendError, sync_channel};
use std::time::Duration;

const RESERVATION_POLL: Duration = Duration::from_millis(20);

impl CoreActorHandle {
    /// Atomically reserves the next boundary against authoritative queue
    /// mutations before a producer enqueues its boundary or successor audio.
    /// Unlike `submit_audio_event`, success means the actor *applied* it.
    ///
    /// # Errors
    /// Rejects stale stream identities, queue conflicts, cancellation, actor
    /// shutdown, and full actor queues; it does not block on queue admission.
    pub fn reserve_host_track_boundary(
        &self,
        stream_id: StreamId,
        entry_id: String,
        first_sample_index: u64,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), CoreError> {
        self.ensure_accepting()?;
        if cancelled.load(Ordering::Acquire) {
            return Err(reservation_cancelled());
        }
        let (reply, receipt) = sync_channel(1);
        match self.sender.try_send(ActorMessage::ReserveBoundary {
            stream_id,
            entry_id,
            first_sample_index,
            cancelled: Arc::clone(&cancelled),
            reply,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(core_error(
                    CoreErrorCode::QueueOverflow,
                    "authoritative actor queue is full during boundary reservation",
                    ErrorSeverity::Error,
                    true,
                    None,
                ));
            }
            Err(TrySendError::Disconnected(_)) => return Err(worker_stopped(None)),
        }
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(reservation_cancelled());
            }
            match receipt.recv_timeout(RESERVATION_POLL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => self.ensure_accepting()?,
                Err(RecvTimeoutError::Disconnected) => return Err(worker_stopped(None)),
            }
        }
    }
}

fn reservation_cancelled() -> CoreError {
    core_error(
        CoreErrorCode::ShutdownInProgress,
        "playlist boundary reservation was cancelled",
        ErrorSeverity::Warning,
        false,
        None,
    )
}
