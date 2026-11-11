//! The clock-offset gate: nothing may play against a placeholder offset, so
//! this is what decides when playback is allowed to start at all, and how a
//! later correction is applied once it has.

use crate::audio::{JitterBufferRejection, OffsetUpdateOutcome};
use crate::protocol::AudioDatagram;

use super::pump::PlaybackPump;

/// Largest in-place clock-offset correction applied from one accepted sync
/// update. At the normal 250ms probe cadence this permits 20ms/s of recovery
/// while keeping every audible adjustment far below the hard-resync boundary.
const MAX_SOFT_OFFSET_STEP_MS: f64 = 5.0;

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
    /// The supplied estimate was NaN or infinite and was ignored without
    /// mutating playback state.
    RejectedNonFinite,
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
        if !offset_ms.is_finite() {
            return SyncApplyOutcome::RejectedNonFinite;
        }
        if !self.sync_locked {
            self.offset_ms = offset_ms;
            self.sync_locked = true;
            self.scheduler.rebuffer(offset_ms);
            self.awaiting_prefill = true;
            self.replay_pre_sync_hold();
            return SyncApplyOutcome::Locked;
        }

        let delta_ms = offset_ms - self.offset_ms;
        if delta_ms.abs() > self.scheduler.hard_resync_threshold_ms() {
            self.offset_ms = offset_ms;
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
        } else {
            // Small corrections are intentionally slewed instead of moving the
            // presentation timeline by the whole estimator delta in one tick.
            // Repeating accepted sync observations converges to the target, but
            // each individual splice is bounded to an inaudible-sized step.
            let applied_delta_ms =
                delta_ms.clamp(-MAX_SOFT_OFFSET_STEP_MS, MAX_SOFT_OFFSET_STEP_MS);
            self.offset_ms += applied_delta_ms;
            let outcome = self.scheduler.apply_offset_update(self.offset_ms);
            debug_assert!(matches!(outcome, OffsetUpdateOutcome::SoftCorrected));
            SyncApplyOutcome::SoftCorrected
        }
    }

    /// True once a real clock-offset estimate has been applied.
    #[must_use]
    pub const fn is_sync_locked(&self) -> bool {
        self.sync_locked
    }

    /// Submits one arriving packet, or holds it until playback can use it.
    ///
    /// Before a clock offset has been accepted there is no timeline to place
    /// a packet on, so it cannot enter the jitter buffer: there it would pile
    /// up against sequence zero and overflow the reorder window during a long
    /// acquisition. But dropping it lost real audio: a host bursts up to a
    /// second of audio at stream start, which on a device all arrived in the
    /// few milliseconds before the first sync sample was accepted, so every
    /// stream's opening second was thrown away.
    ///
    /// Instead, pre-sync packets wait in a hold bounded by the reorder window,
    /// keeping the newest when it is full, and are replayed into the
    /// scheduler the moment sync locks. A long acquisition still ends with
    /// the buffer adopting the live position; a short one loses nothing.
    ///
    /// # Errors
    ///
    /// Returns a [`JitterBufferRejection`] under the same conditions as
    /// [`JitterBuffer::accept`](crate::audio::JitterBuffer::accept). A packet
    /// held or dropped for arriving before sync is not an error.
    pub fn submit_packet(&mut self, datagram: AudioDatagram) -> Result<(), JitterBufferRejection> {
        if !self.sync_locked {
            let limit = usize::try_from(self.scheduler.max_reorder_window())
                .unwrap_or(usize::MAX)
                .max(1);
            self.pre_sync_hold.push_back(datagram);
            while self.pre_sync_hold.len() > limit {
                self.pre_sync_hold.pop_front();
                self.dropped_before_sync = self.dropped_before_sync.saturating_add(1);
            }
            return Ok(());
        }
        self.scheduler.submit_packet(datagram)
    }

    /// Feeds the packets held before sync locked into the scheduler, in
    /// arrival order. Rejections are counted by the jitter buffer as usual.
    fn replay_pre_sync_hold(&mut self) {
        while let Some(datagram) = self.pre_sync_hold.pop_front() {
            self.replayed_after_sync = self.replayed_after_sync.saturating_add(1);
            let _ = self.scheduler.submit_packet(datagram);
        }
    }

    /// Packets discarded because they arrived before playback had a timeline.
    #[must_use]
    pub const fn dropped_before_sync(&self) -> u64 {
        self.dropped_before_sync
    }
}
