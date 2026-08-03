use core::fmt;
use std::error::Error;

use super::JitterBufferRejection;
use super::{
    DEFAULT_TARGET_FILL_FRAMES, DebugPcmRecorder, OffsetUpdateOutcome, PlaybackPhase,
    PlaybackScheduler, RENDER_CHANNELS, RenderRingProducer, ScheduledFrame, SchedulerPoll,
};
use crate::protocol::AudioDatagram;

/// Full-scale magnitude of a 16-bit PCM sample, used to normalize into the
/// render ring's float32 representation.
const PCM16_FULL_SCALE: f32 = 32_768.0;

/// Default distance ahead of its presentation deadline at which a frame is
/// handed to the render ring.
pub const DEFAULT_WRITE_LEAD_MS: u64 = 400;
/// Default ceiling on the stream-start silence prefill.
pub const DEFAULT_MAX_PREFILL_MS: u64 = 800;

/// Tuning for one [`PlaybackPump`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackPumpConfig {
    /// Linear output gain applied while converting to the ring's float32
    /// format, in `0.0..=1.0`.
    pub volume: f32,
    /// How far ahead of its presentation deadline each frame is written into
    /// the ring. The ring is FIFO, so writing early does not move when a frame
    /// is heard — it decides how much audio the real-time consumer has in hand
    /// when the writer is briefly descheduled. Writing at the deadline instead
    /// leaves the ring near-empty, and any writer-side jitter then starves the
    /// output for a few milliseconds at a time: a hard cut below the level any
    /// concealment ramp can reach.
    pub write_lead_ms: u64,
    /// Ceiling on the stream-start silence prefill that aligns the first
    /// frame's ring position with its presentation deadline.
    pub max_prefill_ms: u64,
    /// Ring depth, in frames, at which the pump stops writing and lets the
    /// consumer drain. Without it a large startup backlog pins the ring at
    /// full capacity for the whole stream — maximum latency, and every write
    /// blocked on the consumer — because a full ring would be the only thing
    /// pacing the writer.
    pub target_depth_frames: usize,
}

impl Default for PlaybackPumpConfig {
    fn default() -> Self {
        Self {
            volume: 1.0,
            write_lead_ms: DEFAULT_WRITE_LEAD_MS,
            max_prefill_ms: DEFAULT_MAX_PREFILL_MS,
            target_depth_frames: DEFAULT_TARGET_FILL_FRAMES,
        }
    }
}

/// Stable failure taxonomy for pump configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackPumpConfigErrorKind {
    /// `volume` is not a finite value within `0.0..=1.0`.
    InvalidVolume,
    /// The scheduler's channel count does not match the render ring's fixed
    /// interleaved channel count.
    ChannelCountMismatch,
    /// `target_depth_frames` is zero or exceeds the ring's capacity, so the
    /// pump could never reach it and the depth cap would not pace anything.
    InvalidTargetDepth,
}

/// Typed pump configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackPumpConfigError {
    /// Stable semantic failure category.
    pub kind: PlaybackPumpConfigErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for PlaybackPumpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for PlaybackPumpConfigError {}

/// What one [`PlaybackPump::tick`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpTick {
    /// The scheduler is still accumulating its startup presentation buffer.
    Buffering {
        /// Buffered span accumulated so far, in milliseconds.
        buffered_ms: u64,
    },
    /// A frame was converted and queued into the render ring.
    Queued {
        /// Sequence of the frame queued.
        sequence: u64,
        /// Frames actually accepted by the ring this tick.
        frames: usize,
        /// True when the queued frame was synthesized rather than delivered.
        concealed: bool,
    },
    /// Part of an earlier frame that the ring had no room for was accepted
    /// this tick.
    FlushedPending {
        /// Frames accepted this tick.
        frames: usize,
    },
    /// The ring had no room for the frame in hand; the unwritten remainder is
    /// held and retried on the next tick rather than being dropped.
    RingFull {
        /// Frames still waiting to be accepted by the ring.
        pending_frames: usize,
    },
    /// The ring already holds the configured cushion; the pump is letting the
    /// consumer drain rather than writing further ahead.
    AtTargetDepth {
        /// Frames currently queued in the ring.
        queued_frames: usize,
    },
    /// Nothing is due for this tick.
    Waiting,
    /// No real clock-offset estimate has been applied yet, so no presentation
    /// time can be mapped and nothing may play.
    AwaitingSync,
    /// The scheduler paused and was re-armed to re-accumulate its startup
    /// buffer before playback resumes.
    AwaitingRebuffer,
    /// The scheduler has stopped and will produce no further frames.
    Stopped,
}

/// Drives one [`PlaybackScheduler`] into one [`RenderRingProducer`],
/// converting each scheduled frame from interleaved PCM16 into the ring's
/// interleaved float32 format.
///
/// This is the non-real-time half of playback: it runs on an ordinary worker
/// thread and may allocate, while the real-time audio callback only ever
/// reads from the ring through the narrow C ABI. Frames the ring cannot
/// accept immediately are held and retried, never discarded — silently
/// dropping the unwritten remainder of a partial write is audible as a
/// corrupted note rather than a clean gap.
#[derive(Debug)]
pub struct PlaybackPump {
    scheduler: PlaybackScheduler,
    producer: RenderRingProducer,
    config: PlaybackPumpConfig,
    /// Interleaved float32 samples converted but not yet accepted by the ring.
    pending: Vec<f32>,
    /// Cleared once the stream-start alignment prefill has been queued.
    awaiting_prefill: bool,
    /// Silence frames queued to align the first frame with its deadline.
    prefill_frames: usize,
    /// Set once a real clock-offset estimate has been applied.
    sync_locked: bool,
    /// The offset currently mapping host presentation times onto local time.
    offset_ms: f64,
    /// Largest ring depth observed, in frames.
    peak_queued_frames: usize,
    /// Packets discarded because they arrived before sync locked.
    dropped_before_sync: u64,
    /// Optional capture of exactly what was released toward the ring.
    recorder: Option<DebugPcmRecorder>,
    /// First recorder failure, kept so a broken capture is visible rather than
    /// silently producing a truncated file.
    recorder_error: Option<String>,
}

/// Everything needed to tell, after the fact, where audio went missing or
/// wrong — without relying on a description of what it sounded like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackDiagnostics {
    /// What the scheduler is currently doing.
    pub phase: PlaybackPhase,
    /// True once a real clock-offset estimate has been applied.
    pub sync_locked: bool,
    /// Packets accepted into the jitter buffer.
    pub packets_accepted: u64,
    /// Packets emitted in order, including those drained at stop.
    pub packets_emitted: u64,
    /// Sequences abandoned without being played: lost packets covered by
    /// concealment, plus whole gaps skipped as too wide to cover.
    pub sequences_skipped: u64,
    /// Packets rejected as arriving after their slot had already played.
    pub late_rejections: u64,
    /// Packets rejected as duplicates of an already-buffered sequence.
    pub duplicate_rejections: u64,
    /// Packets rejected as too far ahead to reorder. Climbing while the
    /// phase stays `Buffering` is the signature of a listener stranded
    /// behind the live stream.
    pub reorder_window_rejections: u64,
    /// Times the buffer adopted a far-ahead position because the stream had
    /// moved beyond its reorder window (a recovered outage, or a mid-stream
    /// join).
    pub resynchronisations: u64,
    /// Packets discarded because they arrived before a clock offset existed.
    pub dropped_before_sync: u64,
    /// Frames synthesized to cover missing packets.
    pub concealed_packets: u64,
    /// Times the consecutive-concealment bound forced a rebuffer.
    pub hard_resync_signals: u64,
    /// Buffered presentation-time span currently held, in milliseconds.
    pub buffered_span_ms: u64,
    /// Frames currently queued in the render ring.
    pub ring_queued_frames: usize,
    /// Largest ring depth observed, in frames.
    pub ring_peak_queued_frames: usize,
    /// Frames converted but not yet accepted by the ring.
    pub pending_frames: usize,
    /// Silence frames queued at stream start to align the first frame.
    pub prefill_frames: usize,
    /// Real-time reads that had to substitute silence because the ring ran
    /// dry — the measure of whether the cushion is holding.
    pub ring_underruns: u64,
    /// Frames of silence substituted by those reads.
    pub ring_silence_filled_frames: u64,
    /// Producer writes that could not fit every frame requested.
    pub ring_full_events: u64,
}

/// Result of applying a fresh clock-offset estimate to a running pump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncApplyOutcome {
    /// The first real estimate; playback is now free to start.
    Locked,
    /// The estimate moved within tolerance and was applied in place.
    SoftCorrected,
    /// The estimate moved too far to splice; playback re-accumulates its
    /// startup buffer before resuming.
    Rebuffered,
}

impl PlaybackPump {
    /// Creates a pump driving `scheduler` into `producer`.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackPumpConfigErrorKind::InvalidVolume`] when `volume` is
    /// not finite and within `0.0..=1.0`, or
    /// [`PlaybackPumpConfigErrorKind::ChannelCountMismatch`] when the
    /// scheduler's stream is not the ring's fixed interleaved channel count.
    pub fn new(
        scheduler: PlaybackScheduler,
        producer: RenderRingProducer,
        config: PlaybackPumpConfig,
    ) -> Result<Self, PlaybackPumpConfigError> {
        if !config.volume.is_finite() || config.volume < 0.0 || config.volume > 1.0 {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::InvalidVolume,
                message: format!(
                    "volume of {} must be finite and within 0.0..=1.0",
                    config.volume
                ),
            });
        }
        let channels = scheduler.channels();
        if usize::from(channels) != RENDER_CHANNELS {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::ChannelCountMismatch,
                message: format!(
                    "stream has {channels} channels but the render ring is fixed at \
                     {RENDER_CHANNELS}"
                ),
            });
        }
        if config.target_depth_frames == 0
            || config.target_depth_frames > producer.capacity_frames()
        {
            return Err(PlaybackPumpConfigError {
                kind: PlaybackPumpConfigErrorKind::InvalidTargetDepth,
                message: format!(
                    "target depth of {} frames must be nonzero and within the ring's {}-frame \
                     capacity",
                    config.target_depth_frames,
                    producer.capacity_frames()
                ),
            });
        }
        Ok(Self {
            scheduler,
            producer,
            config,
            pending: Vec::new(),
            awaiting_prefill: true,
            prefill_frames: 0,
            sync_locked: false,
            offset_ms: 0.0,
            peak_queued_frames: 0,
            dropped_before_sync: 0,
            recorder: None,
            recorder_error: None,
        })
    }

    /// Applies a clock-offset estimate produced from a genuinely accepted sync
    /// sample, unlocking playback on the first one.
    ///
    /// The first estimate is adopted outright rather than treated as a
    /// correction: host and listener clocks have unrelated epochs, so a real
    /// first offset is enormous next to the placeholder it replaces and any
    /// threshold comparison against that placeholder is meaningless. Later
    /// estimates are corrections, and one too large to splice re-accumulates
    /// the startup buffer instead of jumping the timeline mid-stream.
    pub fn apply_sync_offset(&mut self, offset_ms: f64) -> SyncApplyOutcome {
        self.offset_ms = offset_ms;
        if !self.sync_locked {
            self.sync_locked = true;
            self.scheduler.rebuffer(offset_ms);
            self.awaiting_prefill = true;
            return SyncApplyOutcome::Locked;
        }
        match self.scheduler.apply_offset_update(offset_ms) {
            OffsetUpdateOutcome::SoftCorrected => SyncApplyOutcome::SoftCorrected,
            OffsetUpdateOutcome::HardResyncRequired => {
                self.scheduler.rebuffer(offset_ms);
                // Playback restarts from an empty ring, so its position must
                // be re-aligned to the timeline rather than inheriting
                // whatever depth happened to remain.
                self.awaiting_prefill = true;
                SyncApplyOutcome::Rebuffered
            }
        }
    }

    /// True once a real clock-offset estimate has been applied.
    #[must_use]
    pub const fn is_sync_locked(&self) -> bool {
        self.sync_locked
    }

    /// Silence frames queued at stream start to align the first frame with its
    /// presentation deadline.
    #[must_use]
    pub const fn prefill_frames(&self) -> usize {
        self.prefill_frames
    }

    /// The scheduler this pump drives, for submitting packets and applying
    /// clock-offset updates.
    pub const fn scheduler_mut(&mut self) -> &mut PlaybackScheduler {
        &mut self.scheduler
    }

    /// Submits one arriving packet, or drops it if playback cannot yet use it.
    ///
    /// Before a clock offset has been accepted there is no timeline to place
    /// a packet on. Buffering them anyway is worse than dropping them: they
    /// accumulate against sequence zero, overflow the jitter buffer's reorder
    /// window within about a second, and are then rejected for the rest of
    /// the acquisition — losing far more audio than the wait itself, and
    /// leaving the buffer holding content that is stale by the time it could
    /// be played. Dropping them lets the buffer adopt the live position once
    /// sync lands.
    ///
    /// # Errors
    ///
    /// Returns a [`JitterBufferRejection`] under the same conditions as
    /// [`JitterBuffer::accept`](super::JitterBuffer::accept). A packet dropped
    /// for arriving before sync is not an error.
    pub fn submit_packet(&mut self, datagram: AudioDatagram) -> Result<(), JitterBufferRejection> {
        if !self.sync_locked {
            self.dropped_before_sync = self.dropped_before_sync.saturating_add(1);
            return Ok(());
        }
        self.scheduler.submit_packet(datagram)
    }

    /// Packets discarded because they arrived before playback had a timeline.
    #[must_use]
    pub const fn dropped_before_sync(&self) -> u64 {
        self.dropped_before_sync
    }

    /// Frames converted but not yet accepted by the ring.
    #[must_use]
    pub fn pending_frames(&self) -> usize {
        self.pending.len() / RENDER_CHANNELS
    }

    /// Frames currently queued in the ring, written but not yet consumed.
    #[must_use]
    pub fn queued_frames(&self) -> usize {
        self.producer.capacity_frames() - self.producer.free_frames()
    }

    /// Captures every frame released toward the ring into `recorder`.
    ///
    /// Records real, concealed, and drained frames — exactly what this pump
    /// believed it was playing — but not the stream-start alignment silence,
    /// which is ring positioning rather than stream content.
    pub fn set_recorder(&mut self, recorder: DebugPcmRecorder) {
        self.recorder = Some(recorder);
    }

    /// Sample rate of the stream this pump serves.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.scheduler.sample_rate()
    }

    /// First failure from the debug recorder, if capture stopped early.
    #[must_use]
    pub fn recorder_error(&self) -> Option<&str> {
        self.recorder_error.as_deref()
    }

    /// Everything needed to tell where audio went missing or wrong.
    #[must_use]
    pub fn diagnostics(&self) -> PlaybackDiagnostics {
        let jitter = self.scheduler.jitter_statistics();
        let concealment = self.scheduler.concealment_statistics();
        let ring = self.producer.telemetry();
        PlaybackDiagnostics {
            phase: self.scheduler.phase(),
            sync_locked: self.sync_locked,
            packets_accepted: jitter.accepted,
            packets_emitted: jitter.emitted,
            sequences_skipped: jitter.skipped,
            late_rejections: jitter.late_rejections,
            duplicate_rejections: jitter.duplicate_rejections,
            reorder_window_rejections: jitter.reorder_window_rejections,
            resynchronisations: jitter.resynchronisations,
            dropped_before_sync: self.dropped_before_sync,
            concealed_packets: concealment.total_concealed_packets,
            hard_resync_signals: concealment.hard_resync_signals,
            buffered_span_ms: self.scheduler.buffered_span_ms(),
            ring_queued_frames: self.queued_frames(),
            ring_peak_queued_frames: self.peak_queued_frames,
            pending_frames: self.pending_frames(),
            prefill_frames: self.prefill_frames,
            ring_underruns: ring.underrun_callbacks,
            ring_silence_filled_frames: ring.silence_filled_frames,
            ring_full_events: ring.ring_full_events,
        }
    }

    /// Advances playback by one tick at the given local monotonic time.
    pub fn tick(&mut self, local_now_ms: u64) -> PumpTick {
        // Nothing may play against a placeholder offset. A stream started on
        // one would map every presentation time through a meaningless
        // timeline and either dump the whole stream at once or drop all of it
        // as late — with no signal that anything is wrong.
        if !self.sync_locked {
            return PumpTick::AwaitingSync;
        }
        if !self.pending.is_empty() {
            // Never poll for new audio while an earlier frame is still
            // partly unwritten: that would reorder the stream.
            let flushed = self.flush_pending();
            return if self.pending.is_empty() {
                PumpTick::FlushedPending { frames: flushed }
            } else {
                PumpTick::RingFull {
                    pending_frames: self.pending_frames(),
                }
            };
        }

        // Hold the ring at its intended cushion. Beyond this the pump simply
        // waits: without the cap, a large startup backlog would run the ring
        // to capacity and stay there for the rest of the stream.
        let queued_frames = self.queued_frames();
        self.peak_queued_frames = self.peak_queued_frames.max(queued_frames);
        if queued_frames >= self.config.target_depth_frames {
            return PumpTick::AtTargetDepth { queued_frames };
        }

        // Release frames early by the configured lead. The ring's FIFO
        // position, not the moment of writing, decides when a frame is heard.
        let poll_time_ms = local_now_ms.saturating_add(self.config.write_lead_ms);
        match self.scheduler.poll(poll_time_ms) {
            SchedulerPoll::Buffering { buffered_ms } => PumpTick::Buffering { buffered_ms },
            SchedulerPoll::Waiting { .. } => PumpTick::Waiting,
            SchedulerPoll::AwaitingRebuffer => {
                // Re-arm immediately: the pause exists to force a fresh
                // startup buffer, not to end playback. Without this the
                // stream would stay silent forever after one long outage.
                self.scheduler.rebuffer(self.offset_ms);
                self.awaiting_prefill = true;
                PumpTick::AwaitingRebuffer
            }
            SchedulerPoll::Stopped => PumpTick::Stopped,
            SchedulerPoll::Frame { frame, .. } => {
                let sequence = frame.sequence;
                let concealed = frame.concealed;
                self.queue_alignment_prefill(&frame, local_now_ms);
                let frames = self.enqueue_frame(&frame);
                if self.pending.is_empty() {
                    PumpTick::Queued {
                        sequence,
                        frames,
                        concealed,
                    }
                } else {
                    PumpTick::RingFull {
                        pending_frames: self.pending_frames(),
                    }
                }
            }
        }
    }

    /// Drains the scheduler's buffered tail into the ring and stops it.
    ///
    /// Returns the number of frames accepted by the ring. The ring may not
    /// have room for all of it at once; whatever does not fit is held pending
    /// and can be flushed with further [`Self::tick`] calls before the output
    /// is torn down. The held amount is bounded by the drained tail itself,
    /// which the jitter buffer already bounds.
    pub fn finish(&mut self) -> usize {
        let drained = self.scheduler.drain_remaining();
        let mut written = 0;
        for frame in &drained {
            written += self.enqueue_frame(frame);
        }
        self.scheduler.stop();
        self.finish_recording();
        written
    }

    /// Queues silence ahead of the stream's first frame so that frame is heard
    /// at its presentation deadline rather than as soon as it is written.
    ///
    /// The real-time consumer starts draining the moment the output opens, so
    /// a frame is heard once everything already queued ahead of it has played:
    /// its play time is the write time plus the current ring depth. Writing
    /// the first frame into an empty ring would therefore play it immediately,
    /// discarding the lead the write-ahead is meant to establish. Queuing
    /// exactly the remaining time until its deadline both places it correctly
    /// and seeds the ring's steady-state cushion.
    fn queue_alignment_prefill(&mut self, frame: &ScheduledFrame, local_now_ms: u64) {
        if !self.awaiting_prefill {
            return;
        }
        self.awaiting_prefill = false;
        let deadline_ms = self
            .scheduler
            .local_time_for_host_ms(frame.host_presentation_time_ms);
        // Already due or late: play it now rather than delaying it further.
        let lead_ms = deadline_ms
            .saturating_sub(local_now_ms)
            .min(self.config.max_prefill_ms);
        if lead_ms == 0 {
            return;
        }
        let frames = usize::try_from(lead_ms * u64::from(self.scheduler.sample_rate()) / 1_000)
            .unwrap_or(usize::MAX);
        self.prefill_frames = frames;
        self.pending
            .extend(core::iter::repeat_n(0.0_f32, frames * RENDER_CHANNELS));
    }

    /// Converts one frame and appends it to the pending queue, then pushes as
    /// much of that queue as the ring will take. Appending rather than
    /// replacing keeps playback order intact when an earlier frame was only
    /// partly accepted.
    fn enqueue_frame(&mut self, frame: &ScheduledFrame) -> usize {
        self.record_frame(frame);
        self.pending.reserve(frame.samples.len());
        for &sample in &frame.samples {
            self.pending
                .push(f32::from(sample) / PCM16_FULL_SCALE * self.config.volume);
        }
        self.flush_pending()
    }

    /// Appends one released frame to the debug capture, disabling capture on
    /// the first failure and retaining it for reporting.
    fn record_frame(&mut self, frame: &ScheduledFrame) {
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        if let Err(error) = recorder.append(&frame.samples) {
            self.recorder_error = Some(format!("debug capture failed: {error}"));
            self.recorder = None;
        }
    }

    /// Finalizes the debug capture, if one is active.
    fn finish_recording(&mut self) {
        let sample_rate = self.scheduler.sample_rate();
        if let Some(recorder) = self.recorder.as_mut()
            && let Err(error) = recorder.finish(sample_rate)
        {
            self.recorder_error = Some(format!("debug capture failed to finalize: {error}"));
        }
        self.recorder = None;
    }

    /// Pushes as much of `pending` as the ring accepts, retaining the rest.
    fn flush_pending(&mut self) -> usize {
        if self.pending.is_empty() {
            return 0;
        }
        let written = self.producer.push_frames(&self.pending);
        if written > 0 {
            self.pending.drain(..written * RENDER_CHANNELS);
        }
        self.peak_queued_frames = self.peak_queued_frames.max(self.queued_frames());
        written
    }
}

#[cfg(test)]
mod tests {
    // Exact round trips through the ring's bit-identical float storage, not
    // approximate arithmetic.
    #![allow(clippy::float_cmp)]

    use super::{
        PlaybackPump, PlaybackPumpConfig, PlaybackPumpConfigErrorKind, PumpTick, SyncApplyOutcome,
    };
    use crate::audio::{
        DebugPcmRecorder, PlaybackPhase, PlaybackScheduler, RenderRing, RenderRingConfig,
        RenderRingConsumer, SchedulerConfig,
    };
    use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
    use crate::protocol::{AudioCodec, AudioDatagram};

    const PACKET_DURATION_MS: u32 = 20;
    const SAMPLES_PER_PACKET: u32 = 960;
    const HOST_START_MS: u64 = 1_000;

    fn datagram(sequence: u64, sample_value: i16) -> AudioDatagram {
        AudioDatagram {
            session_id: SessionId::new("session-pump").expect("session id"),
            stream_id: StreamId::new("stream-pump").expect("stream id"),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: SAMPLES_PER_PACKET,
            first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
            host_presentation_time_ms: MonotonicMillis::new(
                HOST_START_MS + sequence * u64::from(PACKET_DURATION_MS),
            ),
            payload: (0..SAMPLES_PER_PACKET * 2)
                .flat_map(|_| sample_value.to_le_bytes())
                .collect(),
        }
    }

    fn pump_with(capacity_frames: usize) -> (PlaybackPump, RenderRingConsumer) {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, consumer) = ring.split();
        // Pacing off: these cases cover conversion and queueing semantics.
        // Write-lead, depth cap, and prefill have their own tests below.
        let mut pump = PlaybackPump::new(scheduler, producer, unpaced_config(capacity_frames))
            .expect("valid pump");
        pump.apply_sync_offset(0.0);
        (pump, consumer)
    }

    /// A pump with production pacing: a one-second ring, a 400ms write lead
    /// and cushion, and an 800ms prefill ceiling.
    fn paced_pump_with(config: PlaybackPumpConfig) -> (PlaybackPump, RenderRingConsumer) {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        })
        .expect("valid ring");
        let (producer, consumer) = ring.split();
        let mut pump = PlaybackPump::new(scheduler, producer, config).expect("valid pump");
        // These cases exercise pacing, not the sync gate; lock it at zero
        // offset so the scheduler's timeline matches the test clock.
        pump.apply_sync_offset(0.0);
        (pump, consumer)
    }

    fn paced_pump() -> (PlaybackPump, RenderRingConsumer) {
        paced_pump_with(PlaybackPumpConfig::default())
    }

    /// A paced pump whose sync gate has NOT been unlocked.
    fn pump_with_unlocked_sync() -> (PlaybackPump, RenderRingConsumer) {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        })
        .expect("valid ring");
        let (producer, consumer) = ring.split();
        let pump =
            PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
        (pump, consumer)
    }

    /// A config with pacing disabled and a depth cap the given ring can reach.
    fn unpaced_config(capacity_frames: usize) -> PlaybackPumpConfig {
        PlaybackPumpConfig {
            volume: 1.0,
            write_lead_ms: 0,
            max_prefill_ms: 0,
            target_depth_frames: capacity_frames,
        }
    }

    #[test]
    fn queues_a_due_frame_into_the_ring_as_normalized_float32() {
        let (mut pump, consumer) = pump_with(4_800);
        pump.scheduler_mut()
            .submit_packet(datagram(0, 16_384))
            .expect("accepted");

        let tick = pump.tick(HOST_START_MS);

        assert!(matches!(
            tick,
            PumpTick::Queued {
                sequence: 0,
                frames: 960,
                concealed: false
            }
        ));
        // A stream's first frame fades in, so sample the steady-state body
        // past the 240-frame (5ms) ramp rather than the very first sample.
        let mut output = vec![0.0_f32; 960 * 2];
        let outcome = consumer.read_frames(&mut output);
        assert_eq!(outcome.frames_supplied, 960);
        assert_eq!(output[0], 0.0);
        // 16384 / 32768 is exactly half scale.
        assert_eq!(output[300 * 2], 0.5);
        assert_eq!(output[300 * 2 + 1], 0.5);
    }

    #[test]
    fn volume_scales_the_queued_samples() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 4_800,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, consumer) = ring.split();
        let mut pump = PlaybackPump::new(
            scheduler,
            producer,
            PlaybackPumpConfig {
                volume: 0.5,
                ..unpaced_config(4_800)
            },
        )
        .expect("valid pump");
        pump.apply_sync_offset(0.0);
        pump.scheduler_mut()
            .submit_packet(datagram(0, 16_384))
            .expect("accepted");

        pump.tick(HOST_START_MS);

        let mut output = vec![0.0_f32; 960 * 2];
        let _ = consumer.read_frames(&mut output);
        assert_eq!(output[300 * 2], 0.25);
    }

    #[test]
    fn a_frame_the_ring_cannot_hold_is_retried_rather_than_partly_discarded() {
        // The smallest permitted ring holds exactly five 960-frame packets.
        let (mut pump, consumer) = pump_with(4_800);
        for sequence in 0..7 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }
        for slot in 0..5 {
            let tick = pump.tick(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS));
            assert!(
                matches!(tick, PumpTick::Queued { .. }),
                "slot {slot}: {tick:?}"
            );
        }

        // Free less than one packet's worth, so the next frame can only be
        // written in part.
        let mut small = vec![0.0_f32; 300 * 2];
        let _ = consumer.read_frames(&mut small);
        let blocked = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));

        assert!(
            matches!(
                blocked,
                PumpTick::RingFull {
                    pending_frames: 660
                }
            ),
            "expected a held remainder, got {blocked:?}"
        );
        assert_eq!(pump.pending_frames(), 660);

        // Draining the ring lets the held remainder through intact: a partial
        // write must never cost audio.
        let mut all = vec![0.0_f32; 4_800 * 2];
        let _ = consumer.read_frames(&mut all);
        let flushed = pump.tick(HOST_START_MS + 5 * u64::from(PACKET_DURATION_MS));

        assert!(matches!(flushed, PumpTick::FlushedPending { frames: 660 }));
        assert_eq!(pump.pending_frames(), 0);
    }

    #[test]
    fn reports_buffering_waiting_and_stopped_states() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 400;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();
        let mut pump =
            PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("pump");
        pump.apply_sync_offset(0.0);

        assert!(matches!(
            pump.tick(HOST_START_MS),
            PumpTick::Buffering { buffered_ms: 0 }
        ));

        for sequence in 0..30 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 1_000))
                .expect("accepted");
        }
        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
        // Polled before the next slot is due.
        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Waiting));

        pump.finish();
        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Stopped));
    }

    #[test]
    fn finish_queues_the_buffered_tail_and_stops_the_scheduler() {
        let (mut pump, consumer) = pump_with(48_000);
        for sequence in 0..5 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }
        let _first = pump.tick(HOST_START_MS);

        let drained_frames = pump.finish();

        // Four packets were still buffered and must not be discarded.
        assert_eq!(drained_frames, 4 * 960);
        let mut output = vec![0.0_f32; 5 * 960 * 2];
        let outcome = consumer.read_frames(&mut output);
        assert_eq!(outcome.frames_supplied, 5 * 960);
        // The stream ends at zero rather than cutting mid-waveform.
        assert_eq!(output[5 * 960 * 2 - 1], 0.0);
        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Stopped));
    }

    #[test]
    fn the_startup_prefill_places_the_first_frame_at_its_presentation_deadline() {
        let (mut pump, consumer) = paced_pump();
        for sequence in 0..40 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }

        // Sequence 0 is due at HOST_START_MS; tick a full lead ahead of that.
        let tick = pump.tick(HOST_START_MS - 400);

        assert!(matches!(tick, PumpTick::Queued { sequence: 0, .. }));
        // 400ms of silence at 48kHz precedes it, so it is heard exactly when
        // the ring has drained that much: at its deadline, not immediately.
        assert_eq!(pump.prefill_frames(), 19_200);
        let mut output = vec![0.0_f32; 19_200 * 2];
        let outcome = consumer.read_frames(&mut output);
        assert_eq!(outcome.frames_supplied, 19_200);
        assert!(output.iter().all(|&sample| sample == 0.0));
    }

    #[test]
    fn a_first_frame_that_is_already_due_is_not_delayed_by_a_prefill() {
        let (mut pump, _consumer) = paced_pump();
        for sequence in 0..40 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }

        // Well past sequence 0's deadline. The scheduler drops the elapsed
        // head and starts on a frame that is genuinely due, so there is
        // nothing to align and no prefill is queued.
        let tick = pump.tick(HOST_START_MS + 5_000);

        assert!(
            matches!(tick, PumpTick::Queued { .. }),
            "expected a queued frame, got {tick:?}"
        );
        assert_eq!(pump.prefill_frames(), 0);
    }

    #[test]
    fn the_prefill_is_clamped_so_a_distant_first_deadline_cannot_flood_the_ring() {
        // A lead wider than the ceiling is the only way the clamp can bind:
        // ordinarily a frame is released at most `write_lead_ms` early, so the
        // prefill never exceeds the lead.
        let (mut pump, _consumer) = paced_pump_with(PlaybackPumpConfig {
            write_lead_ms: 2_000,
            max_prefill_ms: 800,
            ..PlaybackPumpConfig::default()
        });
        for sequence in 0..40 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }

        // With a lead longer than the prefill ceiling, a first frame a full
        // second out would otherwise queue a second of silence.
        let tick = pump.tick(0);

        assert!(matches!(tick, PumpTick::Queued { .. }));
        // 800ms at 48kHz, the configured ceiling, not the full 1000ms gap.
        assert_eq!(pump.prefill_frames(), 38_400);
    }

    #[test]
    fn the_write_lead_releases_frames_before_their_deadline() {
        // Prefill off, so the lead is observable on its own: with it on, the
        // alignment silence immediately establishes the cushion and the depth
        // cap takes over (see the prefill tests above).
        let (mut pump, _consumer) = paced_pump_with(PlaybackPumpConfig {
            max_prefill_ms: 0,
            ..PlaybackPumpConfig::default()
        });
        for sequence in 0..40 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }

        // Sequence 0 is due at HOST_START_MS. One millisecond earlier than a
        // full lead, it is not released yet.
        assert!(matches!(pump.tick(HOST_START_MS - 401), PumpTick::Waiting));
        // Exactly one lead ahead of the deadline, it is.
        assert!(matches!(
            pump.tick(HOST_START_MS - 400),
            PumpTick::Queued { sequence: 0, .. }
        ));
        // The next frame follows the same rule against its own deadline.
        assert!(matches!(
            pump.tick(HOST_START_MS - 380),
            PumpTick::Queued { sequence: 1, .. }
        ));
    }

    #[test]
    fn the_depth_cap_stops_a_startup_backlog_from_pinning_the_ring_full() {
        let (mut pump, consumer) = paced_pump();
        // A second of audio arrives at once, as it does when a send-ahead
        // host floods a listener that has just locked sync.
        for sequence in 0..50 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }

        // Drive far past every deadline: without the cap this would run the
        // ring to capacity and hold it there.
        let mut ticks = 0;
        loop {
            if let PumpTick::AtTargetDepth { queued_frames } = pump.tick(HOST_START_MS + 10_000) {
                assert!(queued_frames >= 19_200);
                break;
            }
            ticks += 1;
            assert!(ticks < 200, "the pump never reached its target depth");
        }

        // The cushion is the configured depth, not the ring's 48000-frame
        // capacity.
        assert!(pump.queued_frames() < 48_000);
        assert!(pump.queued_frames() >= 19_200);

        // Once the consumer drains below the cushion, writing resumes.
        let mut output = vec![0.0_f32; 5_000 * 2];
        let _ = consumer.read_frames(&mut output);
        assert!(matches!(
            pump.tick(HOST_START_MS + 10_000),
            PumpTick::Queued { .. }
        ));
    }

    #[test]
    fn rejects_a_target_depth_the_ring_could_never_reach() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 4_800,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();

        let error = PlaybackPump::new(
            scheduler,
            producer,
            PlaybackPumpConfig {
                target_depth_frames: 9_600,
                ..unpaced_config(4_800)
            },
        )
        .expect_err("a depth beyond the ring's capacity must be rejected");
        assert_eq!(error.kind, PlaybackPumpConfigErrorKind::InvalidTargetDepth);
    }

    #[test]
    fn nothing_plays_until_a_real_clock_offset_has_been_applied() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();
        let mut pump =
            PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
        for sequence in 0..30 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }

        // Audio is buffered and its deadlines have passed, but no sync sample
        // has been accepted: playing now would map every presentation time
        // through a placeholder offset.
        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::AwaitingSync));
        assert!(!pump.is_sync_locked());
        assert_eq!(pump.queued_frames(), 0);

        assert_eq!(pump.apply_sync_offset(0.0), SyncApplyOutcome::Locked);
        assert!(pump.is_sync_locked());
        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
    }

    #[test]
    fn the_first_offset_is_adopted_outright_rather_than_treated_as_a_correction() {
        let (mut pump, _consumer) = pump_with_unlocked_sync();

        // Host and listener clocks have unrelated epochs, so a real first
        // offset dwarfs any correction threshold. Adopting it must not be
        // mistaken for a jump that needs a rebuffer.
        assert_eq!(
            pump.apply_sync_offset(-746_105_745.0),
            SyncApplyOutcome::Locked
        );
        assert!(!pump.scheduler_mut().is_awaiting_rebuffer());
    }

    #[test]
    fn later_offsets_correct_softly_or_force_a_rebuffer_when_they_jump() {
        let (mut pump, _consumer) = pump_with_unlocked_sync();
        pump.apply_sync_offset(1_000.0);

        assert_eq!(
            pump.apply_sync_offset(1_010.0),
            SyncApplyOutcome::SoftCorrected
        );
        // Beyond the hard-resync threshold: re-accumulate rather than splice.
        assert_eq!(
            pump.apply_sync_offset(1_500.0),
            SyncApplyOutcome::Rebuffered
        );
    }

    #[test]
    fn a_paused_scheduler_is_re_armed_so_playback_recovers_after_an_outage() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        scheduler_config.max_consecutive_concealed_packets = 2;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();
        let mut pump =
            PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
        pump.apply_sync_offset(0.0);
        pump.scheduler_mut()
            .submit_packet(datagram(0, 16_384))
            .expect("accepted");

        assert!(matches!(pump.tick(HOST_START_MS), PumpTick::Queued { .. }));
        // Nothing more arrives; the concealment bound is reached.
        assert!(matches!(
            pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS)),
            PumpTick::Queued {
                concealed: true,
                ..
            }
        ));
        let paused = pump.tick(HOST_START_MS + 2 * u64::from(PACKET_DURATION_MS));
        assert!(matches!(paused, PumpTick::AwaitingRebuffer));

        // The pause forces a fresh startup buffer, but must not end playback:
        // when audio resumes, so does the pump.
        for sequence in 10..40 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }
        let resumed = pump.tick(HOST_START_MS + 20 * u64::from(PACKET_DURATION_MS));
        assert!(
            matches!(
                resumed,
                PumpTick::Queued { .. } | PumpTick::Buffering { .. }
            ),
            "playback must recover, got {resumed:?}"
        );
    }

    #[test]
    fn diagnostics_account_for_delivered_concealed_and_skipped_audio() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        scheduler_config.concealment_skip_threshold_packets = 3;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        })
        .expect("valid ring");
        let (producer, consumer) = ring.split();
        let mut pump =
            PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");
        pump.apply_sync_offset(0.0);

        // Sequence 0 arrives, 1 is lost (concealed), 2 arrives, then a wide
        // gap to 20 is skipped outright.
        for sequence in [0, 2, 20, 21] {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }
        // A duplicate and a stale arrival, both ordinary traffic to account for.
        assert!(pump.scheduler_mut().submit_packet(datagram(20, 1)).is_err());
        // Stop at the last real packet: ticking past it would start the
        // outage bridge, whose concealments are a separate behaviour.
        for slot in 0..=21 {
            pump.tick(HOST_START_MS + slot * u64::from(PACKET_DURATION_MS));
        }
        assert!(pump.scheduler_mut().submit_packet(datagram(1, 1)).is_err());

        let diagnostics = pump.diagnostics();

        assert_eq!(diagnostics.packets_accepted, 4);
        assert_eq!(diagnostics.duplicate_rejections, 1);
        assert_eq!(diagnostics.late_rejections, 1);
        // One concealed slot (sequence 1) and one skipped gap (3..=19).
        assert_eq!(diagnostics.concealed_packets, 1);
        assert!(
            diagnostics.sequences_skipped >= 17,
            "expected the wide gap to be accounted for, got {}",
            diagnostics.sequences_skipped
        );
        assert!(diagnostics.sync_locked);
        assert_eq!(diagnostics.phase, PlaybackPhase::Playing);

        // Ring-side counters come from the ring's own telemetry.
        assert!(diagnostics.ring_queued_frames > 0);
        assert_eq!(
            diagnostics.ring_peak_queued_frames,
            diagnostics.ring_queued_frames
        );
        let mut output = vec![0.0_f32; 60_000 * 2];
        let _ = consumer.read_frames(&mut output);
        let after_underrun = pump.diagnostics();
        assert!(after_underrun.ring_underruns > 0);
        assert!(after_underrun.ring_silence_filled_frames > 0);
    }

    #[test]
    fn diagnostics_report_the_phase_through_a_streams_life() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 400;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();
        let mut pump =
            PlaybackPump::new(scheduler, producer, unpaced_config(48_000)).expect("valid pump");

        assert!(!pump.diagnostics().sync_locked);
        pump.apply_sync_offset(0.0);
        assert_eq!(pump.diagnostics().phase, PlaybackPhase::Buffering);

        for sequence in 0..30 {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }
        pump.tick(HOST_START_MS);
        assert_eq!(pump.diagnostics().phase, PlaybackPhase::Playing);
        assert!(pump.diagnostics().buffered_span_ms > 0);

        pump.finish();
        assert_eq!(pump.diagnostics().phase, PlaybackPhase::Stopped);
    }

    #[test]
    fn the_debug_capture_records_exactly_what_was_released_toward_the_ring() {
        let directory =
            std::env::temp_dir().join(format!("silent-disco-pump-capture-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join("capture.wav");

        let (mut pump, _consumer) = pump_with(48_000);
        pump.set_recorder(DebugPcmRecorder::create(&path, 48_000, 2).expect("recorder created"));
        // Sequence 1 is lost, so the capture must contain the concealed frame
        // too: it records what playback believed it was playing.
        for sequence in [0, 2, 3] {
            pump.scheduler_mut()
                .submit_packet(datagram(sequence, 16_384))
                .expect("accepted");
        }
        pump.tick(HOST_START_MS);
        pump.tick(HOST_START_MS + u64::from(PACKET_DURATION_MS));
        pump.finish();

        assert!(pump.recorder_error().is_none());
        let bytes = std::fs::read(&path).expect("capture readable");
        // Four 960-frame stereo PCM16 frames: 0, concealed 1, then 2 and 3
        // drained at stop.
        let expected_data_bytes = 4 * 960 * 2 * 2;
        assert_eq!(bytes.len(), 44 + expected_data_bytes);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[36..40], b"data");
        // The header is patched with the real length when the stream ends.
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            u32::try_from(expected_data_bytes).expect("fits"),
        );
        // Sample rate and channel count round-trip.
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            48_000
        );
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_pump_without_a_recorder_configured_captures_nothing() {
        let (mut pump, _consumer) = pump_with(48_000);
        pump.scheduler_mut()
            .submit_packet(datagram(0, 16_384))
            .expect("accepted");

        pump.tick(HOST_START_MS);
        pump.finish();

        assert!(pump.recorder_error().is_none());
    }

    #[test]
    fn steady_state_ring_depth_converges_to_the_configured_cushion() {
        let (mut pump, consumer) = paced_pump();

        // Simulate a real stream: the host keeps delivering, the pump ticks
        // every 10ms, and the consumer drains at the sample rate.
        let mut clock_ms = HOST_START_MS - 400;
        let mut next_sequence = 0_u64;
        let mut output = vec![0.0_f32; 480 * 2];
        let mut depths = Vec::new();

        for step in 0..400 {
            // Packets arrive a little ahead of their deadlines, as a
            // send-ahead host delivers them.
            while next_sequence * u64::from(PACKET_DURATION_MS) + HOST_START_MS < clock_ms + 600 {
                let _ = pump
                    .scheduler_mut()
                    .submit_packet(datagram(next_sequence, 16_384));
                next_sequence += 1;
            }
            pump.tick(clock_ms);
            // Every 10ms the output consumes 480 frames at 48kHz.
            let _ = consumer.read_frames(&mut output);
            clock_ms += 10;
            if step > 200 {
                depths.push(pump.queued_frames());
            }
        }

        // Once running, the ring holds roughly the configured 400ms cushion:
        // deep enough to absorb writer jitter, and far short of the ring's
        // 48000-frame capacity.
        let minimum = depths.iter().copied().min().expect("samples");
        let maximum = depths.iter().copied().max().expect("samples");
        assert!(
            minimum > 12_000,
            "cushion collapsed toward empty: minimum depth {minimum}"
        );
        assert!(
            maximum <= 20_160,
            "cushion grew past the cap: maximum depth {maximum}"
        );

        // With the cushion holding, the consumer never had to invent silence.
        let diagnostics = pump.diagnostics();
        assert_eq!(
            diagnostics.ring_underruns, 0,
            "a held cushion must prevent underruns"
        );
        assert_eq!(diagnostics.ring_silence_filled_frames, 0);
    }

    #[test]
    fn packets_arriving_before_sync_locks_are_dropped_rather_than_stranding_the_buffer() {
        let (mut pump, _consumer) = pump_with_unlocked_sync();

        // A whole second of audio arrives while the clock is still unlocked.
        // Buffering it would pile it against sequence zero until the reorder
        // window overflows, losing far more than the wait itself costs.
        for sequence in 0..50 {
            pump.submit_packet(datagram(sequence, 16_384))
                .expect("a pre-sync packet is dropped, not an error");
        }
        assert_eq!(pump.dropped_before_sync(), 50);
        assert_eq!(pump.diagnostics().packets_accepted, 0);
        assert_eq!(pump.diagnostics().reorder_window_rejections, 0);

        // Once the clock locks, the buffer adopts the live position: the
        // first arrivals corroborate the jump, then audio flows again.
        pump.apply_sync_offset(0.0);
        for sequence in 200..215 {
            let _ = pump.submit_packet(datagram(sequence, 16_384));
        }
        let diagnostics = pump.diagnostics();
        assert!(
            diagnostics.packets_accepted > 0,
            "the live stream was never picked up after sync locked"
        );
        assert_eq!(diagnostics.resynchronisations, 1);
    }

    #[test]
    fn rejects_an_invalid_volume() {
        let (pump, _consumer) = pump_with(4_800);
        drop(pump);

        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            2,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 4_800,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();

        let error = PlaybackPump::new(
            scheduler,
            producer,
            PlaybackPumpConfig {
                volume: 1.5,
                ..unpaced_config(4_800)
            },
        )
        .expect_err("an out-of-range volume must be rejected");
        assert_eq!(error.kind, PlaybackPumpConfigErrorKind::InvalidVolume);
    }

    #[test]
    fn rejects_a_stream_whose_channel_count_the_ring_cannot_render() {
        let mut scheduler_config = SchedulerConfig::new(
            SessionId::new("session-pump").expect("session id"),
            StreamId::new("stream-pump").expect("stream id"),
            PACKET_DURATION_MS,
            HOST_START_MS,
            SAMPLES_PER_PACKET,
            1,
        );
        scheduler_config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("valid scheduler");
        let ring = RenderRing::new(RenderRingConfig {
            capacity_frames: 4_800,
            target_fill_frames: 1,
        })
        .expect("valid ring");
        let (producer, _consumer) = ring.split();

        let error = PlaybackPump::new(scheduler, producer, unpaced_config(4_800))
            .expect_err("a mono stream must be rejected by a stereo ring");
        assert_eq!(
            error.kind,
            PlaybackPumpConfigErrorKind::ChannelCountMismatch
        );
    }
}
