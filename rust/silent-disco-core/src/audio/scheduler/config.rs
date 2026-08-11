//! Tuning constants and the fixed identity/geometry/tuning-bound
//! configuration for one [`PlaybackScheduler`](super::engine::PlaybackScheduler).

use crate::audio::{DEFAULT_CONCEALMENT_RAMP_MS, DEFAULT_MAX_BUFFERED_DURATION_MS};
use crate::domain::{SessionId, StreamId};

/// Default presentation buffer, in milliseconds, accumulated before playback
/// starts for a fresh or rebuffering stream.
pub const DEFAULT_STARTUP_BUFFER_TARGET_MS: u64 = 400;
/// Presentation span rebuilt before playback resumes after a *mid-stream*
/// rebuffer, as distinct from a stream's initial startup buffer.
///
/// A stream's first start can afford a generous cushion -- nobody is
/// listening yet. A mid-stream recovery cannot: every millisecond of it is
/// a hole in audio the listener is already hearing, and the span rebuilds
/// at 1x real time, so the target *is* the outage length. Reusing the
/// startup target for both turned a ~500ms arrival stall into ~1.5-2s of
/// silence on a real device (LG G6, 2026-08-09), because that platform
/// sets a 1000ms startup buffer.
pub const DEFAULT_REBUFFER_TARGET_MS: u64 = 400;
/// Default buffered-span threshold, in milliseconds, below which
/// [`BufferHealth::Low`](super::types::BufferHealth::Low) is reported.
pub const DEFAULT_LOW_WATER_MS: u64 = 200;
/// Default buffered-span threshold, in milliseconds, above which
/// [`BufferHealth::High`](super::types::BufferHealth::High) is reported.
pub const DEFAULT_HIGH_WATER_MS: u64 = 700;
/// Default magnitude of host/local clock offset change, in milliseconds,
/// beyond which an updated sync estimate forces a hard resync rather than a
/// soft correction.
pub const DEFAULT_HARD_RESYNC_THRESHOLD_MS: f64 = 120.0;
/// Default gap width, in packets, beyond which a hole is skipped outright
/// rather than covered packet by packet, at the default packet duration.
///
/// Prefer [`DEFAULT_CONCEALMENT_SKIP_THRESHOLD_MS`]: the meaningful quantity
/// is how much *audio* a hole covers, not how many packets it spans.
pub const DEFAULT_CONCEALMENT_SKIP_THRESHOLD_PACKETS: u32 = 10;

/// Default consecutive-concealment bridge, in milliseconds, before playback
/// gives up and rebuffers.
pub const DEFAULT_CONCEALMENT_BRIDGE_MS: u32 = 500;
/// Default gap width, in milliseconds, beyond which a hole is abandoned
/// outright rather than covered packet by packet.
pub const DEFAULT_CONCEALMENT_SKIP_THRESHOLD_MS: u32 = 200;
/// Default reorder tolerance, in milliseconds of stream time.
pub const DEFAULT_REORDER_WINDOW_MS: u32 = 1_280;

/// Converts a duration in milliseconds into whole packets at
/// `packet_duration_ms`, never returning zero.
///
/// The tuning bounds below are all really statements about *time* — how long
/// a bridge may last, how much audio a hole may cover, how far out of order a
/// packet may arrive. Storing them as packet counts silently rescales every
/// one of them whenever the packet duration changes, which is exactly the
/// class of regression that hides until a device run.
pub(super) const fn packets_spanning(duration_ms: u32, packet_duration_ms: u32) -> u32 {
    if packet_duration_ms == 0 {
        return 1;
    }
    let packets = duration_ms / packet_duration_ms;
    if packets == 0 { 1 } else { packets }
}

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
    /// Presentation buffer accumulated before playback starts for the first
    /// time. Applies only to a stream's initial start; a mid-stream recovery
    /// uses [`Self::rebuffer_target_ms`].
    pub startup_buffer_target_ms: u64,
    /// Presentation buffer rebuilt before playback resumes after a
    /// mid-stream rebuffer. See [`DEFAULT_REBUFFER_TARGET_MS`] for why this
    /// is deliberately separate from the startup target.
    pub rebuffer_target_ms: u64,
    /// Buffered-span threshold below which [`BufferHealth::Low`](super::types::BufferHealth::Low) is reported.
    pub low_water_ms: u64,
    /// Buffered-span threshold above which [`BufferHealth::High`](super::types::BufferHealth::High) is reported.
    pub high_water_ms: u64,
    /// Magnitude of clock-offset change beyond which an updated sync
    /// estimate forces a hard resync rather than a soft correction.
    pub hard_resync_threshold_ms: f64,
    /// Consecutive concealed packets tolerated before a hard resync is
    /// required; forwarded to the internal `ConcealmentPolicy`.
    pub max_consecutive_concealed_packets: u32,
    /// Amplitude-ramp length, in milliseconds, applied at both edges of every
    /// concealed frame so neither seam steps discontinuously.
    pub concealment_ramp_ms: u32,
    /// Gap width, in packets, beyond which the missing range is skipped
    /// outright instead of concealed packet by packet. Must be smaller than
    /// `max_reorder_window`, since no wider gap can ever be observed.
    pub concealment_skip_threshold_packets: u32,
    /// Reorder window tolerated by the internal `JitterBuffer`.
    pub max_reorder_window: u32,
    /// Maximum buffered duration tolerated by the internal `JitterBuffer`.
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
            rebuffer_target_ms: DEFAULT_REBUFFER_TARGET_MS,
            low_water_ms: DEFAULT_LOW_WATER_MS,
            high_water_ms: DEFAULT_HIGH_WATER_MS,
            hard_resync_threshold_ms: DEFAULT_HARD_RESYNC_THRESHOLD_MS,
            max_consecutive_concealed_packets: packets_spanning(
                DEFAULT_CONCEALMENT_BRIDGE_MS,
                packet_duration_ms,
            ),
            concealment_ramp_ms: DEFAULT_CONCEALMENT_RAMP_MS,
            concealment_skip_threshold_packets: packets_spanning(
                DEFAULT_CONCEALMENT_SKIP_THRESHOLD_MS,
                packet_duration_ms,
            ),
            max_reorder_window: packets_spanning(DEFAULT_REORDER_WINDOW_MS, packet_duration_ms),
            max_buffered_duration_ms: DEFAULT_MAX_BUFFERED_DURATION_MS,
        }
    }
}
