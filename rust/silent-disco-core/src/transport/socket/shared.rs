#![allow(
    clippy::too_many_lines,
    clippy::manual_let_else,
    clippy::single_match_else
)]
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{
    DecodePolicy, FRAME_HEADER_BYTES, MAX_CONTROL_PAYLOAD_BYTES, ProtocolDecoder, ProtocolFrame,
    decode_header,
};

use super::super::{TransportChannel, TransportError, TransportErrorKind, TransportEvent};

#[path = "event_delivery_class.rs"]
mod event_delivery_class;
pub(super) use event_delivery_class::is_best_effort_event;

#[path = "counters.rs"]
mod counters;
pub(super) use counters::CounterState;

pub(super) const MAX_CONTROL_FRAME_BYTES: usize = FRAME_HEADER_BYTES + MAX_CONTROL_PAYLOAD_BYTES;

pub(super) fn send_event(
    sender: &SyncSender<TransportEvent>,
    counters: &CounterState,
    event: TransportEvent,
) {
    assert!(
        is_best_effort_event(&event),
        "authoritative events require reliable delivery"
    );
    match sender.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            counters.queue_overflow();
            counters.best_effort_drop();
        }
        Err(TrySendError::Disconnected(_)) => {
            counters.delivery_failure();
            counters.best_effort_drop();
        }
    }
}

/// TCP control events use bounded backpressure instead of telemetry loss.
/// Only the TCP reader may block this way; never UDP or a native callback.
/// Shutdown interrupts a stopped collector without deadlocking teardown.
pub(super) fn send_authoritative_event<T>(
    sender: &SyncSender<T>,
    stop: &AtomicBool,
    counters: &CounterState,
    event: T,
) -> bool {
    send_authoritative_event_until(sender, stop, counters, event, None)
}

/// Admission completion must not wait forever behind a stopped application
/// collector. A timeout is an explicit failed authorization; callers must
/// revoke the provisional route before returning an error.
pub(super) fn send_authoritative_event_bounded<T>(
    sender: &SyncSender<T>,
    stop: &AtomicBool,
    counters: &CounterState,
    event: T,
    timeout: Duration,
) -> bool {
    send_authoritative_event_until(sender, stop, counters, event, Some(timeout))
}

fn send_authoritative_event_until<T>(
    sender: &SyncSender<T>,
    stop: &AtomicBool,
    counters: &CounterState,
    mut event: T,
    timeout: Option<Duration>,
) -> bool {
    let started = Instant::now();
    let mut congested = false;
    loop {
        if stop.load(Ordering::Acquire) {
            // Even an immediately cancelled authoritative event is a failed
            // delivery. Do not hide it merely because the queue was empty.
            counters.delivery_failure();
            counters.authoritative_event_failure();
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(TrySendError::Full(pending)) => {
                if !congested {
                    counters.queue_overflow();
                    congested = true;
                }
                if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                    counters.delivery_failure();
                    counters.authoritative_event_failure();
                    return false;
                }
                event = pending;
                thread::sleep(Duration::from_millis(2));
            }
            Err(TrySendError::Disconnected(_)) => {
                counters.delivery_failure();
                counters.authoritative_event_failure();
                return false;
            }
        }
    }
}

pub(super) struct ControlWrite {
    pub bytes: Vec<u8>,
    pub result: SyncSender<Result<usize, TransportError>>,
    // An acknowledged timeout must not leave an obsolete control frame in
    // the writer's backlog to be transmitted much later. In-flight kernel
    // writes remain ambiguous and are handled by session/generation policy.
    pub deadline: Option<Instant>,
}

#[path = "control_sender.rs"]
mod control_sender;
pub(super) use control_sender::ControlSender;

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
                // A caller may already have timed out while this frame sat
                // behind another TCP write. Never send a stale queued Start,
                // Pause, Stop or approval after its delivery deadline.
                if write
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    counters.delivery_failure();
                    drop(write.result.send(Err(TransportError::new(
                        TransportErrorKind::Timeout,
                        TransportChannel::Control,
                        "queued control frame expired before TCP write",
                    ))));
                    continue;
                }
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
    frame_timeout: Duration,
    expected_session: &crate::domain::SessionId,
    pending_join: Option<(&AtomicBool, Duration)>,
    mut on_outcome: F,
) where
    F: FnMut(ReadControlOutcome) -> bool,
{
    // The absolute per-frame deadline applies even after a peer has joined.
    // A slow-trickling TCP control frame must not reserve a peer slot forever.
    let read_timeout = pending_join
        .map_or(io_timeout, |(_, timeout)| io_timeout.min(timeout))
        .min(frame_timeout);
    if let Err(error) = stream.set_read_timeout(Some(read_timeout)) {
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
    let join_started = Instant::now();
    let mut frame_started: Option<Instant> = None;
    while !stop.load(Ordering::Acquire) {
        // Unjoined TCP peers must not hold all host slots indefinitely. A
        // partial header or slow trickle cannot extend this absolute deadline.
        // Once the first join is forwarded, manual approval may take longer.
        if pending_join.is_some_and(|(joined, deadline)| {
            !joined.load(Ordering::Acquire) && join_started.elapsed() >= deadline
        }) {
            let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::new(
                TransportErrorKind::Timeout,
                TransportChannel::Control,
                "TCP peer did not submit a join request before the admission deadline",
            )));
            break;
        }
        if frame_started.is_some_and(|started| started.elapsed() >= frame_timeout) {
            let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::new(
                TransportErrorKind::Timeout,
                TransportChannel::Control,
                "TCP peer did not complete a control frame before the frame deadline",
            )));
            break;
        }
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
                // A completed read is not evidence that an unjoined peer met
                // its admission deadline: the kernel may return just after it.
                if pending_join.is_some_and(|(joined, deadline)| {
                    !joined.load(Ordering::Acquire) && join_started.elapsed() >= deadline
                }) {
                    let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::new(
                        TransportErrorKind::Timeout,
                        TransportChannel::Control,
                        "TCP peer completed its pre-join read after the admission deadline",
                    )));
                    break;
                }
                if buffer.len().saturating_add(read) > MAX_CONTROL_FRAME_BYTES {
                    let error = TransportError::new(
                        TransportErrorKind::Protocol,
                        TransportChannel::Control,
                        "control receive buffer exceeded the maximum frame size",
                    );
                    let _ = on_outcome(ReadControlOutcome::Rejected(error));
                    break;
                }
                if buffer.is_empty() {
                    frame_started = Some(Instant::now());
                }
                buffer.extend_from_slice(&chunk[..read]);
                // Check again after read: a peer can complete its frame only
                // after the absolute deadline while the socket is blocked.
                if frame_started.is_some_and(|started| started.elapsed() >= frame_timeout) {
                    let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::new(
                        TransportErrorKind::Timeout,
                        TransportChannel::Control,
                        "TCP peer completed a control frame after the frame deadline",
                    )));
                    break;
                }
                loop {
                    // An earlier frame's application delivery can block while
                    // another pre-join frame is already buffered. Never admit
                    // that buffered frame after the absolute join deadline.
                    if pending_join.is_some_and(|(joined, deadline)| {
                        !joined.load(Ordering::Acquire) && join_started.elapsed() >= deadline
                    }) {
                        let _ = on_outcome(ReadControlOutcome::Rejected(TransportError::new(
                            TransportErrorKind::Timeout,
                            TransportChannel::Control,
                            "TCP peer exceeded the admission deadline while frames were buffered",
                        )));
                        return;
                    }
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
                            // Actor backpressure handling the prior frame must not
                            // consume the next frame's network assembly deadline.
                            frame_started = (!buffer.is_empty()).then(Instant::now);
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

/// Reap completed per-peer reader/writer threads while the host remains live.
/// A busy venue can reconnect thousands of times; retaining every finished
/// `JoinHandle` until final shutdown would otherwise grow the worker registry
/// for the entire session. Never join a still-running worker or hold the
/// registry mutex while joining, and propagate panics to the host owner.
pub(super) fn reap_finished_workers(
    handles: &Mutex<Vec<thread::JoinHandle<()>>>,
) -> Result<(), TransportError> {
    let completed = {
        let mut registry = handles.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "transport worker registry is poisoned",
            )
        })?;
        let (completed, active): (Vec<_>, Vec<_>) = core::mem::take(&mut *registry)
            .into_iter()
            .partition(thread::JoinHandle::is_finished);
        *registry = active;
        completed
    };
    let mut panicked = false;
    for handle in completed {
        if handle.join().is_err() {
            panicked = true;
        }
    }
    if panicked {
        Err(TransportError::new(
            TransportErrorKind::WorkerPanicked,
            TransportChannel::Runtime,
            "a completed transport worker panicked",
        ))
    } else {
        Ok(())
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

#[cfg(test)]
mod terminal_event_pressure_tests {
    use super::{CounterState, send_authoritative_event};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::sync_channel;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn saturated_event_queue_delivers_terminal_after_existing_event() {
        let (sender, receiver) = sync_channel(1);
        sender.send(1_u8).expect("fill bounded queue");
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CounterState::default());
        let producer = {
            let stop = stop.clone();
            let counters = counters.clone();
            thread::spawn(move || send_authoritative_event(&sender, &stop, &counters, 2_u8))
        };

        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), 2);
        assert!(producer.join().expect("terminal sender"));
        assert_eq!(counters.snapshot().delivery_failures, 0);
    }

    #[test]
    fn shutdown_interrupts_terminal_wait_without_displacing_queued_event() {
        let (sender, receiver) = sync_channel(1);
        sender.send(1_u8).expect("fill bounded queue");
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CounterState::default());
        let (started_sender, started_receiver) = sync_channel(0);
        let producer = {
            let stop = stop.clone();
            let counters = counters.clone();
            thread::spawn(move || {
                started_sender.send(()).expect("signal started");
                send_authoritative_event(&sender, &stop, &counters, 2_u8)
            })
        };

        started_receiver.recv().expect("producer started");
        stop.store(true, Ordering::Release);
        assert!(!producer.join().expect("sender exits after shutdown"));
        assert_eq!(receiver.try_recv().expect("existing event retained"), 1);
        assert!(receiver.try_recv().is_err());
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

#[cfg(test)]
#[path = "control_frame_deadline_tests.rs"]
mod control_frame_deadline_tests;

#[cfg(test)]
#[path = "control_sender_pressure_tests.rs"]
mod control_sender_pressure_tests;
