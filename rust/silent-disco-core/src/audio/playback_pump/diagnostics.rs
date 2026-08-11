//! The point-in-time diagnostics snapshot: everything needed to tell, after
//! the fact, where audio went missing or wrong.

use crate::audio::PlaybackPhase;

use super::pump::PlaybackPump;

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
    pub concealment_driven_rebuffers: u64,
    /// Times a clock-offset jump too large to splice forced a rebuffer.
    pub offset_driven_rebuffers: u64,
    /// Every hard resync regardless of cause -- `concealment_driven_rebuffers`
    /// `+ offset_driven_rebuffers`. Kept as the total under its original name
    /// so every existing reader (Kotlin diagnostics logging, prior device
    /// measurements recorded in `memory.md`) keeps meaning "how many times
    /// did playback hard-rebuffer", not just one of the two causes (A4.4).
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

impl PlaybackPump {
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
            concealment_driven_rebuffers: concealment.hard_resync_signals,
            offset_driven_rebuffers: self.offset_driven_rebuffers,
            hard_resync_signals: concealment
                .hard_resync_signals
                .saturating_add(self.offset_driven_rebuffers),
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
}
