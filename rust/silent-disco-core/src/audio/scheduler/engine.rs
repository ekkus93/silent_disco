//! [`PlaybackScheduler`] itself: construction/validation, the tick/
//! presentation-time/drift-resync engine (`poll`), drain and
//! waveform-continuity handling (`drain_remaining`), clock-offset/lifecycle
//! controls, diagnostics accessors, and the low-level `host_to_local_ms`/
//! `decode_payload_samples` helpers.
//!
//! Kept as one file rather than force-split further: `poll`, `drain_remaining`,
//! and the offset-handling methods share private mutable state
//! (`fade_in_next_real_frame`, `resume_from_silence`, `last_emitted_tail`),
//! and the drift/resync/concealment invariants are only safely auditable
//! together.

use crate::audio::ramp::{apply_blend_in, apply_fade_out_tail, last_frame};
use crate::audio::{
    ConcealmentOutcome, ConcealmentPolicy, ConcealmentStatistics, JitterBuffer, JitterBufferConfig,
    JitterBufferRejection, JitterBufferStatistics, MAX_PACKET_DURATION_MS, MIN_PACKET_DURATION_MS,
};
use crate::protocol::AudioDatagram;

use super::config::SchedulerConfig;
use super::errors::{SchedulerConfigError, SchedulerConfigErrorKind};
use super::types::{
    BufferHealth, OffsetUpdateOutcome, PlaybackPhase, ScheduledFrame, SchedulerPoll, SchedulerState,
};

/// Maps a bounded jitter buffer and a silence-concealment policy onto a
/// host-presentation timeline, producing one ordered, render-ready PCM frame
/// per tick for exactly one stream.
///
/// This does not itself own or write into a real-time render ring; it
/// produces the frames a future render-ring producer worker will pace into
/// one. `poll` performs no allocation beyond one `Vec<i16>` per frame, no
/// I/O, and no blocking, so it is safe to call at the stream's packet-duration
/// cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferTargetProgress {
    Accumulating,
    Satisfied,
}

#[derive(Debug)]
pub struct PlaybackScheduler {
    config: SchedulerConfig,
    jitter_buffer: JitterBuffer,
    concealment: ConcealmentPolicy,
    state: SchedulerState,
    offset_ms: f64,
    ramp_frames: usize,
    /// Set whenever the next real frame would resume from silence or from
    /// concealed content rather than continuing the previous real frame.
    fade_in_next_real_frame: bool,
    /// Per-channel values the last emitted frame ended on, real or concealed,
    /// so a frame spliced after it continues the waveform instead of stepping.
    last_emitted_tail: Vec<i16>,
    /// Set when real silence genuinely intervenes before the next frame — a
    /// stream's first frame, or a rebuffer that drains the ring — in which
    /// case there is no waveform to continue and the seam is a fade from zero.
    resume_from_silence: bool,
    /// True once this stream has reached `Playing` at least once, which is
    /// what distinguishes a startup buffer from a mid-stream rebuffer.
    has_played: bool,
    /// True once the current startup/rebuffer span target has been reached.
    ///
    /// Alignment may then discard the entire stale head while waiting for a
    /// packet whose deadline is still reachable. Keeping this separate from
    /// `state` prevents the scheduler from demanding another full buffer
    /// target every time that happens -- a real-time arrival stream would
    /// otherwise repeat "accumulate 400ms, discard 400ms" forever.
    buffer_target_progress: BufferTargetProgress,
}

impl PlaybackScheduler {
    /// Creates a scheduler for one stream with an initial host/local
    /// clock-offset estimate.
    ///
    /// # Errors
    ///
    /// Returns a [`SchedulerConfigError`] when the configured sample geometry
    /// implies an unsupported packet duration, the sample rate or
    /// `samples_per_packet` is zero, the water marks are not strictly ordered, the hard-resync
    /// threshold is not positive, or the jitter buffer/concealment bounds
    /// are individually invalid.
    pub fn new(
        config: SchedulerConfig,
        initial_offset_ms: f64,
    ) -> Result<Self, SchedulerConfigError> {
        validate_scheduler_config(&config)?;

        let jitter_buffer = JitterBuffer::new(JitterBufferConfig {
            session_id: config.session_id.clone(),
            stream_id: config.stream_id.clone(),
            max_reorder_window: config.max_reorder_window,
            max_buffered_duration_ms: config.max_buffered_duration_ms,
        })
        .map_err(|error| SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidJitterBufferBounds,
            message: error.message,
        })?;
        // The ramp is expressed in milliseconds but applied in frames.
        // Use the exact sample rate rather than dividing by a rounded packet
        // duration, or the shaping window drifts with non-integer packet time.
        let ramp_frames = u32::try_from(
            u64::from(config.sample_rate) * u64::from(config.concealment_ramp_ms) / 1_000,
        )
        .unwrap_or(u32::MAX)
        .max(1);
        // A ramp as long as the packet leaves no steady-state body between the
        // two edges: every synthesized frame would be pure ramp, so concealed
        // content would never reach its intended amplitude and a run's final
        // fade would consume the whole frame. This is reachable by shortening
        // the packet duration without shortening the ramp, so it is rejected
        // rather than clamped -- a silently halved ramp is exactly the kind of
        // change that only shows up as a device-run regression.
        if ramp_frames >= config.samples_per_packet {
            return Err(SchedulerConfigError {
                kind: SchedulerConfigErrorKind::InvalidConcealmentRamp,
                message: format!(
                    "concealment ramp of {}ms is {ramp_frames} frames, which must be shorter \
                     than the {}-frame packet it shapes",
                    config.concealment_ramp_ms, config.samples_per_packet
                ),
            });
        }
        let concealment =
            ConcealmentPolicy::new(config.max_consecutive_concealed_packets, ramp_frames).map_err(
                |error| SchedulerConfigError {
                    kind: SchedulerConfigErrorKind::InvalidConcealmentBound,
                    message: error.message,
                },
            )?;

        Ok(Self {
            config,
            jitter_buffer,
            concealment,
            state: SchedulerState::Buffering,
            offset_ms: initial_offset_ms,
            ramp_frames: usize::try_from(ramp_frames).unwrap_or(usize::MAX),
            // A stream's own first frame starts from silence and routinely
            // begins mid-waveform (playback opens at whatever sample the
            // presentation timeline lands on), so it is faded in too.
            fade_in_next_real_frame: true,
            last_emitted_tail: Vec::new(),
            resume_from_silence: true,
            has_played: false,
            buffer_target_progress: BufferTargetProgress::Accumulating,
        })
    }

    /// Validates and inserts one arriving packet into the internal jitter
    /// buffer.
    ///
    /// # Errors
    ///
    /// Returns a [`JitterBufferRejection`] under the same conditions as
    /// [`JitterBuffer::accept`].
    pub fn submit_packet(&mut self, datagram: AudioDatagram) -> Result<(), JitterBufferRejection> {
        self.jitter_buffer.accept(datagram)
    }

    fn buffer_health(&self) -> BufferHealth {
        let span = self.jitter_buffer.buffered_span_ms();
        if span < self.config.low_water_ms {
            BufferHealth::Low
        } else if span > self.config.high_water_ms {
            BufferHealth::High
        } else {
            BufferHealth::Normal
        }
    }

    fn expected_host_presentation_time_ms(&self, sequence: u64) -> u64 {
        let sample_index = u128::from(sequence) * u128::from(self.config.samples_per_packet);
        let elapsed_ms = sample_index * 1_000 / u128::from(self.config.sample_rate);
        let elapsed_ms = u64::try_from(elapsed_ms).unwrap_or(u64::MAX);
        self.config.host_start_time_ms.saturating_add(elapsed_ms)
    }

    fn expected_first_sample_index(&self, sequence: u64) -> u64 {
        sequence.saturating_mul(u64::from(self.config.samples_per_packet))
    }

    /// Advances the scheduler by one tick at the given local monotonic time,
    /// returning the frame ready for this tick's presentation slot, if any.
    ///
    /// This is the no-write-ahead form used by direct scheduler callers and
    /// tests. The render-ring pump uses an internal two-clock form so stale
    /// startup alignment is evaluated against *actual now*, not its future
    /// release horizon.
    ///
    /// # Panics
    ///
    /// Never panics; every arithmetic path is bounded by validated
    /// configuration.
    #[must_use]
    pub fn poll(&mut self, local_now_ms: u64) -> SchedulerPoll {
        self.poll_with_release_horizon(local_now_ms, local_now_ms)
    }

    fn advance_buffering(&mut self, local_now_ms: u64) -> Option<SchedulerPoll> {
        let buffered_ms = self.jitter_buffer.buffered_span_ms();
        // A stream's first start and a mid-stream recovery are not the same
        // situation. Clamp recovery to the startup target so lowering startup
        // latency can never silently make a later recovery longer.
        let target = if self.has_played {
            self.config
                .rebuffer_target_ms
                .min(self.config.startup_buffer_target_ms)
        } else {
            self.config.startup_buffer_target_ms
        };
        if self.buffer_target_progress == BufferTargetProgress::Accumulating && buffered_ms < target
        {
            return Some(SchedulerPoll::Buffering { buffered_ms });
        }
        self.buffer_target_progress = BufferTargetProgress::Satisfied;

        // Start only on a packet whose host-timeline deadline is still reachable.
        // Remember a satisfied target even if alignment discards the whole stale
        // head; otherwise a real-time stream can loop forever accumulating and
        // discarding the same target-sized window.
        self.discard_already_late_head(local_now_ms);
        if target > 0 && self.jitter_buffer.is_empty() {
            return Some(SchedulerPoll::Buffering { buffered_ms: 0 });
        }

        self.state = SchedulerState::Playing;
        self.has_played = true;
        self.buffer_target_progress = BufferTargetProgress::Accumulating;
        None
    }

    /// Advances the scheduler with separate actual-time and release-horizon
    /// clocks.
    ///
    /// `local_now_ms` is the listener's real current monotonic time and is
    /// used only to decide which startup/rebuffer packets are already too
    /// late to play on the authoritative host timeline.
    /// `local_release_horizon_ms` is how far ahead a non-real-time producer
    /// may queue into its FIFO render ring. It controls ordinary due/waiting
    /// decisions but must never make future audio look stale.
    ///
    /// A release horizon before actual now is harmlessly clamped to now.
    ///
    /// # Panics
    ///
    /// Never panics; every arithmetic path is bounded by validated
    /// configuration.
    #[must_use]
    pub(crate) fn poll_with_release_horizon(
        &mut self,
        local_now_ms: u64,
        local_release_horizon_ms: u64,
    ) -> SchedulerPoll {
        let local_release_horizon_ms = local_release_horizon_ms.max(local_now_ms);
        match self.state {
            SchedulerState::Stopped => return SchedulerPoll::Stopped,
            SchedulerState::AwaitingRebuffer => return SchedulerPoll::AwaitingRebuffer,
            SchedulerState::Buffering => {
                if let Some(poll) = self.advance_buffering(local_now_ms) {
                    return poll;
                }
            }
            SchedulerState::Playing => {}
        }

        let mut next_sequence = self.jitter_buffer.next_expected_sequence();
        let mut host_deadline_ms = self.expected_host_presentation_time_ms(next_sequence);
        let mut local_deadline_ms = host_to_local_ms(host_deadline_ms, self.offset_ms);

        if local_release_horizon_ms < local_deadline_ms {
            return SchedulerPoll::Waiting {
                buffer_health: self.buffer_health(),
            };
        }

        // A gap too wide to cover packet by packet is abandoned outright.
        // Concealing it would queue one synthesized frame per missing slot
        // ahead of the audio that actually arrived, so the whole outage would
        // replay as filler and every later frame would trail its deadline by
        // the outage's length. Skipping lets the post-gap audio play at its
        // own correct presentation time instead.
        if self.jitter_buffer.missing_sequence_count()
            > u64::from(self.config.concealment_skip_threshold_packets)
        {
            self.jitter_buffer.skip_to_earliest_buffered();
            self.concealment.reset_consecutive_count();
            // Nothing is emitted for the abandoned span: the post-gap frame is
            // queued directly behind the pre-gap one, so the two play back to
            // back with no silence between them. The seam therefore has to be
            // crossfaded from the outgoing waveform -- fading in from a zero
            // that is never actually rendered is itself a step, and a real
            // device produced exactly one waveform discontinuity per skip.
            self.fade_in_next_real_frame = true;

            next_sequence = self.jitter_buffer.next_expected_sequence();
            host_deadline_ms = self.expected_host_presentation_time_ms(next_sequence);
            local_deadline_ms = host_to_local_ms(host_deadline_ms, self.offset_ms);
            if local_release_horizon_ms < local_deadline_ms {
                return SchedulerPoll::Waiting {
                    buffer_health: self.buffer_health(),
                };
            }
        }

        if let Some(datagram) = self.jitter_buffer.pop_in_order() {
            let mut samples = decode_payload_samples(&datagram.payload);
            // Record the packet as delivered *before* any fade, so a later
            // concealment repeats the real waveform rather than a ramped copy.
            self.concealment
                .record_delivery(&samples, self.config.channels);
            let channels = usize::from(self.config.channels);
            if self.fade_in_next_real_frame {
                let continues_from: &[i16] = if self.resume_from_silence {
                    &[]
                } else {
                    &self.last_emitted_tail
                };
                apply_blend_in(&mut samples, channels, self.ramp_frames, continues_from);
                self.fade_in_next_real_frame = false;
                self.resume_from_silence = false;
            }
            self.remember_emitted_tail(&samples, channels);
            let frame = ScheduledFrame {
                sequence: datagram.sequence.get(),
                first_sample_index: datagram.first_sample_index.get(),
                host_presentation_time_ms: datagram.host_presentation_time_ms.get(),
                samples,
                concealed: false,
            };
            return SchedulerPoll::Frame {
                frame,
                buffer_health: self.buffer_health(),
            };
        }

        let (samples, outcome) = self
            .concealment
            .conceal(self.config.samples_per_packet, self.config.channels);
        self.jitter_buffer.skip_expected_sequence();
        // Concealed content ends mid-decay rather than at zero, so whatever
        // real audio resumes after it is blended in from that value.
        self.fade_in_next_real_frame = true;
        self.remember_emitted_tail(&samples, usize::from(self.config.channels));

        match outcome {
            ConcealmentOutcome::Concealed => {
                let frame = ScheduledFrame {
                    sequence: next_sequence,
                    first_sample_index: self.expected_first_sample_index(next_sequence),
                    host_presentation_time_ms: host_deadline_ms,
                    samples,
                    concealed: true,
                };
                SchedulerPoll::Frame {
                    frame,
                    buffer_health: self.buffer_health(),
                }
            }
            ConcealmentOutcome::HardResyncRequired => {
                self.state = SchedulerState::AwaitingRebuffer;
                // This frame is not emitted, and the rebuffer that follows
                // drains the ring: real silence, so nothing continues from it.
                self.resume_from_silence = true;
                SchedulerPoll::AwaitingRebuffer
            }
        }
    }

    /// Removes and returns every buffered packet as a render-ready frame, in
    /// sequence order and ignoring presentation deadlines.
    ///
    /// Call this when a stream is stopping: everything still buffered already
    /// arrived in time, so it is real tail content (a song's final note, for
    /// example) rather than backlog, and discarding it would truncate the
    /// stream. Because these frames play back to back rather than at their own
    /// deadlines, a sequence hole inside the drained range would splice two
    /// unrelated waveforms directly together — a click with no silence around
    /// it. Every hole edge is faded, as is the final frame's tail, so playback
    /// ends at zero instead of cutting mid-waveform when the output stops.
    pub fn drain_remaining(&mut self) -> Vec<ScheduledFrame> {
        let channels = usize::from(self.config.channels);
        let ramp = self.ramp_frames;
        let mut expected_next = self.jitter_buffer.next_expected_sequence();
        // The first drained frame continues from whatever the last emitted
        // frame ended on: mid-decay concealed content, or silence.
        let mut fade_in_next = self.fade_in_next_real_frame;
        let mut blend_from = if self.resume_from_silence {
            Vec::new()
        } else {
            std::mem::take(&mut self.last_emitted_tail)
        };
        let mut frames: Vec<ScheduledFrame> = Vec::new();

        for datagram in self.jitter_buffer.drain_all() {
            let sequence = datagram.sequence.get();
            if sequence > expected_next {
                if let Some(previous) = frames.last_mut() {
                    apply_fade_out_tail(&mut previous.samples, channels, ramp);
                }
                fade_in_next = true;
                // A faded hole edge really does reach zero.
                blend_from.clear();
            }
            let mut samples = decode_payload_samples(&datagram.payload);
            if fade_in_next {
                apply_blend_in(&mut samples, channels, ramp, &blend_from);
                fade_in_next = false;
                blend_from.clear();
            }
            frames.push(ScheduledFrame {
                sequence,
                first_sample_index: datagram.first_sample_index.get(),
                host_presentation_time_ms: datagram.host_presentation_time_ms.get(),
                samples,
                concealed: false,
            });
            expected_next = sequence + 1;
        }

        if let Some(last) = frames.last_mut() {
            apply_fade_out_tail(&mut last.samples, channels, ramp);
        }
        self.fade_in_next_real_frame = true;
        // The drained tail ends faded to zero, so anything after it starts
        // from genuine silence.
        self.resume_from_silence = true;
        self.last_emitted_tail.clear();
        frames
    }

    /// Records the per-channel values an emitted frame ended on, so the next
    /// frame spliced after it can continue from them rather than stepping.
    fn remember_emitted_tail(&mut self, samples: &[i16], channels: usize) {
        self.last_emitted_tail.clear();
        if let Some(tail) = last_frame(samples, channels) {
            self.last_emitted_tail.extend_from_slice(tail);
        }
    }

    /// Drops buffered or missing head sequences whose presentation deadline
    /// has already passed, so playback begins on a frame that is genuinely
    /// reachable on the authoritative timeline.
    ///
    /// This is what makes two listeners agree. A stream is heard at
    /// `write time + ring depth`, and writing ahead into a FIFO preserves
    /// relative timing exactly — so the entire stream is offset by however
    /// late its *first* frame was when playback began. Starting on a stale
    /// head therefore shifts everything that follows by the listener's own
    /// startup latency, and two devices that lock sync at different moments
    /// stay that far apart for the whole session.
    ///
    /// The cost is the already-elapsed head of the stream, bounded by the
    /// startup/rebuffer window. Being late together is the product; replaying
    /// stale audio is not.
    fn discard_already_late_head(&mut self, local_now_ms: u64) {
        while !self.jitter_buffer.is_empty() {
            let sequence = self.jitter_buffer.next_expected_sequence();
            let deadline_ms = host_to_local_ms(
                self.expected_host_presentation_time_ms(sequence),
                self.offset_ms,
            );
            if deadline_ms >= local_now_ms {
                break;
            }

            if self.jitter_buffer.peek_next_sequence() == Some(sequence) {
                let discarded = self.jitter_buffer.discard_in_order();
                debug_assert!(discarded, "peeked next packet must still be discardable");
            } else {
                // A missing slot that is already in the past cannot be
                // repaired by concealment without shifting all later audio.
                self.jitter_buffer.skip_expected_sequence();
            }
            self.fade_in_next_real_frame = true;
        }
    }

    /// Re-aligns a playing scheduler after the real-time output has run dry.
    ///
    /// Once the render ring underruns, wall-clock time advances while no audio
    /// is rendered. Replaying packets whose deadlines elapsed during that
    /// silence would permanently move this listener behind the host timeline,
    /// so only those now-unreachable head slots are discarded. The next real
    /// frame is treated as a resume from silence and blended in accordingly.
    pub(crate) fn realign_to_now(&mut self, local_now_ms: u64) {
        self.discard_already_late_head(local_now_ms);
        self.concealment.reset_consecutive_count();
        self.fade_in_next_real_frame = true;
        self.resume_from_silence = true;
        self.last_emitted_tail.clear();
    }

    /// Hard-resync threshold configured for this scheduler.
    #[must_use]
    pub(crate) const fn hard_resync_threshold_ms(&self) -> f64 {
        self.config.hard_resync_threshold_ms
    }

    /// Applies an updated host/local clock-offset estimate, deciding between
    /// a soft correction and a hard resync based on the configured
    /// hard-resync threshold.
    #[must_use]
    pub fn apply_offset_update(&mut self, new_offset_ms: f64) -> OffsetUpdateOutcome {
        let delta_ms = (new_offset_ms - self.offset_ms).abs();
        self.offset_ms = new_offset_ms;
        if delta_ms > self.config.hard_resync_threshold_ms {
            self.state = SchedulerState::AwaitingRebuffer;
            OffsetUpdateOutcome::HardResyncRequired
        } else {
            OffsetUpdateOutcome::SoftCorrected
        }
    }

    /// Explicitly resumes a scheduler that is awaiting rebuffer, applying a
    /// fresh clock-offset estimate and re-arming the startup presentation
    /// buffer. Previously buffered packets are preserved.
    pub fn rebuffer(&mut self, new_offset_ms: f64) {
        self.offset_ms = new_offset_ms;
        self.concealment.reset_consecutive_count();
        self.state = SchedulerState::Buffering;
        self.buffer_target_progress = BufferTargetProgress::Accumulating;
        // Playback resumes after a real interruption; fade back in.
        self.fade_in_next_real_frame = true;
        self.resume_from_silence = true;
    }

    /// Re-anchors this scheduler's presentation-time base to
    /// `host_start_time_ms`, matching a host that re-broadcast `StreamStart`
    /// with an updated anchor after a pause (see the desktop host's
    /// pause/resume offset accounting). Without this,
    /// [`Self::expected_host_presentation_time_ms`] would keep comparing
    /// newly arriving, already-shifted wire timestamps against an anchor
    /// still fixed at stream start, misreading every post-resume packet as
    /// far ahead of its expected slot. Takes the absolute new value rather
    /// than a delta so repeated calls (e.g. a duplicate re-broadcast) are
    /// idempotent instead of compounding.
    pub fn set_host_start_time_ms(&mut self, host_start_time_ms: u64) {
        self.config.host_start_time_ms = host_start_time_ms;
    }

    /// Explicitly stops this scheduler. Idempotent; a new stream requires a
    /// new [`PlaybackScheduler`] instance.
    pub fn stop(&mut self) {
        self.state = SchedulerState::Stopped;
    }

    /// True once [`Self::stop`] has been called.
    #[must_use]
    pub const fn is_stopped(&self) -> bool {
        matches!(self.state, SchedulerState::Stopped)
    }

    /// True while this scheduler is paused awaiting [`Self::rebuffer`].
    #[must_use]
    pub const fn is_awaiting_rebuffer(&self) -> bool {
        matches!(self.state, SchedulerState::AwaitingRebuffer)
    }

    /// Interleaved channel count of the stream this scheduler serves.
    #[must_use]
    pub const fn channels(&self) -> u16 {
        self.config.channels
    }

    /// What this scheduler is currently doing.
    #[must_use]
    pub const fn phase(&self) -> PlaybackPhase {
        match self.state {
            SchedulerState::Buffering => PlaybackPhase::Buffering,
            SchedulerState::Playing => PlaybackPhase::Playing,
            SchedulerState::AwaitingRebuffer => PlaybackPhase::AwaitingRebuffer,
            SchedulerState::Stopped => PlaybackPhase::Stopped,
        }
    }

    /// Cumulative packet-ordering counters from the internal jitter buffer.
    #[must_use]
    pub const fn jitter_statistics(&self) -> JitterBufferStatistics {
        self.jitter_buffer.statistics()
    }

    /// Cumulative concealment counters from the internal policy.
    #[must_use]
    pub const fn concealment_statistics(&self) -> ConcealmentStatistics {
        self.concealment.statistics()
    }

    /// Buffered presentation-time span currently held, in milliseconds.
    #[must_use]
    pub fn buffered_span_ms(&self) -> u64 {
        self.jitter_buffer.buffered_span_ms()
    }

    /// Exact sample rate configured for this stream.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// Maps a host presentation time onto this scheduler's local timeline
    /// using its current clock-offset estimate.
    #[must_use]
    pub fn local_time_for_host_ms(&self, host_time_ms: u64) -> u64 {
        host_to_local_ms(host_time_ms, self.offset_ms)
    }
}

fn validate_scheduler_config(config: &SchedulerConfig) -> Result<(), SchedulerConfigError> {
    if config.sample_rate == 0 {
        return Err(SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidPacketDuration,
            message: "sample rate must be nonzero".to_owned(),
        });
    }
    if config.samples_per_packet == 0 {
        return Err(SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidSamplesPerPacket,
            message: "samples per packet must be nonzero".to_owned(),
        });
    }

    let packet_time_numerator = u64::from(config.samples_per_packet) * 1_000;
    let minimum_time_numerator = u64::from(MIN_PACKET_DURATION_MS) * u64::from(config.sample_rate);
    let maximum_time_numerator = u64::from(MAX_PACKET_DURATION_MS) * u64::from(config.sample_rate);
    if packet_time_numerator < minimum_time_numerator
        || packet_time_numerator > maximum_time_numerator
    {
        return Err(SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidPacketDuration,
            message: format!(
                "packet geometry of {} frames at {}Hz is outside the supported {}..={}ms range",
                config.samples_per_packet,
                config.sample_rate,
                MIN_PACKET_DURATION_MS,
                MAX_PACKET_DURATION_MS,
            ),
        });
    }
    if config.low_water_ms >= config.high_water_ms {
        return Err(SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidWaterMarks,
            message: format!(
                "low water of {}ms must be strictly less than high water of {}ms",
                config.low_water_ms, config.high_water_ms
            ),
        });
    }
    if config.hard_resync_threshold_ms <= 0.0 {
        return Err(SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidHardResyncThreshold,
            message: "hard resync threshold must be positive".to_owned(),
        });
    }
    // The jitter buffer rejects anything beyond the reorder window, so a gap
    // can never be wider than the window itself. A threshold at or above it
    // would silently disable the skip policy rather than tune it.
    if config.concealment_skip_threshold_packets >= config.max_reorder_window {
        return Err(SchedulerConfigError {
            kind: SchedulerConfigErrorKind::InvalidConcealmentSkipThreshold,
            message: format!(
                "concealment skip threshold of {} packets must be smaller than the \
                 {}-packet reorder window",
                config.concealment_skip_threshold_packets, config.max_reorder_window
            ),
        });
    }
    Ok(())
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(super) fn host_to_local_ms(host_time_ms: u64, offset_ms: f64) -> u64 {
    let local_ms = host_time_ms as f64 - offset_ms;
    if local_ms <= 0.0 {
        0
    } else {
        local_ms.round() as u64
    }
}

fn decode_payload_samples(payload: &[u8]) -> Vec<i16> {
    payload
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}
