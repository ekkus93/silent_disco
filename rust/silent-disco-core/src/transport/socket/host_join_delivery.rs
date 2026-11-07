//! Authoritative join admission at a saturated Rust actor queue.
//! The TCP reader can only report "refused" after the writer acknowledges the
//! actual busy response. A failed refusal instead closes the peer generation
//! and surfaces its failure on the terminal event.
use std::sync::mpsc::{SyncSender, TrySendError};

use crate::transport::{TransportChannel, TransportError, TransportErrorKind};

use super::super::shared::CounterState;

/// `Ok(true)` means forwarded to actor, `Ok(false)` means busy refusal
/// confirmed by the TCP writer. `Err` means neither was delivered.
pub(in crate::transport::socket) fn forward_join_or_refuse<T>(
    sender: &SyncSender<T>,
    counters: &CounterState,
    event: T,
    refuse: impl FnOnce() -> Result<(), TransportError>,
) -> Result<bool, TransportError> {
    match sender.try_send(event) {
        Ok(()) => Ok(true),
        Err(error) => {
            if matches!(error, TrySendError::Full(_)) {
                counters.queue_overflow();
            }
            counters.delivery_failure();
            counters.busy_admission_refusal();
            refuse().map_err(|failure| {
                // Both actor delivery and TCP refusal failed. Never turn a
                // failed outbound control write into apparent admission.
                counters.delivery_failure();
                TransportError::new(
                    TransportErrorKind::Delivery,
                    TransportChannel::Control,
                    format!("failed to deliver join busy refusal: {}", failure.message),
                )
            })?;
            Ok(false)
        }
    }
}
