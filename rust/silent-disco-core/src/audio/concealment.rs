use core::fmt;
use std::error::Error;

/// Default bound on consecutive concealed packets before playback signals
/// that a hard resync/rebuffer is required.
pub const DEFAULT_MAX_CONSECUTIVE_CONCEALED_PACKETS: u32 = 5;
/// Hard ceiling on [`ConcealmentPolicy`]'s configured consecutive-concealment bound.
pub const MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT: u32 = 200;

/// Stable failure taxonomy for concealment policy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcealmentConfigErrorKind {
    /// The configured consecutive-concealment bound is zero or exceeds
    /// [`MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT`].
    ConsecutiveBoundOutOfRange,
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
    /// The gap was concealed with silence; playback may continue.
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
    /// packet delivery or the last [`ConcealmentPolicy::reset`].
    pub consecutive_concealed_packets: u32,
    /// Number of times the consecutive bound was reached, requiring a hard
    /// resync/rebuffer.
    pub hard_resync_signals: u64,
}

/// Silence-concealment policy for missing PCM packets in exactly one stream.
///
/// This policy only ever synthesizes fresh zero-filled silence; it never
/// retains or replays a prior packet's samples, so a concealed frame can
/// never leak previously played audio.
#[derive(Debug)]
pub struct ConcealmentPolicy {
    max_consecutive_concealed_packets: u32,
    statistics: ConcealmentStatistics,
}

impl ConcealmentPolicy {
    /// Creates a concealment policy bounded by `max_consecutive_concealed_packets`.
    ///
    /// # Errors
    ///
    /// Returns [`ConcealmentConfigErrorKind::ConsecutiveBoundOutOfRange`] when
    /// `max_consecutive_concealed_packets` is zero or exceeds
    /// [`MAX_CONSECUTIVE_CONCEALED_PACKETS_LIMIT`].
    pub fn new(max_consecutive_concealed_packets: u32) -> Result<Self, ConcealmentConfigError> {
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
        Ok(Self {
            max_consecutive_concealed_packets,
            statistics: ConcealmentStatistics::default(),
        })
    }

    /// Synthesizes exactly one silent, interleaved PCM frame for a missing
    /// packet slot and reports whether the consecutive-concealment bound has
    /// now been reached.
    pub fn conceal(
        &mut self,
        samples_per_packet: u32,
        channels: u16,
    ) -> (Vec<i16>, ConcealmentOutcome) {
        let sample_count = usize::try_from(samples_per_packet)
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::from(channels));
        let silence = vec![0_i16; sample_count];

        self.statistics.total_concealed_packets += 1;
        self.statistics.consecutive_concealed_packets += 1;

        if self.statistics.consecutive_concealed_packets >= self.max_consecutive_concealed_packets {
            self.statistics.hard_resync_signals += 1;
            (silence, ConcealmentOutcome::HardResyncRequired)
        } else {
            (silence, ConcealmentOutcome::Concealed)
        }
    }

    /// Records that a real (non-concealed) packet was delivered, resetting
    /// the consecutive-concealment count. Cumulative totals are unaffected.
    pub fn record_delivery(&mut self) {
        self.statistics.consecutive_concealed_packets = 0;
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
