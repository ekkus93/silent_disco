//! Diagnostics and output types produced by
//! [`PlaybackScheduler`](super::engine::PlaybackScheduler), plus its private
//! lifecycle state.

/// What a scheduler is currently doing, for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackPhase {
    /// Accumulating the startup presentation buffer.
    Buffering,
    /// Delivering frames against the presentation timeline.
    Playing,
    /// Paused until an explicit rebuffer.
    AwaitingRebuffer,
    /// Stopped; no further frames will be produced.
    Stopped,
}

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
    /// jump; playback is paused until
    /// [`PlaybackScheduler::rebuffer`](super::engine::PlaybackScheduler::rebuffer)
    /// is called.
    AwaitingRebuffer,
    /// [`PlaybackScheduler::stop`](super::engine::PlaybackScheduler::stop)
    /// was called; this scheduler produces no further frames.
    Stopped,
}

/// Outcome of applying an updated host/local clock-offset estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetUpdateOutcome {
    /// The offset changed by less than the hard-resync threshold and was
    /// applied in place; playback continues without interruption.
    SoftCorrected,
    /// The offset changed by at least the hard-resync threshold; playback is
    /// now paused until
    /// [`PlaybackScheduler::rebuffer`](super::engine::PlaybackScheduler::rebuffer)
    /// is called.
    HardResyncRequired,
}

/// Internal lifecycle state, distinct from the diagnostics-facing
/// [`PlaybackPhase`] so the engine can add transient states later without
/// changing the public enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SchedulerState {
    Buffering,
    Playing,
    AwaitingRebuffer,
    Stopped,
}
