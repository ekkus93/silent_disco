use core::fmt;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::decoder::StreamingDecodeHandle;
use super::packetizer::{Packetizer, PacketizerError, PacketizerErrorKind};
use crate::domain::{MonotonicMillis, SessionId, StreamId};
use crate::protocol::ProtocolFrame;

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Default bounded output-queue capacity for one packetizer worker.
pub const DEFAULT_PACKETIZER_QUEUE_CAPACITY: usize = 32;
/// Largest caller-selectable bounded output-queue capacity.
pub const MAX_PACKETIZER_QUEUE_CAPACITY: usize = 256;

/// Resource bounds for one streaming packetizer worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingPacketizeConfig {
    /// Packet duration in milliseconds; see [`super::DEFAULT_PACKET_DURATION_MS`].
    pub packet_duration_ms: u32,
    /// Maximum audio datagrams buffered between the worker and consumer.
    pub queue_capacity: usize,
}

impl Default for StreamingPacketizeConfig {
    fn default() -> Self {
        Self {
            packet_duration_ms: super::packetizer::DEFAULT_PACKET_DURATION_MS,
            queue_capacity: DEFAULT_PACKETIZER_QUEUE_CAPACITY,
        }
    }
}

/// Lifecycle state of one packetizer worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketizerWorkerState {
    /// Worker is packetizing or blocked by bounded backpressure.
    Running,
    /// Worker emitted the final packet and exited successfully.
    Completed,
    /// Worker observed explicit cancellation or consumer disconnect.
    Cancelled,
    /// Worker exited with a typed failure.
    Failed,
}

/// Stable failure taxonomy for one packetizer worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketizerWorkerErrorKind {
    /// The underlying decode worker failed or was cancelled.
    Source,
    /// A chunk violated the packetizer's own contract (see [`PacketizerErrorKind`]).
    Packetize,
    /// Worker was explicitly cancelled or lost its consumer.
    Cancelled,
    /// Worker thread panicked or violated an internal invariant.
    WorkerPanicked,
}

/// Typed packetizer-worker failure with a user-safe diagnostic message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketizerWorkerError {
    /// Stable semantic failure category.
    pub kind: PacketizerWorkerErrorKind,
    /// Human-readable diagnostic without native path disclosure.
    pub message: String,
}

impl PacketizerWorkerError {
    fn new(kind: PacketizerWorkerErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for PacketizerWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for PacketizerWorkerError {}

impl From<PacketizerError> for PacketizerWorkerError {
    fn from(error: PacketizerError) -> Self {
        let kind = match error.kind {
            PacketizerErrorKind::InvalidConfiguration
            | PacketizerErrorKind::FormatMismatch
            | PacketizerErrorKind::AlreadyFinished => PacketizerWorkerErrorKind::Packetize,
        };
        Self::new(kind, error.message)
    }
}

/// Terminal worker accounting returned by [`StreamingPacketizeHandle::join`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketizerSummary {
    /// Terminal worker state.
    pub state: PacketizerWorkerState,
    /// Total audio datagrams handed to the bounded channel.
    pub emitted_packets: u64,
    /// Number of packets that observed bounded backpressure.
    pub backpressure_events: u64,
}

#[derive(Default)]
struct SharedStatistics {
    queued_packets: AtomicUsize,
    backpressure_events: AtomicU64,
    emitted_packets: AtomicU64,
}

/// Owner of one cancellable packetizer worker and its bounded frame receiver.
///
/// Dropping the handle cancels and joins the worker and its underlying
/// decoder. Normal completion should consume through the final packet
/// before calling [`Self::join`].
#[must_use = "dropping the handle immediately cancels the packetizer worker"]
pub struct StreamingPacketizeHandle {
    receiver: Receiver<ProtocolFrame>,
    cancellation: Arc<AtomicBool>,
    statistics: Arc<SharedStatistics>,
    config: StreamingPacketizeConfig,
    stream_start: ProtocolFrame,
    join: Option<JoinHandle<Result<PacketizerSummary, PacketizerWorkerError>>>,
}

impl StreamingPacketizeHandle {
    /// Starts one bounded packetizer worker consuming an already-opened
    /// decode worker.
    ///
    /// # Errors
    ///
    /// Returns a typed [`PacketizerWorkerError`] if the packetizer's own
    /// configuration is invalid for the decoder's format (see
    /// [`Packetizer::new`]) or the worker thread could not be started.
    pub fn spawn(
        decoder: StreamingDecodeHandle,
        session_id: SessionId,
        stream_id: StreamId,
        host_start_time_ms: MonotonicMillis,
        config: StreamingPacketizeConfig,
    ) -> Result<Self, PacketizerWorkerError> {
        let format = decoder.stream_info().format;
        let packetizer = Packetizer::new(
            session_id,
            stream_id,
            format,
            config.packet_duration_ms,
            host_start_time_ms,
        )?;
        let stream_start = packetizer.stream_start_message();
        let (sender, receiver) = sync_channel(config.queue_capacity);
        let cancellation = Arc::new(AtomicBool::new(false));
        let statistics = Arc::new(SharedStatistics::default());
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_statistics = Arc::clone(&statistics);
        let join = thread::Builder::new()
            .name("silent-disco-streaming-packetizer".to_owned())
            .spawn(move || {
                run_packetizer_worker(
                    decoder,
                    packetizer,
                    &sender,
                    config,
                    &worker_cancellation,
                    &worker_statistics,
                )
            })
            .map_err(|error| {
                PacketizerWorkerError::new(
                    PacketizerWorkerErrorKind::WorkerPanicked,
                    format!("could not start the streaming packetizer worker: {error}"),
                )
            })?;
        Ok(Self {
            receiver,
            cancellation,
            statistics,
            config,
            stream_start,
            join: Some(join),
        })
    }

    /// Returns the exact `StreamStart` control message for this stream.
    #[must_use]
    pub const fn stream_start_message(&self) -> &ProtocolFrame {
        &self.stream_start
    }

    /// Returns a lock-free snapshot of queue pressure and worker progress.
    #[must_use]
    pub fn statistics(&self) -> (usize, usize, u64, u64) {
        (
            self.statistics.queued_packets.load(Ordering::Acquire),
            self.config.queue_capacity,
            self.statistics.backpressure_events.load(Ordering::Acquire),
            self.statistics.emitted_packets.load(Ordering::Acquire),
        )
    }

    /// Receives the next bounded audio datagram with a caller-selected timeout.
    ///
    /// # Errors
    ///
    /// Returns [`RecvTimeoutError::Timeout`] when no packet arrives in time,
    /// or `Disconnected` after worker exit and queue drain.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<ProtocolFrame, RecvTimeoutError> {
        let frame = self.receiver.recv_timeout(timeout)?;
        self.statistics
            .queued_packets
            .fetch_sub(1, Ordering::AcqRel);
        Ok(frame)
    }

    /// Attempts to receive one audio datagram without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`TryRecvError::Empty`] when no packet is ready, or
    /// `Disconnected` after worker exit and queue drain.
    pub fn try_recv(&self) -> Result<ProtocolFrame, TryRecvError> {
        let frame = self.receiver.try_recv()?;
        self.statistics
            .queued_packets
            .fetch_sub(1, Ordering::AcqRel);
        Ok(frame)
    }

    /// Requests cooperative cancellation. Use [`Self::cancel_and_join`] when
    /// a terminal result is required immediately.
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    /// Joins a worker expected to have completed after its final packet.
    ///
    /// # Errors
    ///
    /// Returns the typed worker failure or a panic-mapped error. Calling
    /// this before draining a full queue can block indefinitely; use
    /// [`Self::cancel_and_join`] for early termination.
    pub fn join(mut self) -> Result<PacketizerSummary, PacketizerWorkerError> {
        self.join_worker()
    }

    /// Cancels the worker and joins it deterministically.
    ///
    /// # Errors
    ///
    /// Returns [`PacketizerWorkerErrorKind::Cancelled`] after ordinary
    /// cancellation, or a more specific terminal failure if one occurred
    /// first.
    pub fn cancel_and_join(mut self) -> Result<PacketizerSummary, PacketizerWorkerError> {
        self.cancel();
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<PacketizerSummary, PacketizerWorkerError> {
        let Some(join) = self.join.take() else {
            return Err(PacketizerWorkerError::new(
                PacketizerWorkerErrorKind::WorkerPanicked,
                "streaming packetizer worker was already joined",
            ));
        };
        join.join().map_err(|_| {
            PacketizerWorkerError::new(
                PacketizerWorkerErrorKind::WorkerPanicked,
                "streaming packetizer worker panicked",
            )
        })?
    }
}

impl Drop for StreamingPacketizeHandle {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn ensure_not_cancelled(cancellation: &AtomicBool) -> Result<(), PacketizerWorkerError> {
    if cancellation.load(Ordering::Acquire) {
        return Err(PacketizerWorkerError::new(
            PacketizerWorkerErrorKind::Cancelled,
            "streaming packetizer worker was cancelled",
        ));
    }
    Ok(())
}

fn run_packetizer_worker(
    decoder: StreamingDecodeHandle,
    mut packetizer: Packetizer,
    sender: &SyncSender<ProtocolFrame>,
    config: StreamingPacketizeConfig,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
) -> Result<PacketizerSummary, PacketizerWorkerError> {
    let result = packetize_stream(
        &decoder,
        &mut packetizer,
        sender,
        config,
        cancellation,
        statistics,
    );
    decoder.cancel();
    drop(decoder.cancel_and_join());
    result
}

fn packetize_stream(
    decoder: &StreamingDecodeHandle,
    packetizer: &mut Packetizer,
    sender: &SyncSender<ProtocolFrame>,
    config: StreamingPacketizeConfig,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
) -> Result<PacketizerSummary, PacketizerWorkerError> {
    loop {
        ensure_not_cancelled(cancellation)?;
        let chunk = match decoder.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(PacketizerWorkerError::new(
                    PacketizerWorkerErrorKind::Source,
                    "streaming decode worker disconnected before end-of-stream",
                ));
            }
        };
        let outcome = packetizer.push_chunk(chunk)?;
        for frame in outcome.frames {
            send_with_backpressure(
                sender,
                frame,
                cancellation,
                statistics,
                config.queue_capacity,
            )?;
        }
        if outcome.end_of_stream {
            return Ok(PacketizerSummary {
                state: PacketizerWorkerState::Completed,
                emitted_packets: statistics.emitted_packets.load(Ordering::Acquire),
                backpressure_events: statistics.backpressure_events.load(Ordering::Acquire),
            });
        }
    }
}

fn send_with_backpressure(
    sender: &SyncSender<ProtocolFrame>,
    frame: ProtocolFrame,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
    queue_capacity: usize,
) -> Result<(), PacketizerWorkerError> {
    let mut backpressure_recorded = false;
    let mut pending = Some(frame);
    loop {
        ensure_not_cancelled(cancellation)?;
        let reserved =
            statistics
                .queued_packets
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    (queued < queue_capacity).then_some(queued + 1)
                });
        if reserved.is_err() {
            if !backpressure_recorded {
                statistics
                    .backpressure_events
                    .fetch_add(1, Ordering::AcqRel);
                backpressure_recorded = true;
            }
            thread::sleep(BACKPRESSURE_POLL_INTERVAL);
            continue;
        }
        match sender.try_send(pending.take().expect("frame present until sent")) {
            Ok(()) => {
                statistics.emitted_packets.fetch_add(1, Ordering::AcqRel);
                return Ok(());
            }
            Err(std::sync::mpsc::TrySendError::Full(returned)) => {
                statistics.queued_packets.fetch_sub(1, Ordering::AcqRel);
                pending = Some(returned);
                thread::sleep(BACKPRESSURE_POLL_INTERVAL);
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                statistics.queued_packets.fetch_sub(1, Ordering::AcqRel);
                return Err(PacketizerWorkerError::new(
                    PacketizerWorkerErrorKind::Cancelled,
                    "packetizer output receiver disconnected",
                ));
            }
        }
    }
}
