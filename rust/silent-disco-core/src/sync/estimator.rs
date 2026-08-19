use core::fmt;
use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
};

use crate::domain::SyncConfidence;

use super::types::{
    HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId, SyncExchange, SyncSample,
    SyncTimestampError,
};

pub const MAX_PENDING_PROBES: usize = 64;
pub const MAX_ESTIMATOR_SAMPLES: usize = 128;
pub const MAX_DRIFT_HISTORY_SAMPLES: usize = 256;
/// Longest a probe may sit unanswered before `begin_probe` evicts it.
///
/// `pending` used to shrink only in `observe_response`, so response loss
/// was unbounded: enough lost responses filled it to `MAX_PENDING_PROBES`
/// and every later `begin_probe` failed permanently for the rest of the
/// stream -- the caller's probe loop (Kotlin) treats a failed `begin_probe`
/// as "do not send this probe either", so probing itself stopped, not just
/// accounting for it. A stall that dropped enough responses in a row would
/// have turned into permanent silence rather than eventual recovery. Five
/// seconds is comfortably longer than any real round trip this estimator
/// would ever accept (`max_accepted_rtt_ms` defaults to 200ms) or has been
/// observed taking even on a badly congested real device this session, so
/// eviction only ever discards probes whose response is genuinely never
/// coming, not a slow-but-real one.
pub const PENDING_PROBE_MAX_AGE_MS: u64 = 5_000;

/// Acquisition stays strict until enough measured rejections show the default
/// gate is unsuitable for the current path.
pub const ACQUISITION_ADAPT_AFTER_REJECTIONS: u64 = 3;
/// Earliest elapsed acquisition time at which measured RTTs may widen the gate.
pub const ACQUISITION_ADAPT_AFTER_MS: u64 = 750;
/// Ceiling for the measured adaptive gate before the hard acquisition bound.
pub const ACQUISITION_ADAPTIVE_CEILING_MS: f64 = 600.0;
/// Time after which acquisition may use the hard bounded ceiling.
pub const ACQUISITION_HARD_CEILING_AFTER_MS: u64 = 2_000;
/// Absolute default acquisition ceiling; steady-state samples never use it.
pub const ACQUISITION_HARD_CEILING_MS: f64 = 1_000.0;
const ACQUISITION_REJECTED_RTT_HISTORY: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncEstimatorConfig {
    pub max_samples: usize,
    pub max_accepted_rtt_ms: f64,
    pub cadence_ms: u64,
    pub drift_threshold_ms: f64,
    pub drift_history_size: usize,
}

impl Default for SyncEstimatorConfig {
    fn default() -> Self {
        Self {
            max_samples: 12,
            max_accepted_rtt_ms: 200.0,
            cadence_ms: 2_000,
            drift_threshold_ms: 18.0,
            drift_history_size: 24,
        }
    }
}

impl SyncEstimatorConfig {
    /// Validates all estimator limits before allocating history buffers.
    ///
    /// # Errors
    ///
    /// Returns [`SyncEstimatorError::InvalidConfiguration`] for zero, excessive,
    /// non-finite, or negative limits.
    pub fn validate(self) -> Result<Self, SyncEstimatorError> {
        let valid = (1..=MAX_ESTIMATOR_SAMPLES).contains(&self.max_samples)
            && self.max_accepted_rtt_ms.is_finite()
            && self.max_accepted_rtt_ms >= 0.0
            && self.cadence_ms > 0
            && self.drift_threshold_ms.is_finite()
            && self.drift_threshold_ms >= 0.0
            && (1..=MAX_DRIFT_HISTORY_SAMPLES).contains(&self.drift_history_size);
        if valid {
            Ok(self)
        } else {
            Err(SyncEstimatorError::InvalidConfiguration)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncSnapshot {
    pub offset_ms: f64,
    pub round_trip_time_ms: f64,
    pub jitter_ms: f64,
    pub confidence: SyncConfidence,
    pub skew_ppm: f64,
    pub accepted_sample_count: usize,
}

impl Default for SyncSnapshot {
    fn default() -> Self {
        Self {
            offset_ms: 0.0,
            round_trip_time_ms: 0.0,
            jitter_ms: 0.0,
            confidence: SyncConfidence::Unknown,
            skew_ppm: 0.0,
            accepted_sample_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncAcquisitionStatus {
    /// Samples rejected before the first accepted synchronization sample.
    pub rejected_sample_count: u64,
    /// Milliseconds elapsed since the first probe of the current acquisition.
    pub elapsed_ms: u64,
    /// RTT gate used for the current observation.
    pub effective_rtt_limit_ms: f64,
    /// True when initial lock required the acquisition-only widened gate.
    pub degraded_lock: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncObservation {
    pub sample: SyncSample,
    pub accepted: bool,
    pub snapshot: SyncSnapshot,
    pub acquisition: SyncAcquisitionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDecision {
    Wait,
    InitialProbeRequired,
    PeriodicProbeRequired,
    DriftProbeRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEstimatorError {
    InvalidConfiguration,
    DuplicateCorrelationId { correlation_id: SyncCorrelationId },
    PendingProbeLimitReached { maximum: usize },
    StaleCorrelationId { correlation_id: SyncCorrelationId },
    CorrelationTimestampMismatch { correlation_id: SyncCorrelationId },
    Timestamp(SyncTimestampError),
    LocalClockMovedBackwards,
}

impl fmt::Display for SyncEstimatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => {
                formatter.write_str("sync estimator configuration is invalid")
            }
            Self::DuplicateCorrelationId { correlation_id } => {
                write!(
                    formatter,
                    "sync correlation ID {correlation_id} is already pending"
                )
            }
            Self::PendingProbeLimitReached { maximum } => {
                write!(
                    formatter,
                    "sync pending-probe limit of {maximum} was reached"
                )
            }
            Self::StaleCorrelationId { correlation_id } => {
                write!(
                    formatter,
                    "sync correlation ID {correlation_id} is stale or unknown"
                )
            }
            Self::CorrelationTimestampMismatch { correlation_id } => write!(
                formatter,
                "sync response timestamp does not match correlation ID {correlation_id}"
            ),
            Self::Timestamp(error) => error.fmt(formatter),
            Self::LocalClockMovedBackwards => {
                formatter.write_str("local monotonic clock moved backwards")
            }
        }
    }
}

impl Error for SyncEstimatorError {}

impl From<SyncTimestampError> for SyncEstimatorError {
    fn from(error: SyncTimestampError) -> Self {
        Self::Timestamp(error)
    }
}

#[derive(Debug)]
pub struct ClockSyncEstimator {
    config: SyncEstimatorConfig,
    pending: BTreeMap<SyncCorrelationId, LocalMonotonicMillis>,
    accepted_samples: VecDeque<SyncSample>,
    drift_history: VecDeque<(LocalMonotonicMillis, f64)>,
    last_accepted_sync_at: Option<LocalMonotonicMillis>,
    acquisition_started_at: Option<LocalMonotonicMillis>,
    rejected_acquisition_rtts: VecDeque<f64>,
    acquisition_rejected_count: u64,
    degraded_lock: bool,
}

impl ClockSyncEstimator {
    /// Creates a bounded estimator after validating configuration.
    ///
    /// # Errors
    ///
    /// Returns [`SyncEstimatorError::InvalidConfiguration`] when any configured
    /// sample count, duration, RTT threshold, or drift threshold is invalid.
    pub fn new(config: SyncEstimatorConfig) -> Result<Self, SyncEstimatorError> {
        let config = config.validate()?;
        Ok(Self {
            pending: BTreeMap::new(),
            accepted_samples: VecDeque::with_capacity(config.max_samples),
            drift_history: VecDeque::with_capacity(config.drift_history_size),
            last_accepted_sync_at: None,
            acquisition_started_at: None,
            rejected_acquisition_rtts: VecDeque::with_capacity(ACQUISITION_REJECTED_RTT_HISTORY),
            acquisition_rejected_count: 0,
            degraded_lock: false,
            config,
        })
    }

    #[must_use]
    pub const fn config(&self) -> SyncEstimatorConfig {
        self.config
    }

    #[must_use]
    pub fn pending_probe_count(&self) -> usize {
        self.pending.len()
    }

    /// Registers one outbound probe before transport delivery.
    ///
    /// # Errors
    ///
    /// Returns a duplicate-ID error or a visible bounded-capacity error. No
    /// existing pending probe is overwritten.
    pub fn begin_probe(
        &mut self,
        correlation_id: SyncCorrelationId,
        local_send_time: LocalMonotonicMillis,
    ) -> Result<(), SyncEstimatorError> {
        if self.pending.contains_key(&correlation_id) {
            return Err(SyncEstimatorError::DuplicateCorrelationId { correlation_id });
        }
        // `local_send_time` is the caller's own fresh "now" for this probe,
        // so it doubles as the current time for evicting older ones -- no
        // separate clock reference needed here.
        self.evict_stale_pending_probes(local_send_time);
        if self.pending.len() >= MAX_PENDING_PROBES {
            return Err(SyncEstimatorError::PendingProbeLimitReached {
                maximum: MAX_PENDING_PROBES,
            });
        }
        if self.accepted_samples.is_empty() && self.acquisition_started_at.is_none() {
            self.acquisition_started_at = Some(local_send_time);
        }
        self.pending.insert(correlation_id, local_send_time);
        Ok(())
    }

    /// Drops pending probes older than [`PENDING_PROBE_MAX_AGE_MS`]. Runs on
    /// every `begin_probe`, so a stall recovers by itself on the very next
    /// probe attempt rather than needing an explicit, separately-scheduled
    /// sweep.
    fn evict_stale_pending_probes(&mut self, now: LocalMonotonicMillis) {
        self.pending.retain(|_, &mut sent_at| {
            now.get().saturating_sub(sent_at.get()) < PENDING_PROBE_MAX_AGE_MS
        });
    }

    /// Consumes one correlated response and updates the estimate when its RTT is
    /// within the configured acceptance window.
    ///
    /// # Errors
    ///
    /// Returns a stale-correlation, timestamp-mismatch, or checked timestamp
    /// ordering error. A correlation ID is consumed at most once.
    pub fn observe_response(
        &mut self,
        correlation_id: SyncCorrelationId,
        echoed_local_send_time: LocalMonotonicMillis,
        host_receive_time: HostMonotonicMillis,
        host_send_time: HostMonotonicMillis,
        local_receive_time: LocalMonotonicMillis,
    ) -> Result<SyncObservation, SyncEstimatorError> {
        let registered_send_time = self
            .pending
            .remove(&correlation_id)
            .ok_or(SyncEstimatorError::StaleCorrelationId { correlation_id })?;
        if registered_send_time != echoed_local_send_time {
            return Err(SyncEstimatorError::CorrelationTimestampMismatch { correlation_id });
        }

        let sample = SyncSample::from_exchange(SyncExchange {
            correlation_id,
            t1_local_send: registered_send_time,
            t2_host_receive: host_receive_time,
            t3_host_send: host_send_time,
            t4_local_receive: local_receive_time,
        })?;
        let effective_rtt_limit_ms = self.effective_rtt_limit_ms(local_receive_time);
        let accepted = sample.round_trip_time_ms <= effective_rtt_limit_ms;
        if accepted {
            if self.accepted_samples.is_empty()
                && sample.round_trip_time_ms > self.config.max_accepted_rtt_ms
            {
                self.degraded_lock = true;
            }
            if self.accepted_samples.len() == self.config.max_samples {
                self.accepted_samples.pop_front();
            }
            self.accepted_samples.push_back(sample);

            if self.drift_history.len() == self.config.drift_history_size {
                self.drift_history.pop_front();
            }
            self.drift_history
                .push_back((sample.local_receive_time, sample.offset_ms));
            self.last_accepted_sync_at = Some(sample.local_receive_time);
        } else if self.accepted_samples.is_empty() {
            self.acquisition_rejected_count = self.acquisition_rejected_count.saturating_add(1);
            if self.rejected_acquisition_rtts.len() == ACQUISITION_REJECTED_RTT_HISTORY {
                self.rejected_acquisition_rtts.pop_front();
            }
            self.rejected_acquisition_rtts
                .push_back(sample.round_trip_time_ms);
        }

        Ok(SyncObservation {
            sample,
            accepted,
            snapshot: self.snapshot(),
            acquisition: SyncAcquisitionStatus {
                rejected_sample_count: self.acquisition_rejected_count,
                elapsed_ms: self.acquisition_elapsed_ms(local_receive_time),
                effective_rtt_limit_ms,
                degraded_lock: self.degraded_lock,
            },
        })
    }

    fn acquisition_elapsed_ms(&self, now: LocalMonotonicMillis) -> u64 {
        self.acquisition_started_at
            .map_or(0, |started| now.get().saturating_sub(started.get()))
    }

    fn effective_rtt_limit_ms(&self, now: LocalMonotonicMillis) -> f64 {
        // Once a real sample has locked the timeline, steady-state quality is
        // strict again. The widened gate is acquisition-only.
        if !self.accepted_samples.is_empty() {
            return self.config.max_accepted_rtt_ms;
        }
        let elapsed_ms = self.acquisition_elapsed_ms(now);
        if elapsed_ms >= ACQUISITION_HARD_CEILING_AFTER_MS {
            return self
                .config
                .max_accepted_rtt_ms
                .max(ACQUISITION_HARD_CEILING_MS);
        }
        if elapsed_ms < ACQUISITION_ADAPT_AFTER_MS
            || self.acquisition_rejected_count < ACQUISITION_ADAPT_AFTER_REJECTIONS
            || self.rejected_acquisition_rtts.is_empty()
        {
            return self.config.max_accepted_rtt_ms;
        }

        let mut measured: Vec<f64> = self.rejected_acquisition_rtts.iter().copied().collect();
        measured.sort_by(f64::total_cmp);
        let median = measured[measured.len() / 2];
        (median * 1.5)
            .max(self.config.max_accepted_rtt_ms)
            .min(ACQUISITION_ADAPTIVE_CEILING_MS.max(self.config.max_accepted_rtt_ms))
    }

    #[must_use]
    pub fn acquisition_status(&self, now: LocalMonotonicMillis) -> SyncAcquisitionStatus {
        SyncAcquisitionStatus {
            rejected_sample_count: self.acquisition_rejected_count,
            elapsed_ms: self.acquisition_elapsed_ms(now),
            effective_rtt_limit_ms: self.effective_rtt_limit_ms(now),
            degraded_lock: self.degraded_lock,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> SyncSnapshot {
        if self.accepted_samples.is_empty() {
            return SyncSnapshot::default();
        }

        let mut selected: Vec<SyncSample> = self.accepted_samples.iter().copied().collect();
        selected
            .sort_by(|left, right| left.round_trip_time_ms.total_cmp(&right.round_trip_time_ms));
        selected.truncate((selected.len() / 2).max(1));

        let sample_count = selected.len();
        let divisor = usize_to_f64(sample_count);
        let offset_ms = selected.iter().map(|sample| sample.offset_ms).sum::<f64>() / divisor;
        let round_trip_time_ms = selected
            .iter()
            .map(|sample| sample.round_trip_time_ms)
            .sum::<f64>()
            / divisor;
        let jitter_ms = if sample_count == 1 {
            0.0
        } else {
            selected
                .iter()
                .map(|sample| (sample.offset_ms - offset_ms).abs())
                .sum::<f64>()
                / divisor
        };

        SyncSnapshot {
            offset_ms,
            round_trip_time_ms,
            jitter_ms,
            confidence: classify_confidence(round_trip_time_ms, jitter_ms),
            skew_ppm: estimate_skew_ppm(&self.drift_history),
            accepted_sample_count: self.accepted_samples.len(),
        }
    }

    /// Determines whether initial, periodic, or drift-driven synchronization is
    /// required using only local monotonic time.
    ///
    /// # Errors
    ///
    /// Returns [`SyncEstimatorError::LocalClockMovedBackwards`] when the supplied
    /// local monotonic time precedes the most recent accepted synchronization.
    pub fn decision(
        &self,
        now: LocalMonotonicMillis,
        snapshot: SyncSnapshot,
    ) -> Result<SyncDecision, SyncEstimatorError> {
        let Some(last_sync) = self.last_accepted_sync_at else {
            return Ok(SyncDecision::InitialProbeRequired);
        };
        let elapsed = now
            .checked_elapsed_since(last_sync)
            .ok_or(SyncEstimatorError::LocalClockMovedBackwards)?;
        if snapshot.offset_ms.abs() > self.config.drift_threshold_ms {
            Ok(SyncDecision::DriftProbeRequired)
        } else if elapsed >= self.config.cadence_ms {
            Ok(SyncDecision::PeriodicProbeRequired)
        } else {
            Ok(SyncDecision::Wait)
        }
    }
}

#[must_use]
pub const fn classify_confidence(round_trip_time_ms: f64, jitter_ms: f64) -> SyncConfidence {
    if round_trip_time_ms <= 20.0 && jitter_ms <= 2.0 {
        SyncConfidence::Excellent
    } else if round_trip_time_ms <= 50.0 && jitter_ms <= 5.0 {
        SyncConfidence::Good
    } else if round_trip_time_ms <= 90.0 && jitter_ms <= 12.0 {
        SyncConfidence::Fair
    } else {
        SyncConfidence::Poor
    }
}

fn estimate_skew_ppm(history: &VecDeque<(LocalMonotonicMillis, f64)>) -> f64 {
    if history.len() < 3 {
        return 0.0;
    }
    let Some((origin, _)) = history.front().copied() else {
        return 0.0;
    };
    let points: Vec<(f64, f64)> = history
        .iter()
        .map(|(time, offset)| {
            let delta = time.get().saturating_sub(origin.get());
            (u64_to_f64(delta), *offset)
        })
        .collect();
    let divisor = usize_to_f64(points.len());
    let x_mean = points.iter().map(|(x, _)| x).sum::<f64>() / divisor;
    let y_mean = points.iter().map(|(_, y)| y).sum::<f64>() / divisor;
    let numerator = points
        .iter()
        .map(|(x, y)| (x - x_mean) * (y - y_mean))
        .sum::<f64>();
    let denominator = points
        .iter()
        .map(|(x, _)| (x - x_mean) * (x - x_mean))
        .sum::<f64>();
    if denominator == 0.0 {
        0.0
    } else {
        (numerator / denominator) * 1_000_000.0
    }
}

#[allow(clippy::cast_precision_loss)]
fn u64_to_f64(value: u64) -> f64 {
    value as f64
}

#[allow(clippy::cast_precision_loss)]
fn usize_to_f64(value: usize) -> f64 {
    value as f64
}

#[cfg(test)]
mod tests;
