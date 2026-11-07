//! Frame I/O shared by the host and listener TLS control pumps.
//!
//! A pump owns its session and runs on one thread. Each pass drains queued
//! outbound frames and waits briefly for one inbound frame, so the channel is
//! never shared across threads and a queued write waits at most one poll.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use super::super::secure_control_session::{SecureControlSession, SecureSessionError};
use super::super::secure_frame_channel::SecureChannelError;
use super::super::{TransportChannel, TransportError, TransportErrorKind};
use super::shared::{ControlWrite, CounterState, ReadControlOutcome};

/// Longest one pass waits for inbound bytes.
pub(super) const PUMP_POLL: Duration = Duration::from_millis(5);

/// Waits up to `PUMP_POLL` for one inbound frame. Returns `None` when nothing
/// arrived, so the caller can drain writes and check its stop flag.
pub(super) fn receive_outcome(session: &mut SecureControlSession) -> Option<ReadControlOutcome> {
    match session.recv_frame_sized(PUMP_POLL) {
        Ok((frame, bytes)) => Some(ReadControlOutcome::Frame(frame, bytes)),
        Err(SecureSessionError::Channel(SecureChannelError::Io(error)))
            if is_poll_timeout(&error) =>
        {
            None
        }
        Err(SecureSessionError::Channel(SecureChannelError::Closed)) => {
            Some(ReadControlOutcome::Closed)
        }
        Err(error) => Some(ReadControlOutcome::Rejected(TransportError::new(
            TransportErrorKind::Protocol,
            TransportChannel::Control,
            error.to_string(),
        ))),
    }
}

/// Sends queued control frames over the session. Each acknowledgement reports
/// that the frame was queued on the TLS channel, which is the same point the
/// plaintext writer reports after its socket write.
pub(super) fn drain_writes(
    session: &SecureControlSession,
    writes: &Receiver<ControlWrite>,
    counters: &CounterState,
) -> Result<(), TransportError> {
    loop {
        let write = match writes.try_recv() {
            Ok(write) => write,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
        };
        // A caller that already timed out must not see a stale frame sent later.
        if write
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            counters.delivery_failure();
            drop(write.result.send(Err(TransportError::new(
                TransportErrorKind::Timeout,
                TransportChannel::Control,
                "queued control frame expired before TLS write",
            ))));
            continue;
        }
        let length = write.bytes.len();
        let result = session
            .send_encoded(write.bytes)
            .map(|()| length)
            .map_err(|error| {
                counters.delivery_failure();
                TransportError::new(
                    TransportErrorKind::Delivery,
                    TransportChannel::Control,
                    format!("failed to write control frame over TLS: {error}"),
                )
            });
        let failure = result.as_ref().err().cloned();
        if let Ok(bytes) = result.as_ref() {
            counters.frame_sent(*bytes);
        }
        drop(write.result.send(result));
        if let Some(error) = failure {
            return Err(error);
        }
    }
}

pub(super) fn is_poll_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}
