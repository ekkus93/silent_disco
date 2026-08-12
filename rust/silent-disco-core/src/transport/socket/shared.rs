#![allow(
    clippy::too_many_lines,
    clippy::manual_let_else,
    clippy::single_match_else
)]
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::protocol::{
    DecodePolicy, FRAME_HEADER_BYTES, MAX_CONTROL_PAYLOAD_BYTES, ProtocolDecoder, ProtocolFrame,
    decode_header,
};

use super::super::{
    TransportChannel, TransportCounters, TransportError, TransportErrorKind, TransportEvent,
};

pub(super) const MAX_CONTROL_FRAME_BYTES: usize = FRAME_HEADER_BYTES + MAX_CONTROL_PAYLOAD_BYTES;

#[derive(Debug, Default)]
pub(super) struct CounterState {
    accepted_control_connections: AtomicU64,
    closed_control_connections: AtomicU64,
    control_frames_sent: AtomicU64,
    control_frames_received: AtomicU64,
    sync_datagrams_sent: AtomicU64,
    sync_datagrams_received: AtomicU64,
    audio_datagrams_sent: AtomicU64,
    audio_datagrams_received: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    malformed_frames: AtomicU64,
    oversized_frames: AtomicU64,
    unauthorized_frames: AtomicU64,
    queue_overflows: AtomicU64,
    delivery_failures: AtomicU64,
}

impl CounterState {
    pub(super) fn snapshot(&self) -> TransportCounters {
        TransportCounters {
            accepted_control_connections: self.accepted_control_connections.load(Ordering::Relaxed),
            closed_control_connections: self.closed_control_connections.load(Ordering::Relaxed),
            control_frames_sent: self.control_frames_sent.load(Ordering::Relaxed),
            control_frames_received: self.control_frames_received.load(Ordering::Relaxed),
            sync_datagrams_sent: self.sync_datagrams_sent.load(Ordering::Relaxed),
            sync_datagrams_received: self.sync_datagrams_received.load(Ordering::Relaxed),
            audio_datagrams_sent: self.audio_datagrams_sent.load(Ordering::Relaxed),
            audio_datagrams_received: self.audio_datagrams_received.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            malformed_frames: self.malformed_frames.load(Ordering::Relaxed),
            oversized_frames: self.oversized_frames.load(Ordering::Relaxed),
            unauthorized_frames: self.unauthorized_frames.load(Ordering::Relaxed),
            queue_overflows: self.queue_overflows.load(Ordering::Relaxed),
            delivery_failures: self.delivery_failures.load(Ordering::Relaxed),
        }
    }

    pub(super) fn accepted(&self) {
        self.accepted_control_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn closed(&self) {
        self.closed_control_connections
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn frame_sent(&self, bytes: usize) {
        self.control_frames_sent.fetch_add(1, Ordering::Relaxed);
        self.add_sent(bytes);
    }

    pub(super) fn frame_received(&self, bytes: usize) {
        self.control_frames_received.fetch_add(1, Ordering::Relaxed);
        self.add_received(bytes);
    }

    pub(super) fn datagram_sent(&self, channel: TransportChannel, bytes: usize) {
        match channel {
            TransportChannel::Synchronization => {
                self.sync_datagrams_sent.fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Audio => {
                self.audio_datagrams_sent.fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Control | TransportChannel::Runtime => {}
        }
        self.add_sent(bytes);
    }

    pub(super) fn datagram_received(&self, channel: TransportChannel, bytes: usize) {
        match channel {
            TransportChannel::Synchronization => {
                self.sync_datagrams_received.fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Audio => {
                self.audio_datagrams_received
                    .fetch_add(1, Ordering::Relaxed);
            }
            TransportChannel::Control | TransportChannel::Runtime => {}
        }
        self.add_received(bytes);
    }

    pub(super) fn malformed(&self) {
        self.malformed_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn oversized(&self) {
        self.oversized_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn unauthorized(&self) {
        self.unauthorized_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn queue_overflow(&self) {
        self.queue_overflows.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn delivery_failure(&self) {
        self.delivery_failures.fetch_add(1, Ordering::Relaxed);
    }

    fn add_sent(&self, bytes: usize) {
        self.bytes_sent
            .fetch_add(saturating_u64(bytes), Ordering::Relaxed);
    }

    fn add_received(&self, bytes: usize) {
        self.bytes_received
            .fetch_add(saturating_u64(bytes), Ordering::Relaxed);
    }
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub(super) fn send_event(
    sender: &SyncSender<TransportEvent>,
    counters: &CounterState,
    event: TransportEvent,
) {
    if sender.try_send(event).is_err() {
        counters.queue_overflow();
    }
}

pub(super) struct ControlWrite {
    pub bytes: Vec<u8>,
    pub result: SyncSender<Result<usize, TransportError>>,
}

pub(super) struct ControlSender {
    sender: SyncSender<ControlWrite>,
    operation_timeout: Duration,
    counters: Arc<CounterState>,
}

impl ControlSender {
    pub(super) fn new(
        sender: SyncSender<ControlWrite>,
        operation_timeout: Duration,
        counters: Arc<CounterState>,
    ) -> Self {
        Self {
            sender,
            operation_timeout,
            counters,
        }
    }

    pub(super) fn send(&self, bytes: Vec<u8>) -> Result<usize, TransportError> {
        let (result_sender, result_receiver) = sync_channel(1);
        match self.sender.try_send(ControlWrite {
            bytes,
            result: result_sender,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.counters.queue_overflow();
                return Err(TransportError::new(
                    TransportErrorKind::QueueFull,
                    TransportChannel::Control,
                    "control send queue is full",
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(TransportError::new(
                    TransportErrorKind::ShuttingDown,
                    TransportChannel::Control,
                    "control writer is stopped",
                ));
            }
        }
        match result_receiver.recv_timeout(self.operation_timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(TransportError::new(
                TransportErrorKind::Timeout,
                TransportChannel::Control,
                "timed out waiting for control delivery",
            )),
            Err(RecvTimeoutError::Disconnected) => Err(TransportError::new(
                TransportErrorKind::ShuttingDown,
                TransportChannel::Control,
                "control writer stopped before reporting delivery",
            )),
        }
    }
}

pub(super) fn spawn_control_writer(
    mut stream: TcpStream,
    receiver: Receiver<ControlWrite>,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    io_timeout: Duration,
    name: String,
) -> Result<thread::JoinHandle<()>, TransportError> {
    stream
        .set_write_timeout(Some(io_timeout))
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Io,
                TransportChannel::Control,
                "failed to set control write timeout",
                &error,
            )
        })?;
    thread::Builder::new()
        .name(name)
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let write = match receiver.recv_timeout(io_timeout) {
                    Ok(write) => write,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let result = stream
                    .write_all(&write.bytes)
                    .and_then(|()| stream.flush())
                    .map(|()| write.bytes.len())
                    .map_err(|error| {
                        counters.delivery_failure();
                        TransportError::io(
                            TransportErrorKind::Delivery,
                            TransportChannel::Control,
                            "failed to write control frame",
                            &error,
                        )
                    });
                if let Ok(bytes) = result {
                    counters.frame_sent(bytes);
                }
                let failed = result.is_err();
                drop(write.result.send(result));
                if failed {
                    break;
                }
            }
            drop(stream.shutdown(Shutdown::Both));
        })
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to spawn control writer",
                &error,
            )
        })
}

pub(super) enum ReadControlOutcome {
    Frame(ProtocolFrame, usize),
    Closed,
    Rejected(TransportError),
}

pub(super) fn read_control_loop<F>(
    mut stream: TcpStream,
    stop: &AtomicBool,
    io_timeout: Duration,
    expected_session: &crate::domain::SessionId,
    mut on_outcome: F,
) where
    F: FnMut(ReadControlOutcome) -> bool,
{
    if let Err(error) = stream.set_read_timeout(Some(io_timeout)) {
        let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::io(
            TransportErrorKind::Io,
            TransportChannel::Control,
            "failed to set control read timeout",
            &error,
        )));
        return;
    }
    let mut decoder = ProtocolDecoder::default();
    let mut buffer = Vec::with_capacity(4_096);
    let mut chunk = [0_u8; 8_192];
    while !stop.load(Ordering::Acquire) {
        match stream.read(&mut chunk) {
            Ok(0) => {
                if buffer.is_empty() {
                    let _ = on_outcome(ReadControlOutcome::Closed);
                } else {
                    let error = TransportError::new(
                        TransportErrorKind::Protocol,
                        TransportChannel::Control,
                        "control connection closed with a truncated frame",
                    );
                    let _ = on_outcome(ReadControlOutcome::Rejected(error));
                }
                break;
            }
            Ok(read) => {
                if buffer.len().saturating_add(read) > MAX_CONTROL_FRAME_BYTES {
                    let error = TransportError::new(
                        TransportErrorKind::Protocol,
                        TransportChannel::Control,
                        "control receive buffer exceeded the maximum frame size",
                    );
                    let _ = on_outcome(ReadControlOutcome::Rejected(error));
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
                loop {
                    if buffer.len() < FRAME_HEADER_BYTES {
                        break;
                    }
                    let header = match decode_header(&buffer[..FRAME_HEADER_BYTES]) {
                        Ok(header) => header,
                        Err(error) => {
                            let keep_running = on_outcome(ReadControlOutcome::Rejected(
                                TransportError::protocol(TransportChannel::Control, &error),
                            ));
                            if !keep_running {
                                return;
                            }
                            return;
                        }
                    };
                    if !header.kind.is_control() {
                        let error = TransportError::new(
                            TransportErrorKind::Protocol,
                            TransportChannel::Control,
                            "non-control protocol frame received on TCP control channel",
                        );
                        let _ = on_outcome(ReadControlOutcome::Rejected(error));
                        return;
                    }
                    let payload_length = match usize::try_from(header.payload_length) {
                        Ok(length) => length,
                        Err(_) => {
                            let error = TransportError::new(
                                TransportErrorKind::Protocol,
                                TransportChannel::Control,
                                "control payload length cannot be represented on this platform",
                            );
                            let _ = on_outcome(ReadControlOutcome::Rejected(error));
                            return;
                        }
                    };
                    let total = FRAME_HEADER_BYTES.saturating_add(payload_length);
                    if buffer.len() < total {
                        break;
                    }
                    let frame_bytes: Vec<u8> = buffer.drain(..total).collect();
                    let result = decoder.decode(
                        &frame_bytes,
                        DecodePolicy {
                            expected_session_id: Some(expected_session),
                            minimum_audio_sequence: None,
                        },
                    );
                    match result {
                        Ok(frame) => {
                            if !on_outcome(ReadControlOutcome::Frame(frame, total)) {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = on_outcome(ReadControlOutcome::Rejected(
                                TransportError::protocol(TransportChannel::Control, &error),
                            ));
                            return;
                        }
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => {
                let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Control,
                    "failed to read control connection",
                    &error,
                )));
                break;
            }
        }
    }
    drop(stream.shutdown(Shutdown::Both));
}

pub(super) fn shutdown_stream(stream: &Mutex<TcpStream>) -> Result<(), TransportError> {
    let stream = stream.lock().map_err(|_| {
        TransportError::new(
            TransportErrorKind::WorkerPanicked,
            TransportChannel::Control,
            "control stream shutdown lock is poisoned",
        )
    })?;
    match stream.shutdown(Shutdown::Both) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotConnected => Ok(()),
        Err(error) => Err(TransportError::io(
            TransportErrorKind::Io,
            TransportChannel::Control,
            "failed to shut down control stream",
            &error,
        )),
    }
}

pub(super) fn join_workers(
    handles: &Mutex<Vec<thread::JoinHandle<()>>>,
) -> Result<(), TransportError> {
    let mut panicked = false;
    loop {
        let batch = {
            let mut handles = handles.lock().map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "transport worker registry is poisoned",
                )
            })?;
            core::mem::take(&mut *handles)
        };
        if batch.is_empty() {
            break;
        }
        for handle in batch {
            if handle.join().is_err() {
                panicked = true;
            }
        }
    }
    if panicked {
        Err(TransportError::new(
            TransportErrorKind::WorkerPanicked,
            TransportChannel::Runtime,
            "one or more transport workers panicked",
        ))
    } else {
        Ok(())
    }
}

/// Converts an implicit transport shutdown failure into a fail-loud lifecycle
/// violation instead of discarding it from `Drop`. Explicit owners still call
/// `shutdown()` directly and receive the original structured error. During an
/// unrelated panic we avoid double-panicking while unwinding; the process is
/// already visibly failing and `shutdown()` has still attempted every join.
pub(super) fn fail_loud_on_implicit_shutdown(
    result: &Result<(), TransportError>,
    owner: &'static str,
) {
    if let Err(error) = result {
        assert!(
            thread::panicking(),
            "{owner} dropped after shutdown failure: {error}"
        );
    }
}

#[cfg(test)]
mod implicit_shutdown_tests {
    use super::fail_loud_on_implicit_shutdown;
    use crate::transport::{TransportChannel, TransportError, TransportErrorKind};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn implicit_shutdown_failure_is_not_silently_discarded() {
        let failure = TransportError::new(
            TransportErrorKind::WorkerPanicked,
            TransportChannel::Runtime,
            "injected transport worker panic",
        );
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            fail_loud_on_implicit_shutdown(&Err(failure), "test transport");
        }));
        assert!(
            outcome.is_err(),
            "an implicit transport shutdown failure must fail loudly"
        );
    }

    #[test]
    fn clean_implicit_shutdown_remains_a_no_op() {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            fail_loud_on_implicit_shutdown(&Ok(()), "test transport");
        }));
        assert!(outcome.is_ok());
    }
}

pub(super) fn recv_event(
    receiver: &Receiver<TransportEvent>,
    timeout: Duration,
) -> Result<TransportEvent, TransportError> {
    match receiver.recv_timeout(timeout) {
        Ok(event) => Ok(event),
        Err(RecvTimeoutError::Timeout) => Err(TransportError::new(
            TransportErrorKind::Timeout,
            TransportChannel::Runtime,
            "timed out waiting for a transport event",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(TransportError::new(
            TransportErrorKind::ShuttingDown,
            TransportChannel::Runtime,
            "transport event channel is closed",
        )),
    }
}

pub(super) fn classify_rejection(counters: &CounterState, error: &TransportError) {
    match error.kind {
        TransportErrorKind::Unauthorized => counters.unauthorized(),
        TransportErrorKind::Protocol => {
            if error.message.contains("large") || error.message.contains("maximum") {
                counters.oversized();
            } else {
                counters.malformed();
            }
        }
        _ => {}
    }
}
