use core::fmt;
use std::error::Error;

use super::ramp::{apply_fade_out_tail, blend_sample, last_frame};

/// Default bound on consecutive concealed packets before playback signals
/// that a hard resync/rebuffer is required.
///
/// Twenty-five packets is 500ms at the standard 20ms packet duration, the
/// bridge length validated on a physical device. The bound is deliberately
/// not tight: reaching it forces the scheduler to re-accumulate its whole
/// startup buffer, so a bound short enough to trip on an ordinary brief
/// outage would replace a 100ms interruption with a far longer rebuffering
/// silence. Decaying repetition has already faded to inaudibility within the
/// first handful of packets, so the later part of the bridge is effectively
/// silence held open in case real audio resumes.
pub const DEFAULT_MAX_CONSECUTIVE_CONCEALED_PACKETS: u32 = 25;
/// Hard ceiling on [`ConcealmentPolicy`]'s configured consecutive-concealment bound.
pub const MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT: u32 = 200;
/// Default amplitude-ramp length, in milliseconds, applied at both edges of
/// every concealed frame.
///
/// One millisecond is the declick length: long enough that a splice between
/// two independently produced buffers carries no audible step, short enough
/// not to smear real content. It is deliberately an absolute duration rather
/// than a fraction of a packet, because it exists to defeat the ear's
/// sensitivity to discontinuity and that does not depend on how the stream is
/// packetized. It must also stay strictly shorter than one packet — a 5ms
/// ramp does not, once packets are 5ms long.
pub const DEFAULT_CONCEALMENT_RAMP_MS: u32 = 1;
/// Hard ceiling on the configured concealment ramp, in frames.
pub const MAX_CONCEALMENT_RAMP_FRAMES: u32 = 4_800;
/// Largest right-shift applied to the repeated source packet. Beyond this the
/// repetition is inaudible, so further attenuation buys nothing.
const MAX_ATTENUATION_SHIFT: u32 = 8;

/// Stable failure taxonomy for concealment policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcealmentConfigErrorKind {
    /// The configured consecutive-concealment bound is zero or exceeds
    /// [`MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT`].
    ConsecutiveBoundOutOfRange,
    /// The configured ramp length is zero or exceeds
    /// [`MAX_CONCEALMENT_RAMP_FRAMES`].
    RampFramesOutOfRange,
}

/// Typed concealment configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcealmentConfigError {
    /// Stable semantic failure category.
    pub kind: ConcealmentConfigErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl fmt::Display for ConcealmentConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for ConcealmentConfigError {}

/// Outcome of synthesizing one concealment frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcealmentOutcome {
    /// The gap was concealed; playback may continue.
    Concealed,
    /// The configured consecutive-concealment bound was reached; the caller
    /// must treat this as a hard desync and rebuffer before continuing.
    HardResyncRequired,
}

/// Cumulative counters describing everything this policy has observed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConcealmentStatistics {
    /// Total concealment frames synthesized across this policy's lifetime.
    pub total_concealed_packets: u64,
    /// Concealment frames synthesized back-to-back since the last real
    /// packet delivery or the last [`ConcealmentPolicy::reset_consecutive_count`].
    pub consecutive_concealed_packets: u32,
    /// Number of times the consecutive bound was reached, requiring a hard
    /// resync/rebuffer.
    pub hard_resync_signals: u64,
}

/// Concealment policy for missing PCM packets in exactly one stream.
///
/// A lost packet replaced by silence leaves an audible hole: at a 20ms packet
/// duration the hole is plainly perceptible in tonal content no matter how
/// smoothly its edges are ramped, and real-device listening confirmed it as
/// the dominant source of reported "popping and static". Instead this policy
/// repeats the last real packet's audio at progressively halved amplitude, so
/// isolated losses continue the sound rather than interrupting it, while a
/// sustained outage still decays to silence within a few packets rather than
/// looping one packet indefinitely.
///
/// Every synthesized frame begins at the exact sample values the previously
/// emitted frame ended on, so a run of them is one continuous decaying
/// waveform rather than a sequence of separately-ramped blips. Only the final
/// frame of a bounded run fades to silence; when real audio resumes instead,
/// the caller blends it in from [`ConcealmentPolicy::previous_tail`].
#[derive(Debug)]
pub struct ConcealmentPolicy {
    max_consecutive_concealed_packets: u32,
    ramp_frames: usize,
    statistics: ConcealmentStatistics,
    last_real_samples: Vec<i16>,
    previous_tail: Vec<i16>,
}

impl ConcealmentPolicy {
    /// Creates a concealment policy bounded by `max_consecutive_concealed_packets`
    /// and shaping every synthesized frame's edges over `ramp_frames` frames.
    ///
    /// # Errors
    ///
    /// Returns [`ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange`] when
    /// `max_consecutive_concealed_packets` is zero or exceeds
    /// [`MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT`], or
    /// [`ConcealmentConfigErrorKind::RampFramesOutOfRange`] when `ramp_frames`
    /// is zero or exceeds [`MAX_CONCEALMENT_RAMP_FRAMES`].
    pub fn new(
        max_consecutive_concealed_packets: u32,
        ramp_frames: u32,
    ) -> Result<Self, ConcealmentConfigError> {
        if max_consecutive_concealed_packets == 0
            || max_consecutive_concealed_packets > MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT
        {
            return Err(ConcealmentConfigError {
                kind: ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange,
                message: format!(
                    "consecutive concealment bound of {max_consecutive_concealed_packets} must be \
                     nonzero and within the {MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT}-packet limit"
                ),
            });
        }
        if ramp_frames == 0 || ramp_frames > MAX_CONCEALMENT_RAMP_FRAMES {
            return Err(ConcealmentConfigError {
                kind: ConcealmentConfigErrorKind::RampFramesOutOfRange,
                message: format!(
                    "concealment ramp of {ramp_frames} frames must be nonzero and within the \
                     {MAX_CONCEALMENT_RAMP_FRAMES}-frame limit"
                ),
            });
        }
        Ok(Self {
            max_consecutive_concealed_packets,
            ramp_frames: usize::try_from(ramp_frames).unwrap_or(usize::MAX),
            statistics: ConcealmentStatistics::default(),
            last_real_samples: Vec::new(),
            previous_tail: Vec::new(),
        })
    }

    /// Synthesizes exactly one interleaved PCM frame for a missing packet slot
    /// and reports whether the consecutive-concealment bound has now been
    /// reached.
    ///
    /// The frame repeats the most recently delivered real packet attenuated by
    /// one further halving per consecutive concealment, blended in from the
    /// previously emitted frame's final sample values. Its tail is faded to
    /// zero only when this frame reaches the consecutive bound and so ends the
    /// run. Before any real packet has been delivered there is nothing to
    /// repeat and the frame is silence.
    pub fn conceal(
        &mut self,
        samples_per_packet: u32,
        channels: u16,
    ) -> (Vec<i16>, ConcealmentOutcome) {
        let channel_count = usize::from(channels);
        let frame_count = usize::try_from(samples_per_packet).unwrap_or(usize::MAX);
        let attenuation_shift =
            (self.statistics.consecutive_concealed_packets + 1).min(MAX_ATTENUATION_SHIFT);
        let mut samples = self.build_payload(frame_count, channel_count, attenuation_shift);

        self.statistics.total_concealed_packets += 1;
        self.statistics.consecutive_concealed_packets += 1;
        let consecutive = self.statistics.consecutive_concealed_packets;
        let bound_reached = consecutive >= self.max_consecutive_concealed_packets;

        // Only a run's final frame lands on silence. Fading every frame's tail
        // to zero -- and blending the next one back in from that zero -- turned
        // a burst of lost packets into one amplitude dip per packet, audible as
        // modulation at the packet rate rather than as a single decaying gap.
        // Carrying the un-faded tail forward lets the halving attenuation reach
        // silence on its own while the waveform stays continuous throughout.
        //
        // "Final" covers the bound frame *and* the one before it, because a
        // caller that rebuffers on [`ConcealmentOutcome::HardResyncRequired`]
        // discards the bound frame rather than playing it: without the earlier
        // fade, the last frame anyone actually hears would end mid-waveform.
        if consecutive + 1 >= self.max_consecutive_concealed_packets {
            apply_fade_out_tail(&mut samples, channel_count, self.ramp_frames);
        }
        self.previous_tail =
            last_frame(&samples, channel_count).map_or_else(Vec::new, <[i16]>::to_vec);

        if bound_reached {
            self.statistics.hard_resync_signals += 1;
            (samples, ConcealmentOutcome::HardResyncRequired)
        } else {
            (samples, ConcealmentOutcome::Concealed)
        }
    }

    /// Per-channel sample values the most recently synthesized frame ended on,
    /// so a caller splicing real audio after concealment can continue from them
    /// instead of stepping. Empty before any concealment.
    #[must_use]
    pub fn previous_tail(&self) -> &[i16] {
        &self.previous_tail
    }

    fn build_payload(
        &self,
        frame_count: usize,
        channel_count: usize,
        attenuation_shift: u32,
    ) -> Vec<i16> {
        let mut samples = vec![0_i16; frame_count.saturating_mul(channel_count)];
        if channel_count == 0 || frame_count == 0 {
            return samples;
        }
        let ramp = self.ramp_frames.clamp(1, frame_count);
        let source_frames = self.last_real_samples.len() / channel_count;
        let has_tail = self.previous_tail.len() == channel_count;

        for frame in 0..frame_count {
            for channel in 0..channel_count {
                let index = frame * channel_count + channel;
                let mut value = if frame < source_frames {
                    self.last_real_samples[index] >> attenuation_shift
                } else {
                    0
                };
                if frame < ramp && has_tail {
                    // Convex blend from the previously emitted frame's final
                    // value toward the repeated content: the seam has no step.
                    value = blend_sample(value, self.previous_tail[channel], frame, ramp);
                }
                samples[index] = value;
            }
        }
        samples
    }

    /// Records that a real (non-concealed) packet was delivered, resetting the
    /// consecutive-concealment count and adopting `samples` as the source for
    /// any subsequent concealment. Cumulative totals are unaffected.
    pub fn record_delivery(&mut self, samples: &[i16], channels: u16) {
        let channel_count = usize::from(channels);
        self.statistics.consecutive_concealed_packets = 0;
        self.last_real_samples.clear();
        self.last_real_samples.extend_from_slice(samples);
        self.previous_tail =
            last_frame(samples, channel_count).map_or_else(Vec::new, <[i16]>::to_vec);
    }

    /// Resets the consecutive-concealment count after an explicit rebuffer,
    /// without discarding cumulative lifetime totals.
    pub fn reset_consecutive_count(&mut self) {
        self.statistics.consecutive_concealed_packets = 0;
    }

    /// Cumulative counters describing everything this policy has observed.
    #[must_use]
    pub const fn statistics(&self) -> ConcealmentStatistics {
        self.statistics
    }
}
