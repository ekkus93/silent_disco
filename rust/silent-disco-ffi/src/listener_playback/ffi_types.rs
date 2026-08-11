//! `UniFFI` wire-shape DTOs for the listener playback surface.
//!
//! Every type here generates Kotlin/Swift bindings; conversions to and from
//! the shared-core types live in `ffi_convert.rs`, not here.

use core::fmt;

/// Everything needed to start one listener playback stream, flattened for the
/// foreign binding.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiListenerPlaybackConfig {
    /// Session this stream belongs to.
    pub session_id: String,
    /// Stream identity; a new stream generation requires a new runtime.
    pub stream_id: String,
    /// Wire packet duration, matching the host packetizer.
    pub packet_duration_ms: u32,
    /// Host monotonic time at which sequence zero's slot began.
    pub host_start_time_ms: u64,
    /// Samples per channel per packet.
    pub samples_per_packet: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// Presentation buffer accumulated before playback first starts.
    pub startup_buffer_target_ms: u64,
    /// Presentation buffer rebuilt before playback resumes after a
    /// mid-stream rebuffer. Deliberately separate from the startup target:
    /// this one is the length of an audible hole.
    pub rebuffer_target_ms: u64,
    /// Render ring capacity, in frames.
    pub ring_capacity_frames: u32,
    /// Ring depth the pump holds as its jitter cushion, in frames.
    pub ring_target_fill_frames: u32,
    /// How far ahead of its deadline each frame is written.
    pub write_lead_ms: u64,
    /// Ceiling on the stream-start alignment prefill.
    pub max_prefill_ms: u64,
    /// Linear output gain, in `0.0..=1.0`.
    pub volume: f32,
}

/// One arriving audio packet, flattened for the foreign binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiAudioPacket {
    /// Wire sequence number.
    pub sequence: u64,
    /// Sample rate the packet was encoded at.
    pub sample_rate: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// Samples per channel in this packet.
    pub samples_per_packet: u32,
    /// First interleaved sample index this packet covers.
    pub first_sample_index: u64,
    /// Host monotonic presentation time for this packet.
    pub host_presentation_time_ms: u64,
    /// Interleaved PCM16LE payload.
    pub payload: Vec<u8>,
}

/// What the scheduler is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPlaybackPhase {
    /// Accumulating the startup presentation buffer.
    Buffering,
    /// Delivering frames against the presentation timeline.
    Playing,
    /// Paused, re-accumulating before playback resumes.
    AwaitingRebuffer,
    /// Stopped; no further frames will be produced.
    Stopped,
}

/// Playback accounting, flattened for the foreign binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiPlaybackDiagnostics {
    /// Scheduler phase.
    pub phase: FfiPlaybackPhase,
    /// True once a real clock-offset estimate has been applied.
    pub sync_locked: bool,
    /// Packets accepted into the jitter buffer.
    pub packets_accepted: u64,
    /// Packets emitted in order, including the tail drained at stop.
    pub packets_emitted: u64,
    /// Sequences abandoned without playing: concealed losses plus skipped gaps.
    pub sequences_skipped: u64,
    /// Packets that arrived after their slot had already played.
    pub late_rejections: u64,
    /// Duplicate packets rejected.
    pub duplicate_rejections: u64,
    /// Packets too far ahead to reorder.
    pub reorder_window_rejections: u64,
    /// Times the buffer adopted a far-ahead position after the stream moved
    /// beyond its reorder window.
    pub resynchronisations: u64,
    /// Packets discarded because they arrived before a clock offset existed.
    pub dropped_before_sync: u64,
    /// Frames synthesized to cover missing packets.
    pub concealed_packets: u64,
    /// Times the concealment bound forced a rebuffer.
    pub concealment_driven_rebuffers: u64,
    /// Times a clock-offset jump too large to splice forced a rebuffer.
    pub offset_driven_rebuffers: u64,
    /// Every hard resync regardless of cause -- `concealment_driven_rebuffers`
    /// `+ offset_driven_rebuffers` (A4.4).
    pub hard_resync_signals: u64,
    /// Buffered presentation-time span currently held.
    pub buffered_span_ms: u64,
    /// Frames currently queued in the render ring.
    pub ring_queued_frames: u64,
    /// Largest ring depth observed.
    pub ring_peak_queued_frames: u64,
    /// Frames converted but not yet accepted by the ring.
    pub pending_frames: u64,
    /// Alignment silence queued at stream start.
    pub prefill_frames: u64,
    /// Real-time reads that had to substitute silence.
    pub ring_underruns: u64,
    /// Frames of silence those reads substituted.
    pub ring_silence_filled_frames: u64,
    /// Producer writes that could not fit every frame.
    pub ring_full_events: u64,
}

/// The estimator's confidence in its current estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSyncConfidence {
    /// No sample has been accepted yet.
    Unknown,
    /// Accepted, but the round trip or dispersion is poor.
    Poor,
    /// Usable.
    Fair,
    /// Good.
    Good,
    /// Excellent.
    Excellent,
}

/// Result of feeding one correlated sync response, flattened for the binding.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct FfiSyncSampleOutcome {
    /// True when the sample updated the estimate.
    pub accepted: bool,
    /// Current offset estimate, in milliseconds.
    pub offset_ms: f64,
    /// Current skew estimate, in parts per million.
    pub skew_ppm: f64,
    /// Round-trip time behind the current estimate.
    pub round_trip_time_ms: f64,
    /// Offset dispersion across the samples behind the current estimate.
    pub jitter_ms: f64,
    /// The estimator's own confidence in the current estimate.
    pub confidence: FfiSyncConfidence,
    /// Accepted samples behind the current estimate.
    pub accepted_sample_count: u64,
    /// True once playback has a real offset and may start.
    pub sync_locked: bool,
}

/// Errors surfaced to the foreign binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiListenerPlaybackError {
    /// The requested configuration was rejected.
    InvalidConfiguration(String),
    /// The handle was already stopped.
    Stopped(String),
    /// The pump thread failed to start or ended abnormally.
    PumpThread(String),
    /// A sync probe or response was rejected.
    Sync(String),
    /// The debug capture could not be started.
    DebugCapture(String),
}

impl fmt::Display for FfiListenerPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Stopped(message)
            | Self::PumpThread(message)
            | Self::Sync(message)
            | Self::DebugCapture(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FfiListenerPlaybackError {}
