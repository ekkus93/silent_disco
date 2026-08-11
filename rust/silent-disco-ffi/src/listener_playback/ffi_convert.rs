//! Conversions between shared-core types and this module's `UniFFI` DTOs.

use silent_disco_core::audio::{PlaybackDiagnostics, PlaybackPhase};
use silent_disco_core::domain::SyncConfidence;

use super::error::ListenerPlaybackError;
use super::ffi_types::{
    FfiListenerPlaybackError, FfiPlaybackDiagnostics, FfiPlaybackPhase, FfiSyncConfidence,
    FfiSyncSampleOutcome,
};
use super::runtime::SyncSampleOutcome;

impl From<SyncConfidence> for FfiSyncConfidence {
    fn from(confidence: SyncConfidence) -> Self {
        match confidence {
            SyncConfidence::Unknown => Self::Unknown,
            SyncConfidence::Poor => Self::Poor,
            SyncConfidence::Fair => Self::Fair,
            SyncConfidence::Good => Self::Good,
            SyncConfidence::Excellent => Self::Excellent,
        }
    }
}

impl From<FfiSyncConfidence> for SyncConfidence {
    fn from(confidence: FfiSyncConfidence) -> Self {
        match confidence {
            FfiSyncConfidence::Unknown => Self::Unknown,
            FfiSyncConfidence::Poor => Self::Poor,
            FfiSyncConfidence::Fair => Self::Fair,
            FfiSyncConfidence::Good => Self::Good,
            FfiSyncConfidence::Excellent => Self::Excellent,
        }
    }
}

impl From<PlaybackPhase> for FfiPlaybackPhase {
    fn from(phase: PlaybackPhase) -> Self {
        match phase {
            PlaybackPhase::Buffering => Self::Buffering,
            PlaybackPhase::Playing => Self::Playing,
            PlaybackPhase::AwaitingRebuffer => Self::AwaitingRebuffer,
            PlaybackPhase::Stopped => Self::Stopped,
        }
    }
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

impl From<PlaybackDiagnostics> for FfiPlaybackDiagnostics {
    fn from(diagnostics: PlaybackDiagnostics) -> Self {
        Self {
            phase: diagnostics.phase.into(),
            sync_locked: diagnostics.sync_locked,
            packets_accepted: diagnostics.packets_accepted,
            packets_emitted: diagnostics.packets_emitted,
            sequences_skipped: diagnostics.sequences_skipped,
            late_rejections: diagnostics.late_rejections,
            duplicate_rejections: diagnostics.duplicate_rejections,
            reorder_window_rejections: diagnostics.reorder_window_rejections,
            resynchronisations: diagnostics.resynchronisations,
            dropped_before_sync: diagnostics.dropped_before_sync,
            concealed_packets: diagnostics.concealed_packets,
            concealment_driven_rebuffers: diagnostics.concealment_driven_rebuffers,
            offset_driven_rebuffers: diagnostics.offset_driven_rebuffers,
            hard_resync_signals: diagnostics.hard_resync_signals,
            buffered_span_ms: diagnostics.buffered_span_ms,
            ring_queued_frames: to_u64(diagnostics.ring_queued_frames),
            ring_peak_queued_frames: to_u64(diagnostics.ring_peak_queued_frames),
            pending_frames: to_u64(diagnostics.pending_frames),
            prefill_frames: to_u64(diagnostics.prefill_frames),
            ring_underruns: diagnostics.ring_underruns,
            ring_silence_filled_frames: diagnostics.ring_silence_filled_frames,
            ring_full_events: diagnostics.ring_full_events,
        }
    }
}

impl From<SyncSampleOutcome> for FfiSyncSampleOutcome {
    fn from(outcome: SyncSampleOutcome) -> Self {
        Self {
            accepted: outcome.accepted,
            offset_ms: outcome.offset_ms,
            skew_ppm: outcome.skew_ppm,
            round_trip_time_ms: outcome.round_trip_time_ms,
            jitter_ms: outcome.jitter_ms,
            confidence: outcome.confidence.into(),
            accepted_sample_count: to_u64(outcome.accepted_sample_count),
            sync_locked: outcome.sync_locked,
        }
    }
}

impl From<ListenerPlaybackError> for FfiListenerPlaybackError {
    fn from(error: ListenerPlaybackError) -> Self {
        match error {
            ListenerPlaybackError::InvalidConfiguration(message) => {
                Self::InvalidConfiguration(message)
            }
            ListenerPlaybackError::Stopped(message) => Self::Stopped(message),
            ListenerPlaybackError::PumpThread(message) => Self::PumpThread(message),
            ListenerPlaybackError::Sync(message) => Self::Sync(message),
            ListenerPlaybackError::DebugCapture(message) => Self::DebugCapture(message),
        }
    }
}
