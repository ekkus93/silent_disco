use core::fmt;
use std::error::Error;

use super::ramp::{apply_fade_in, apply_fade_out_tail};
use super::{
    ConcealmentOutcome, ConcealmentPolicy, DEFAULT_CONCEALMENT_RAMP_MS,
    DEFAULT_MAX_BUFFERED_DURATION_MS, DEFAULT_MAX_CONSECUTIVE_CONCEALED_PACKETS,
    DEFAULT_MAX_REORDER_WINDOW, JitterBuffer, JitterBufferConfig, JitterBufferRejection,
    MAX_PACKET_DURATION_MS, MIN_PACKET_DURATION_MS,
};
use crate::domain::{SessionId, StreamId};
use crate::protocol::AudioDatagram;

/// Default presentation buffer, in milliseconds, accumulated before playback
/// starts for a fresh or rebuffering stream.
pub const DEFAULT_STARTUP_BUFFER_TARGET_MS: u64 = 400;
/// Default buffered-span threshold, in milliseconds, below which
/// [`BufferHealth::Low`] is reported.
pub const DEFAULT_LOW_WATER_MS: u64 = 200;
/// Default buffered-span threshold, in milliseconds, above which
/// [`BufferHealth::High`] is reported.
pub const DEFAULT_HIGH_WATER_MS: u64 = 700;
/// Default magnitude of host/local clock offset change, in milliseconds,
/// beyond which an updated sync estimate forces a hard resync rather than a
/// soft correction.
pub const DEFAULT_HARD_RESYNC_THRESHOLD_MS: f64 = 120.0;
/// Default gap width, in packets, beyond which a hole is skipped outright
/// rather than covered packet by packet.
pub const DEFAULT_CONCEALMENT_SKIP_THRESHOLD_PACKETS: u32 = 10;

/// Fixed identity, geometry, and tuning bounds for one playback scheduler,
/// covering exactly one host stream.
#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerConfig {
    /// Session this scheduler accepts packets for.
    pub session_id: SessionId,
    /// Stream this scheduler accepts packets for.
    pub stream_id: StreamId,
    /// Wire packet duration, matching the host packetizer's configuration
    /// for this stream.
    pub packet_duration_ms: u32,
    /// Host monotonic time at which sequence zero's presentation slot began,
    /// matching the host packetizer's `host_start_time_ms`.
    pub host_start_time_ms: u64,
    /// Samples per channel per packet, matching the host packetizer's
    /// configuration for this stream.
    pub samples_per_packet: u32,
    /// Interleaved channel count, matching the host packetizer's format.
    pub channels: u16,
    /// Presentation buffer accumulated before playback starts or resumes
    /// after a rebuffer.
    pub startup_buffer_target_ms: u64,
    /// Buffered-span threshold below which [`BufferHealth::Low`] is reported.
    pub low_water_ms: u64,
    /// Buffered-span threshold above which [`BufferHealth::High`] is reported.
    pub high_water_ms: u64,
    /// Magnitude of clock-offset change beyond which an updated sync
    /// estimate forces a hard resync rather than a soft correction.
    pub hard_resync_threshold_ms: f64,
    /// Consecutive concealed packets tolerated before a hard resync is
    /// required; forwarded to the internal [`ConcealmentPolicy`].
    pub max_consecutive_concealed_packets: u32,
    /// Amplitude-ramp length, in milliseconds, applied at both edges of every
    /// concealed frame so neither seam steps discontinuously.
    pub concealment_ramp_ms: u32,
    /// Gap width, in packets, beyond which the missing range is skipped
    /// outright instead of concealed packet by packet. Must be smaller than
    /// `max_reorder_window`, since no wider gap can ever be observed.
    pub concealment_skip_threshold_packets: u32,
    /// Reorder window tolerated by the internal [`JitterBuffer`].
    pub max_reorder_window: u32,
    /// Maximum buffered duration tolerated by the internal [`JitterBuffer`].
    pub max_buffered_duration_ms: u64,
}

impl SchedulerConfig {
    /// Convenience constructor using the recommended default tuning bounds.
    #[must_use]
    pub const fn new(
        session_id: SessionId,
        stream_id: StreamId,
        packet_duration_ms: u32,
        host_start_time_ms: u64,
        samples_per_packet: u32,
        channels: u16,
    ) -> Self {
        Self {
            session_id,
            stream_id,
            packet_duration_ms,
            host_start_time_ms,
            samples_per_packet,
            channels,
            startup_buffer_target_ms: DEFAULT_STARTUP_BUFFER_TARGET_MS,
            low_water_ms: DEFAULT_LOW_WATER_MS,
            high_water_ms: DEFAULT_HIGH_WATER_MS,
            hard_resync_threshold_ms: DEFAULT_HARD_RESYNC_THRESHOLD_MS,
            max_consecutive_concealed_packets: DEFAULT_MAX_CONSECUTIVE_CONCEALED_PACKETS,
            concealment_ramp_ms: DEFAULT_CONCEALMENT_RAMP_MS,
            concealment_skip_threshold_packets: DEFAULT_CONCEALMENT_SKIP_THRESHOLD_PACKETS,
            max_reorder_window: DEFAULT_MAX_REORDER_WINDOW,
            max_buffered_duration_ms: DEFAULT_MAX_BUFFERED_DURATION_MS,
        }
    }
}

/// Stable failure taxonomy for scheduler configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerConfigErrorKind {
    /// `packet_duration_ms` is outside the packetizer's supported range.
    InvalidPacketDuration,
    /// `samples_per_packet` is zero.
    InvalidSamplesPerPacket,
    /// `low_water_ms` is not strictly less than `high_water_ms`.
    InvalidWaterMarks,
    /// `hard_resync_threshold_ms` is not positive.
    InvalidHardResyncThreshold,
    /// The configured reorder window or buffered-duration bound was rejected
    /// by the internal [`JitterBuffer`].
    InvalidJitterBufferBounds,
    /// The configured consecutive-concealment bound was rejected by the
    /// internal [`ConcealmentPolicy`].
    InvalidConcealmentBound,
    /// `concealment_skip_threshold_packets` is not smaller than
    /// `max_reorder_window`, so no observable gap could ever reach it and the
    /// skip policy would never engage.
    InvalidConcealmentSkipThreshold,
}

/// Typed scheduler configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerConfigError {
    /// Stable semantic failure category.
    pub kind: SchedulerConfigErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for SchedulerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for SchedulerConfigError {}

/// Observed buffered-span health relative to the configured low/high water
/// thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferHealth {
    /// Buffered span is below the configured low-water threshold.
    Low,
    /// Buffered span is within the configured healthy range.
    Normal,
    /// Buffered span is above the configured high-water threshold.
    High,
}

/// One ordered, render-ready interleaved PCM frame, real or concealed.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledFrame {
    /// Wire sequence number this frame corresponds to, whether or not the
    /// underlying packet actually arrived.
    pub sequence: u64,
    /// First interleaved sample index this frame covers.
    pub first_sample_index: u64,
    /// Host monotonic presentation time this frame was scheduled for.
    pub host_presentation_time_ms: u64,
    /// Interleaved PCM samples, delivered or synthesized.
    pub samples: Vec<i16>,
    /// True when this frame was synthesized to cover a missing packet rather
    /// than decoded from a delivered one.
    pub concealed: bool,
}

/// Result of one scheduler tick.
#[derive(Debug, Clone, PartialEq)]
pub enum SchedulerPoll {
    /// Still accumulating the startup presentation buffer.
    Buffering {
        /// Buffered span accumulated so far, in milliseconds.
        buffered_ms: u64,
    },
    /// A frame is ready for this tick's presentation slot.
    Frame {
        /// The frame to render.
        frame: ScheduledFrame,
        /// Buffered-span health after this frame was taken.
        buffer_health: BufferHealth,
    },
    /// This tick's presentation slot has not arrived yet; nothing to render.
    Waiting {
        /// Buffered-span health at this tick.
        buffer_health: BufferHealth,
    },
    /// Too many consecutive concealed packets, or too large a clock-offset
    /// jump; playback is paused until [`PlaybackScheduler::rebuffer`] is called.
    AwaitingRebuffer,
    /// [`PlaybackScheduler::stop`] was called; this scheduler produces no
    /// further frames.
    Stopped,
}

/// Outcome of applying an updated host/local clock-offset estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetUpdateOutcome {
    /// The offset changed by less than the hard-resync threshold and was
    /// applied in place; playback continues without interruption.
    SoftCorrected,
    /// The offset changed by at least the hard-resync threshold; playback is
    /// now paused until [`PlaybackScheduler::rebuffer`] is called.
    HardResyncRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchedulerState {
    Buffering,
    Playing,
    AwaitingRebuffer,
    Stopped,
}

/// Maps a bounded jitter buffer and a silence-concealment policy onto a
/// host-presentation timeline, producing one ordered, render-ready PCM frame
/// per tick for exactly one stream.
///
/// This does not itself own or write into a real-time render ring; it
/// produces the frames a future render-ring producer worker will pace into
/// one. `poll` performs no allocation beyond one `Vec<i16>` per frame, no
/// I/O, and no blocking, so it is safe to call at the stream's packet-duration
/// cadence.
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
}

impl PlaybackScheduler {
    /// Creates a scheduler for one stream with an initial host/local
    /// clock-offset estimate.
    ///
    /// # Errors
    ///
    /// Returns a [`SchedulerConfigError`] when `packet_duration_ms` is
    /// outside the packetizer's supported range, `samples_per_packet` is
    /// zero, the water marks are not strictly ordered, the hard-resync
    /// threshold is not positive, or the jitter buffer/concealment bounds
    /// are individually invalid.
    pub fn new(
        config: SchedulerConfig,
        initial_offset_ms: f64,
    ) -> Result<Self, SchedulerConfigError> {
        if !(MIN_PACKET_DURATION_MS..=MAX_PACKET_DURATION_MS).contains(&config.packet_duration_ms) {
            return Err(SchedulerConfigError {
                kind: SchedulerConfigErrorKind::InvalidPacketDuration,
                message: format!(
                    "packet duration of {}ms is outside the supported range",
                    config.packet_duration_ms
                ),
            });
        }
        if config.samples_per_packet == 0 {
            return Err(SchedulerConfigError {
                kind: SchedulerConfigErrorKind::InvalidSamplesPerPacket,
                message: "samples per packet must be nonzero".to_owned(),
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
        // The jitter buffer rejects anything beyond the reorder window, so a
        // gap can never be wider than the window itself. A threshold at or
        // above it would silently disable the skip policy rather than tune it.
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
        // The ramp is expressed in milliseconds but applied in frames; the
        // stream's sample rate is implied by its validated packet geometry.
        let ramp_frames = (config
            .samples_per_packet
            .saturating_mul(config.concealment_ramp_ms)
            / config.packet_duration_ms)
            .max(1);
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
        self.config.host_start_time_ms + sequence * u64::from(self.config.packet_duration_ms)
    }

    fn expected_first_sample_index(&self, sequence: u64) -> u64 {
        sequence * u64::from(self.config.samples_per_packet)
    }

    /// Advances the scheduler by one tick at the given local monotonic time,
    /// returning the frame ready for this tick's presentation slot, if any.
    ///
    /// # Panics
    ///
    /// Never panics; every arithmetic path is bounded by validated
    /// configuration.
    #[must_use]
    pub fn poll(&mut self, local_now_ms: u64) -> SchedulerPoll {
        match self.state {
            SchedulerState::Stopped => return SchedulerPoll::Stopped,
            SchedulerState::AwaitingRebuffer => return SchedulerPoll::AwaitingRebuffer,
            SchedulerState::Buffering => {
                let buffered_ms = self.jitter_buffer.buffered_span_ms();
                if buffered_ms < self.config.startup_buffer_target_ms {
                    return SchedulerPoll::Buffering { buffered_ms };
                }
                self.state = SchedulerState::Playing;
            }
            SchedulerState::Playing => {}
        }

        let mut next_sequence = self.jitter_buffer.next_expected_sequence();
        let mut host_deadline_ms = self.expected_host_presentation_time_ms(next_sequence);
        let mut local_deadline_ms = host_to_local_ms(host_deadline_ms, self.offset_ms);

        if local_now_ms < local_deadline_ms {
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
            self.fade_in_next_real_frame = true;

            next_sequence = self.jitter_buffer.next_expected_sequence();
            host_deadline_ms = self.expected_host_presentation_time_ms(next_sequence);
            local_deadline_ms = host_to_local_ms(host_deadline_ms, self.offset_ms);
            if local_now_ms < local_deadline_ms {
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
            if self.fade_in_next_real_frame {
                apply_fade_in(
                    &mut samples,
                    usize::from(self.config.channels),
                    self.ramp_frames,
                );
                self.fade_in_next_real_frame = false;
            }
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
        // Concealed content fades to zero, so whatever real audio resumes
        // after it must fade back in rather than stepping up from silence.
        self.fade_in_next_real_frame = true;

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
        // A concealed frame faded to zero, so the first drained frame resumes
        // from silence even when it continues the sequence without a hole.
        let mut fade_in_next = self.fade_in_next_real_frame;
        let mut frames: Vec<ScheduledFrame> = Vec::new();

        for datagram in self.jitter_buffer.drain_all() {
            let sequence = datagram.sequence.get();
            if sequence > expected_next {
                if let Some(previous) = frames.last_mut() {
                    apply_fade_out_tail(&mut previous.samples, channels, ramp);
                }
                fade_in_next = true;
            }
            let mut samples = decode_payload_samples(&datagram.payload);
            if fade_in_next {
                apply_fade_in(&mut samples, channels, ramp);
                fade_in_next = false;
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
        frames
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
        // Playback resumes after a real interruption; fade back in.
        self.fade_in_next_real_frame = true;
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

    /// Sample rate implied by this stream's validated packet geometry.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.config.samples_per_packet * 1_000 / self.config.packet_duration_ms
    }

    /// Maps a host presentation time onto this scheduler's local timeline
    /// using its current clock-offset estimate.
    #[must_use]
    pub fn local_time_for_host_ms(&self, host_time_ms: u64) -> u64 {
        host_to_local_ms(host_time_ms, self.offset_ms)
    }
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
