//! Bounded, deadline-aware authoritative TCP control sender.
use super::{ControlWrite, CounterState};
use crate::transport::{TransportChannel, TransportError, TransportErrorKind};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct ControlSender {
    sender: SyncSender<ControlWrite>,
    // Multiple callers share this TCP writer. A control waiting for queue
    // capacity must not be overtaken by a later concurrent control.
    send_gate: Mutex<()>,
    operation_timeout: Duration,
    counters: Arc<CounterState>,
}

impl ControlSender {
    pub(crate) fn new(
        sender: SyncSender<ControlWrite>,
        operation_timeout: Duration,
        counters: Arc<CounterState>,
    ) -> Self {
        Self {
            sender,
            send_gate: Mutex::new(()),
            operation_timeout,
            counters,
        }
    }

    /// Control frames are authoritative: allow a bounded opportunity for the
    /// writer to drain its queue rather than failing immediately under a
    /// transient audio/control burst. The single deadline covers both queue
    /// admission and the TCP acknowledgement; no retry may extend it.
    pub(crate) fn send(&self, bytes: Vec<u8>) -> Result<usize, TransportError> {
        let started = Instant::now();
        // The writer queue itself is FIFO, but retrying try_send on a full
        // queue is not: a later caller can win the freed slot and overtake
        // an earlier Pause/Stop. Serialize control producers for this peer.
        // Waiting for the gate consumes the SAME finite delivery budget.
        let _send_guard = loop {
            match self.send_gate.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::WouldBlock) => {
                    let remaining = self.operation_timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        self.counters.delivery_failure();
                        return Err(TransportError::new(
                            TransportErrorKind::Timeout,
                            TransportChannel::Control,
                            "control delivery deadline expired behind an earlier control",
                        ));
                    }
                    thread::sleep(remaining.min(Duration::from_millis(2)));
                }
                Err(TryLockError::Poisoned(_)) => {
                    self.counters.delivery_failure();
                    return Err(TransportError::new(
                        TransportErrorKind::WorkerPanicked,
                        TransportChannel::Control,
                        "control send ordering gate is poisoned",
                    ));
                }
            }
        };
        if started.elapsed() >= self.operation_timeout {
            self.counters.delivery_failure();
            return Err(TransportError::new(
                TransportErrorKind::Timeout,
                TransportChannel::Control,
                "control delivery deadline expired before queue admission",
            ));
        }
        let (result_sender, result_receiver) = sync_channel(1);
        let mut pending = ControlWrite {
            bytes,
            result: result_sender,
            deadline: started.checked_add(self.operation_timeout),
        };
        let mut congested = false;
        loop {
            match self.sender.try_send(pending) {
                Ok(()) => break,
                Err(TrySendError::Full(write)) => {
                    if !congested {
                        self.counters.queue_overflow();
                        congested = true;
                    }
                    let remaining = self.operation_timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        self.counters.delivery_failure();
                        return Err(TransportError::new(
                            TransportErrorKind::QueueFull,
                            TransportChannel::Control,
                            "control send queue remained full until the delivery deadline",
                        ));
                    }
                    pending = write;
                    thread::sleep(remaining.min(Duration::from_millis(2)));
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.counters.delivery_failure();
                    return Err(TransportError::new(
                        TransportErrorKind::ShuttingDown,
                        TransportChannel::Control,
                        "control writer is stopped",
                    ));
                }
            }
        }
        // Waiting for a queue slot consumes part of the same finite budget.
        // The writer separately checks this deadline before starting TCP IO.
        let remaining = self.operation_timeout.saturating_sub(started.elapsed());
        match result_receiver.recv_timeout(remaining) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                // The writer owns expired-queued-frame failure accounting.
                // Counting the caller's wait here as well would double-count
                // one frame when the writer eventually retires its deadline.
                Err(TransportError::new(
                    TransportErrorKind::Timeout,
                    TransportChannel::Control,
                    "timed out waiting for control delivery",
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.counters.delivery_failure();
                Err(TransportError::new(
                    TransportErrorKind::ShuttingDown,
                    TransportChannel::Control,
                    "control writer stopped before reporting delivery",
                ))
            }
        }
    }
}
