use super::resampler::StereoResampler;
use super::source::{
    MAX_DECODED_PACKET_FRAMES, OpenedDecoder, map_symphonia_error, validate_declared_format,
};
use super::types::{
    AudioFormat, DecodeError, DecodeErrorKind, DecodeStatistics, DecodeStreamInfo, DecodeSummary,
    DecodeWorkerState, DecodedPcmChunk, StreamingDecodeConfig,
};
use crate::domain::SampleIndex;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{
    Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use symphonia::core::audio::sample::Sample;

const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(2);
static ACTIVE_DECODE_WORKERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct SharedStatistics {
    state: AtomicUsize,
    queued_chunks: AtomicUsize,
    backpressure_events: AtomicU64,
    emitted_frames: AtomicU64,
}

impl SharedStatistics {
    fn set_state(&self, state: DecodeWorkerState) {
        self.state.store(state_to_usize(state), Ordering::Release);
    }

    fn snapshot(&self, config: StreamingDecodeConfig) -> DecodeStatistics {
        DecodeStatistics {
            state: usize_to_state(self.state.load(Ordering::Acquire)),
            queued_chunks: self.queued_chunks.load(Ordering::Acquire),
            queue_capacity_chunks: config.queue_chunks,
            maximum_queued_duration_ms: config.maximum_queued_duration_ms(),
            backpressure_events: self.backpressure_events.load(Ordering::Acquire),
            emitted_frames: self.emitted_frames.load(Ordering::Acquire),
        }
    }

    fn decrement_queued(&self) {
        let result =
            self.queued_chunks
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_sub(1)
                });
        debug_assert!(result.is_ok(), "decoded queue accounting underflowed");
    }
}

/// Cheap, cloneable, lock-free reader of one decoder's queue statistics,
/// independent of the owning [`StreamingDecodeHandle`]'s own lifecycle
/// (Block 35.1 "decoder/source queues" diagnostics).
#[derive(Clone)]
pub struct DecodeStatisticsReader {
    statistics: Arc<SharedStatistics>,
    config: StreamingDecodeConfig,
}

impl DecodeStatisticsReader {
    #[must_use]
    pub fn snapshot(&self) -> DecodeStatistics {
        self.statistics.snapshot(self.config)
    }
}

/// Owner of one cancellable worker and its bounded decoded-PCM receiver.
///
/// Dropping the handle cancels and joins the worker. Normal completion should
/// consume through the `end_of_stream` chunk before calling [`Self::join`].
#[must_use = "dropping the handle immediately cancels the decode worker"]
pub struct StreamingDecodeHandle {
    receiver: Receiver<DecodedPcmChunk>,
    cancellation: Arc<AtomicBool>,
    statistics: Arc<SharedStatistics>,
    config: StreamingDecodeConfig,
    stream_info: DecodeStreamInfo,
    join: Option<JoinHandle<Result<DecodeSummary, DecodeError>>>,
}

impl StreamingDecodeHandle {
    /// Opens and validates a staged source, then starts one bounded worker.
    ///
    /// The container and decoder are probed before this function returns, so
    /// unsupported formats and invalid leading metadata fail synchronously.
    ///
    /// # Errors
    ///
    /// Returns a typed [`DecodeError`] for I/O, metadata, format, resource, or
    /// worker-creation failures. No worker survives a failed return.
    pub fn open(
        path: impl AsRef<Path>,
        config: StreamingDecodeConfig,
    ) -> Result<Self, DecodeError> {
        let config = config.validate()?;
        let opened = OpenedDecoder::open(path.as_ref(), config)?;
        let stream_info = opened.stream_info.clone();
        let (sender, receiver) = sync_channel(config.queue_chunks);
        let cancellation = Arc::new(AtomicBool::new(false));
        let statistics = Arc::new(SharedStatistics::default());
        statistics.set_state(DecodeWorkerState::Running);
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_statistics = Arc::clone(&statistics);
        let join = thread::Builder::new()
            .name("silent-disco-streaming-decoder".to_owned())
            .spawn(move || {
                run_decoder_worker(
                    opened,
                    &sender,
                    config,
                    &worker_cancellation,
                    &worker_statistics,
                )
            })
            .map_err(|error| {
                DecodeError::new(
                    DecodeErrorKind::Io,
                    format!("could not start the streaming decoder worker: {error}"),
                )
            })?;
        Ok(Self {
            receiver,
            cancellation,
            statistics,
            config,
            stream_info,
            join: Some(join),
        })
    }

    /// Returns immutable format and validated source facts.
    #[must_use]
    pub fn stream_info(&self) -> &DecodeStreamInfo {
        &self.stream_info
    }

    /// Returns a lock-free snapshot of queue pressure and worker progress.
    #[must_use]
    pub fn statistics(&self) -> DecodeStatistics {
        self.statistics.snapshot(self.config)
    }

    /// Returns a cheap, cloneable, lock-free reader of this decoder's
    /// statistics -- obtainable before `self` is consumed or handed off
    /// elsewhere (e.g. wrapped by a packetizer worker), so a caller that
    /// only wants read-only visibility (diagnostics) never needs to
    /// contend for exclusive ownership of the handle itself.
    #[must_use]
    pub fn statistics_reader(&self) -> DecodeStatisticsReader {
        DecodeStatisticsReader {
            statistics: Arc::clone(&self.statistics),
            config: self.config,
        }
    }

    /// Receives the next bounded chunk, blocking until data or disconnect.
    ///
    /// # Errors
    ///
    /// Returns [`RecvError`] after the worker exits and all queued chunks have
    /// been consumed. Call [`Self::join`] to obtain the typed terminal result.
    pub fn recv(&self) -> Result<DecodedPcmChunk, RecvError> {
        let chunk = self.receiver.recv()?;
        self.statistics.decrement_queued();
        Ok(chunk)
    }

    /// Receives the next bounded chunk with a caller-selected timeout.
    ///
    /// # Errors
    ///
    /// Returns [`RecvTimeoutError::Timeout`] when no chunk arrives in time, or
    /// `Disconnected` after worker exit and queue drain.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<DecodedPcmChunk, RecvTimeoutError> {
        let chunk = self.receiver.recv_timeout(timeout)?;
        self.statistics.decrement_queued();
        Ok(chunk)
    }

    /// Attempts to receive one chunk without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`TryRecvError::Empty`] when no chunk is ready, or `Disconnected`
    /// after worker exit and queue drain.
    pub fn try_recv(&self) -> Result<DecodedPcmChunk, TryRecvError> {
        let chunk = self.receiver.try_recv()?;
        self.statistics.decrement_queued();
        Ok(chunk)
    }

    /// Requests cooperative cancellation. Use [`Self::cancel_and_join`] when a
    /// terminal result is required immediately.
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    /// Joins a worker expected to have completed after its final chunk.
    ///
    /// # Errors
    ///
    /// Returns the typed worker failure or [`DecodeErrorKind::WorkerPanicked`].
    /// Calling this before draining a full queue can block indefinitely; use
    /// [`Self::cancel_and_join`] for early termination.
    pub fn join(mut self) -> Result<DecodeSummary, DecodeError> {
        self.join_worker()
    }

    /// Cancels the worker and joins it deterministically.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeErrorKind::Cancelled`] after ordinary cancellation, or a
    /// more specific terminal failure if one occurred first.
    pub fn cancel_and_join(mut self) -> Result<DecodeSummary, DecodeError> {
        self.cancel();
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<DecodeSummary, DecodeError> {
        let Some(join) = self.join.take() else {
            return Err(DecodeError::new(
                DecodeErrorKind::WorkerPanicked,
                "streaming decoder worker was already joined",
            ));
        };
        join.join().map_err(|_| {
            self.statistics.set_state(DecodeWorkerState::Failed);
            DecodeError::new(
                DecodeErrorKind::WorkerPanicked,
                "streaming decoder worker panicked",
            )
        })?
    }
}

impl Drop for StreamingDecodeHandle {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct ActiveWorkerGuard;

impl ActiveWorkerGuard {
    fn enter() -> Self {
        ACTIVE_DECODE_WORKERS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveWorkerGuard {
    fn drop(&mut self) {
        ACTIVE_DECODE_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn run_decoder_worker(
    mut opened: OpenedDecoder,
    sender: &SyncSender<DecodedPcmChunk>,
    config: StreamingDecodeConfig,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
) -> Result<DecodeSummary, DecodeError> {
    let _active_worker = ActiveWorkerGuard::enter();
    let result = decode_stream(&mut opened, sender, config, cancellation, statistics);
    match &result {
        Ok(summary) => statistics.set_state(summary.state),
        Err(error) if error.kind == DecodeErrorKind::Cancelled => {
            statistics.set_state(DecodeWorkerState::Cancelled);
        }
        Err(_) => statistics.set_state(DecodeWorkerState::Failed),
    }
    result
}

fn decode_stream(
    opened: &mut OpenedDecoder,
    sender: &SyncSender<DecodedPcmChunk>,
    config: StreamingDecodeConfig,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
) -> Result<DecodeSummary, DecodeError> {
    let mut source_spec = None;
    let mut resampler = None;
    let mut chunker = Chunker::new(config.chunk_frames, config.queue_chunks);
    let mut samples = Vec::<f32>::new();
    let mut decoded_any = false;

    loop {
        ensure_not_cancelled(cancellation)?;
        let packet = opened
            .format
            .next_packet()
            .map_err(|error| map_symphonia_error(error, opened.metadata_prefixed))?;
        let Some(packet) = packet else {
            break;
        };
        if packet.track_id != opened.track_id {
            continue;
        }
        let decoded = opened
            .decoder
            .decode(&packet)
            .map_err(|error| map_symphonia_error(error, opened.metadata_prefixed))?;
        let spec = (decoded.spec().rate(), decoded.spec().channels().count());
        validate_declared_format(Some(spec.0), Some(spec.1))?;
        if source_spec.is_some_and(|current| current != spec) {
            return Err(DecodeError::new(
                DecodeErrorKind::FormatChanged,
                "decoder source changed sample rate or channel count mid-stream",
            ));
        }
        source_spec.get_or_insert(spec);

        let frame_count = decoded.frames();
        if frame_count > MAX_DECODED_PACKET_FRAMES {
            return Err(DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "one decoded packet exceeded the bounded source-frame limit",
            ));
        }
        let expected_samples = frame_count.checked_mul(spec.1).ok_or_else(|| {
            DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "decoded packet sample count overflowed",
            )
        })?;
        if decoded.samples_interleaved() != expected_samples {
            return Err(DecodeError::new(
                DecodeErrorKind::CorruptInput,
                "decoded packet reported an inconsistent frame and sample count",
            ));
        }
        samples.resize(expected_samples, f32::MID);
        decoded.copy_to_slice_interleaved(&mut samples);
        let stereo = map_to_stereo(&samples, spec.1)?;
        let active_resampler = match &mut resampler {
            Some(active) => active,
            None => resampler.insert(StereoResampler::new(spec.0)?),
        };
        active_resampler.push(&stereo, false, |frame| {
            chunker.push(frame, sender, cancellation, statistics)
        })?;
        decoded_any |= frame_count > 0;
    }

    if !decoded_any {
        return Err(DecodeError::new(
            DecodeErrorKind::EmptySource,
            "decoder reached end-of-stream without PCM frames",
        ));
    }
    if opened.decoder.finalize().verify_ok == Some(false) {
        return Err(DecodeError::new(
            DecodeErrorKind::CorruptInput,
            "decoded source failed its codec integrity verification",
        ));
    }
    if let Some(active_resampler) = &mut resampler {
        active_resampler.push(&[], true, |frame| {
            chunker.push(frame, sender, cancellation, statistics)
        })?;
    }
    chunker.finish(sender, cancellation, statistics)?;
    Ok(DecodeSummary {
        state: DecodeWorkerState::Completed,
        emitted_frames: statistics.emitted_frames.load(Ordering::Acquire),
        backpressure_events: statistics.backpressure_events.load(Ordering::Acquire),
    })
}

struct Chunker {
    chunk_frames: usize,
    queue_capacity_chunks: usize,
    first_sample_index: u64,
    current: Vec<i16>,
    pending_full: Option<Vec<i16>>,
}

impl Chunker {
    fn new(chunk_frames: usize, queue_capacity_chunks: usize) -> Self {
        Self {
            chunk_frames,
            queue_capacity_chunks,
            first_sample_index: 0,
            current: Vec::with_capacity(chunk_frames * AudioFormat::CANONICAL.samples_per_frame()),
            pending_full: None,
        }
    }

    fn push(
        &mut self,
        frame: [f32; 2],
        sender: &SyncSender<DecodedPcmChunk>,
        cancellation: &AtomicBool,
        statistics: &SharedStatistics,
    ) -> Result<(), DecodeError> {
        if self.pending_full.is_some() {
            self.send_pending(false, sender, cancellation, statistics)?;
        }
        self.current.push(float_to_i16(frame[0]));
        self.current.push(float_to_i16(frame[1]));
        let maximum_samples = self
            .chunk_frames
            .checked_mul(AudioFormat::CANONICAL.samples_per_frame())
            .ok_or_else(|| {
                DecodeError::new(
                    DecodeErrorKind::ResourceLimit,
                    "canonical chunk sample count overflowed",
                )
            })?;
        if self.current.len() == maximum_samples {
            self.pending_full = Some(std::mem::take(&mut self.current));
            self.current = Vec::with_capacity(maximum_samples);
        }
        Ok(())
    }

    fn finish(
        &mut self,
        sender: &SyncSender<DecodedPcmChunk>,
        cancellation: &AtomicBool,
        statistics: &SharedStatistics,
    ) -> Result<(), DecodeError> {
        if !self.current.is_empty() {
            if self.pending_full.is_some() {
                self.send_pending(false, sender, cancellation, statistics)?;
            }
            let frames = std::mem::take(&mut self.current);
            self.send_frames(frames, true, sender, cancellation, statistics)?;
        } else if self.pending_full.is_some() {
            self.send_pending(true, sender, cancellation, statistics)?;
        }
        Ok(())
    }

    fn send_pending(
        &mut self,
        end_of_stream: bool,
        sender: &SyncSender<DecodedPcmChunk>,
        cancellation: &AtomicBool,
        statistics: &SharedStatistics,
    ) -> Result<(), DecodeError> {
        let frames = self.pending_full.take().ok_or_else(|| {
            DecodeError::new(
                DecodeErrorKind::WorkerPanicked,
                "decoder chunker lost its pending bounded chunk",
            )
        })?;
        self.send_frames(frames, end_of_stream, sender, cancellation, statistics)
    }

    fn send_frames(
        &mut self,
        frames: Vec<i16>,
        end_of_stream: bool,
        sender: &SyncSender<DecodedPcmChunk>,
        cancellation: &AtomicBool,
        statistics: &SharedStatistics,
    ) -> Result<(), DecodeError> {
        let frame_count = frames.len() / AudioFormat::CANONICAL.samples_per_frame();
        let frame_count_u64 = u64::try_from(frame_count).map_err(|_| {
            DecodeError::new(
                DecodeErrorKind::ResourceLimit,
                "canonical chunk frame count exceeded the supported range",
            )
        })?;
        let next_sample_index = self
            .first_sample_index
            .checked_add(frame_count_u64)
            .ok_or_else(|| {
                DecodeError::new(
                    DecodeErrorKind::ResourceLimit,
                    "canonical sample index overflowed",
                )
            })?;
        let chunk = DecodedPcmChunk {
            format: AudioFormat::CANONICAL,
            first_sample_index: SampleIndex::new(self.first_sample_index),
            frames,
            end_of_stream,
        };
        send_with_backpressure(
            sender,
            chunk,
            cancellation,
            statistics,
            self.queue_capacity_chunks,
        )?;
        self.first_sample_index = next_sample_index;
        statistics
            .emitted_frames
            .fetch_add(frame_count_u64, Ordering::AcqRel);
        Ok(())
    }
}

fn send_with_backpressure(
    sender: &SyncSender<DecodedPcmChunk>,
    mut chunk: DecodedPcmChunk,
    cancellation: &AtomicBool,
    statistics: &SharedStatistics,
    queue_capacity_chunks: usize,
) -> Result<(), DecodeError> {
    let mut backpressure_recorded = false;
    loop {
        ensure_not_cancelled(cancellation)?;
        let reserved =
            statistics
                .queued_chunks
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    (queued < queue_capacity_chunks).then_some(queued + 1)
                });
        if reserved.is_err() {
            record_backpressure(statistics, &mut backpressure_recorded);
            thread::sleep(BACKPRESSURE_POLL_INTERVAL);
            continue;
        }
        match sender.try_send(chunk) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(returned)) => {
                statistics.decrement_queued();
                chunk = returned;
                record_backpressure(statistics, &mut backpressure_recorded);
                thread::sleep(BACKPRESSURE_POLL_INTERVAL);
            }
            Err(TrySendError::Disconnected(_)) => {
                statistics.decrement_queued();
                return Err(DecodeError::new(
                    DecodeErrorKind::Cancelled,
                    "decoded PCM consumer disconnected",
                ));
            }
        }
    }
}

fn record_backpressure(statistics: &SharedStatistics, recorded: &mut bool) {
    if !*recorded {
        statistics
            .backpressure_events
            .fetch_add(1, Ordering::AcqRel);
        *recorded = true;
    }
}

fn map_to_stereo(samples: &[f32], channels: usize) -> Result<Vec<[f32; 2]>, DecodeError> {
    match channels {
        1 => Ok(samples
            .iter()
            .copied()
            .map(|sample| [sample, sample])
            .collect()),
        2 => {
            let mut chunks = samples.chunks_exact(2);
            let stereo = chunks.by_ref().map(|frame| [frame[0], frame[1]]).collect();
            if chunks.remainder().is_empty() {
                Ok(stereo)
            } else {
                Err(DecodeError::new(
                    DecodeErrorKind::CorruptInput,
                    "decoded stereo packet ended with a partial frame",
                ))
            }
        }
        _ => Err(DecodeError::new(
            DecodeErrorKind::UnsupportedFormat,
            "only mono and stereo sources are supported",
        )),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the sample is clamped to the exact i16 range before conversion"
)]
fn float_to_i16(sample: f32) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * f32::from(i16::MAX);
    scaled.round() as i16
}

fn ensure_not_cancelled(cancellation: &AtomicBool) -> Result<(), DecodeError> {
    if cancellation.load(Ordering::Acquire) {
        Err(DecodeError::new(
            DecodeErrorKind::Cancelled,
            "streaming decode was cancelled",
        ))
    } else {
        Ok(())
    }
}

const fn state_to_usize(state: DecodeWorkerState) -> usize {
    match state {
        DecodeWorkerState::Running => 0,
        DecodeWorkerState::Completed => 1,
        DecodeWorkerState::Cancelled => 2,
        DecodeWorkerState::Failed => 3,
    }
}

const fn usize_to_state(value: usize) -> DecodeWorkerState {
    match value {
        1 => DecodeWorkerState::Completed,
        2 => DecodeWorkerState::Cancelled,
        3 => DecodeWorkerState::Failed,
        _ => DecodeWorkerState::Running,
    }
}

#[cfg(test)]
pub(super) fn active_worker_count() -> usize {
    ACTIVE_DECODE_WORKERS.load(Ordering::Acquire)
}
