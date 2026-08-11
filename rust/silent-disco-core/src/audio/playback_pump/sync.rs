//! The clock-offset gate: nothing may play against a placeholder offset, so
//! this is what decides when playback is allowed to start at all, and how a
//! later correction is applied once it has.

use crate::audio::{JitterBufferRejection, OffsetUpdateOutcome};
use crate::protocol::AudioDatagram;

use super::pump::PlaybackPump;

/// Result of applying a fresh clock-offset estimate to a running pump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncApplyOutcome {
    /// The first real estimate; playback is now free to start.
    Locked,
    /// The estimate moved within tolerance and was applied in place.
    SoftCorrected,
    /// The estimate moved too far to splice; playback re-accumulates its
    /// startup buffer before resuming.
    Rebuffered,
}

impl PlaybackPump {
    /// Applies a clock-offset estimate produced from a genuinely accepted sync
    /// sample, unlocking playback on the first one.
    ///
    /// The first estimate is adopted outright rather than treated as a
    /// correction: host and listener clocks have unrelated epochs, so a real
    /// first offset is enormous next to the placeholder it replaces and any
    /// threshold comparison against that placeholder is meaningless. Later
    /// estimates are corrections, and one too large to splice re-accumulates
    /// the startup buffer instead of jumping the timeline mid-stream.
    pub fn apply_sync_offset(&mut self, offset_ms: f64) -> SyncApplyOutcome {
        self.offset_ms = offset_ms;
        if !self.sync_locked {
            self.sync_locked = true;
            self.scheduler.rebuffer(offset_ms);
            self.awaiting_prefill = true;
            return SyncApplyOutcome::Locked;
        }
        match self.scheduler.apply_offset_update(offset_ms) {
            OffsetUpdateOutcome::SoftCorrected => SyncApplyOutcome::SoftCorrected,
            OffsetUpdateOutcome::HardResyncRequired => {
                self.scheduler.rebuffer(offset_ms);
                // Playback restarts from an empty ring, so its position must
                // be re-aligned to the timeline rather than inheriting
                // whatever depth happened to remain.
                self.awaiting_prefill = true;
                self.offset_driven_rebuffers = self.offset_driven_rebuffers.saturating_add(1);
                SyncApplyOutcome::Rebuffered
            }
        }
    }

    /// True once a real clock-offset estimate has been applied.
    #[must_use]
    pub const fn is_sync_locked(&self) -> bool {
        self.sync_locked
    }

    /// Submits one arriving packet, or drops it if playback cannot yet use it.
    ///
    /// Before a clock offset has been accepted there is no timeline to place
    /// a packet on. Buffering them anyway is worse than dropping them: they
    /// accumulate against sequence zero, overflow the jitter buffer's reorder
    /// window within about a second, and are then rejected for the rest of
    /// the acquisition — losing far more audio than the wait itself, and
    /// leaving the buffer holding content that is stale by the time it could
    /// be played. Dropping them lets the buffer adopt the live position once
    /// sync lands.
    ///
    /// # Errors
    ///
    /// Returns a [`JitterBufferRejection`] under the same conditions as
    /// [`JitterBuffer::accept`](crate::audio::JitterBuffer::accept). A packet
    /// dropped for arriving before sync is not an error.
    pub fn submit_packet(&mut self, datagram: AudioDatagram) -> Result<(), JitterBufferRejection> {
        if !self.sync_locked {
            self.dropped_before_sync = self.dropped_before_sync.saturating_add(1);
            return Ok(());
        }
        self.scheduler.submit_packet(datagram)
    }

    /// Packets discarded because they arrived before playback had a timeline.
    #[must_use]
    pub const fn dropped_before_sync(&self) -> u64 {
        self.dropped_before_sync
    }
}
