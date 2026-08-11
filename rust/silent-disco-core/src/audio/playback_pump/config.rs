//! Tuning for a [`PlaybackPump`](super::PlaybackPump) and its construction-
//! validation failure taxonomy.

use core::fmt;
use std::error::Error;

use crate::audio::DEFAULT_TARGET_FILL_FRAMES;

/// Default distance ahead of its presentation deadline at which a frame is
/// handed to the render ring.
pub const DEFAULT_WRITE_LEAD_MS: u64 = 400;
/// Default ceiling on the stream-start silence prefill.
pub const DEFAULT_MAX_PREFILL_MS: u64 = 800;

/// Tuning for one [`PlaybackPump`](super::PlaybackPump).
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
