//! Stable failure taxonomy for [`SchedulerConfig`](super::config::SchedulerConfig)
//! validation.

use core::fmt;
use std::error::Error;

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
    /// by the internal `JitterBuffer`.
    InvalidJitterBufferBounds,
    /// The configured consecutive-concealment bound was rejected by the
    /// internal `ConcealmentPolicy`.
    InvalidConcealmentBound,
    /// `concealment_skip_threshold_packets` is not smaller than
    /// `max_reorder_window`, so no observable gap could ever reach it and the
    /// skip policy would never engage.
    InvalidConcealmentSkipThreshold,
    /// `concealment_ramp_ms` resolves to a ramp at least as long as one
    /// packet, leaving no steady-state body between the two shaped edges.
    InvalidConcealmentRamp,
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
