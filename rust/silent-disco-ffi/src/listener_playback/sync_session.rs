//! A listener's clock lock before its session's first stream.
//!
//! A playback runtime can only start already locked when it inherits a
//! [`ListenerSyncHandover`]; before 2026-11-19 the only source was the
//! previous stream's runtime, so a session's first stream had to acquire sync
//! after its `StreamStart` arrived. Audio whose presentation time passed
//! meanwhile was skipped: on the A54 the first song lost its first ~0.57 s.
//!
//! This holds the same clock and estimator a runtime would, from the moment
//! the listener is admitted. The platform probes through it exactly as it
//! probes a running stream, and the first stream opens from its handover, so
//! the first packet is scheduled against a real timeline.

use std::sync::{Arc, Mutex, PoisonError};

use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};

use super::error::ListenerPlaybackError;
use super::pump::PumpClock;
use super::runtime::{ListenerSyncHandover, SyncSampleOutcome, fresh_locked_offset};

#[derive(Debug)]
pub struct ListenerSyncSession {
    clock: Arc<PumpClock>,
    estimator: Mutex<ClockSyncEstimator>,
}

impl ListenerSyncSession {
    /// A new clock and an empty estimate.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::InvalidConfiguration`] if the default
    /// estimator configuration is rejected.
    pub fn new() -> Result<Self, ListenerPlaybackError> {
        let estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())
            .map_err(|error| ListenerPlaybackError::InvalidConfiguration(format!("{error:?}")))?;
        Ok(Self {
            clock: Arc::new(PumpClock::new()),
            estimator: Mutex::new(estimator),
        })
    }

    fn lock_estimator(&self) -> std::sync::MutexGuard<'_, ClockSyncEstimator> {
        // The estimator holds plain values; a panic mid-update cannot leave
        // it in a state worse than its last complete sample.
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Monotonic milliseconds on the timeline the first stream will play
    /// against. Every local sync timestamp must come from here.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Registers one outbound sync probe before it is sent.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::Sync`] for a duplicate correlation ID
    /// or when too many probes are already outstanding.
    pub fn begin_sync_probe(
        &self,
        correlation_id: u64,
        local_send_time_ms: u64,
    ) -> Result<(), ListenerPlaybackError> {
        self.lock_estimator()
            .begin_probe(
                SyncCorrelationId::new(correlation_id),
                LocalMonotonicMillis::new(local_send_time_ms),
            )
            .map_err(|error| ListenerPlaybackError::Sync(format!("{error:?}")))
    }

    /// Feeds one correlated four-timestamp response to the estimator.
    /// `sync_locked` reports whether a stream opened now would start locked.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::Sync`] when the response does not
    /// correlate to a registered probe or its timestamps are impossible.
    pub fn observe_sync_response(
        &self,
        correlation_id: u64,
        echoed_local_send_time_ms: u64,
        host_receive_time_ms: u64,
        host_send_time_ms: u64,
        local_receive_time_ms: u64,
    ) -> Result<SyncSampleOutcome, ListenerPlaybackError> {
        let mut estimator = self.lock_estimator();
        let observation = estimator
            .observe_response(
                SyncCorrelationId::new(correlation_id),
                LocalMonotonicMillis::new(echoed_local_send_time_ms),
                HostMonotonicMillis::new(host_receive_time_ms),
                HostMonotonicMillis::new(host_send_time_ms),
                LocalMonotonicMillis::new(local_receive_time_ms),
            )
            .map_err(|error| ListenerPlaybackError::Sync(format!("{error:?}")))?;
        let snapshot = observation.snapshot;
        Ok(SyncSampleOutcome {
            accepted: observation.accepted,
            offset_ms: snapshot.offset_ms,
            skew_ppm: snapshot.skew_ppm,
            round_trip_time_ms: snapshot.round_trip_time_ms,
            jitter_ms: snapshot.jitter_ms,
            confidence: snapshot.confidence,
            accepted_sample_count: snapshot.accepted_sample_count,
            sync_locked: fresh_locked_offset(&estimator, self.clock.now_ms()).is_some(),
            acquisition_rejected_sample_count: observation.acquisition.rejected_sample_count,
            acquisition_elapsed_ms: observation.acquisition.elapsed_ms,
            acquisition_rtt_limit_ms: observation.acquisition.effective_rtt_limit_ms,
            degraded_lock: observation.acquisition.degraded_lock,
        })
    }

    /// Whether a stream opened now from [`Self::sync_handover`] would start
    /// already locked.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        fresh_locked_offset(&self.lock_estimator(), self.clock.now_ms()).is_some()
    }

    /// This session's clock and estimate, for the first stream's runtime.
    #[must_use]
    pub fn sync_handover(&self) -> ListenerSyncHandover {
        ListenerSyncHandover::new(Arc::clone(&self.clock), self.lock_estimator().clone())
    }
}
