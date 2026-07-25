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
pub struct SyncObservation {
    pub sample: SyncSample,
    pub accepted: bool,
    pub snapshot: SyncSnapshot,
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
            Self::InvalidConfiguration => formatter.write_str("sync estimator configuration is invalid"),
            Self::DuplicateCorrelationId { correlation_id } => {
                write!(formatter, "sync correlation ID {correlation_id} is already pending")
            }
            Self::PendingProbeLimitReached { maximum } => {
                write!(formatter, "sync pending-probe limit of {maximum} was reached")
            }
            Self::StaleCorrelationId { correlation_id } => {
                write!(formatter, "sync correlation ID {correlation_id} is stale or unknown")
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
        if self.pending.len() >= MAX_PENDING_PROBES {
            return Err(SyncEstimatorError::PendingProbeLimitReached {
                maximum: MAX_PENDING_PROBES,
            });
        }
        self.pending.insert(correlation_id, local_send_time);
        Ok(())
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
        let accepted = sample.round_trip_time_ms <= self.config.max_accepted_rtt_ms;
        if accepted {
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
        }

        Ok(SyncObservation {
            sample,
            accepted,
            snapshot: self.snapshot(),
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> SyncSnapshot {
        if self.accepted_samples.is_empty() {
            return SyncSnapshot::default();
        }

        let mut selected: Vec<SyncSample> = self.accepted_samples.iter().copied().collect();
        selected.sort_by(|left, right| {
            left.round_trip_time_ms
                .total_cmp(&right.round_trip_time_ms)
        });
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
mod tests {
    use super::{
        ClockSyncEstimator, MAX_PENDING_PROBES, SyncDecision, SyncEstimatorConfig,
        SyncEstimatorError, SyncSnapshot,
    };
    use crate::{
        domain::SyncConfidence,
        sync::{HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId},
    };

    fn observe(
        estimator: &mut ClockSyncEstimator,
        correlation: u64,
        t1: u64,
        t2: u64,
        t3: u64,
        t4: u64,
    ) -> Result<super::SyncObservation, SyncEstimatorError> {
        let correlation = SyncCorrelationId::new(correlation);
        estimator.begin_probe(correlation, LocalMonotonicMillis::new(t1))?;
        estimator.observe_response(
            correlation,
            LocalMonotonicMillis::new(t1),
            HostMonotonicMillis::new(t2),
            HostMonotonicMillis::new(t3),
            LocalMonotonicMillis::new(t4),
        )
    }

    #[test]
    fn matches_kotlin_low_rtt_best_half_behavior() {
        let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())
            .expect("default config is valid");
        assert!(observe(&mut estimator, 1, 1_000, 1_012, 1_014, 1_026)
            .expect("first sample")
            .accepted);
        assert!(observe(&mut estimator, 2, 2_000, 2_015, 2_017, 2_022)
            .expect("second sample")
            .accepted);
        assert!(!observe(&mut estimator, 3, 3_000, 3_100, 3_101, 3_400)
            .expect("high RTT sample is valid but rejected")
            .accepted);

        let snapshot = estimator.snapshot();
        assert_eq!(snapshot.offset_ms, 5.0);
        assert_eq!(snapshot.round_trip_time_ms, 20.0);
        assert_eq!(snapshot.jitter_ms, 0.0);
        assert_eq!(snapshot.confidence, SyncConfidence::Excellent);
        assert_eq!(snapshot.accepted_sample_count, 2);
    }

    #[test]
    fn correlation_ids_are_bounded_unique_and_single_use() {
        let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())
            .expect("default config is valid");
        let correlation = SyncCorrelationId::new(7);
        estimator
            .begin_probe(correlation, LocalMonotonicMillis::new(100))
            .expect("first registration succeeds");
        assert_eq!(
            estimator.begin_probe(correlation, LocalMonotonicMillis::new(100)),
            Err(SyncEstimatorError::DuplicateCorrelationId { correlation_id: correlation })
        );
        estimator
            .observe_response(
                correlation,
                LocalMonotonicMillis::new(100),
                HostMonotonicMillis::new(110),
                HostMonotonicMillis::new(111),
                LocalMonotonicMillis::new(120),
            )
            .expect("first response consumes correlation");
        assert_eq!(
            estimator.observe_response(
                correlation,
                LocalMonotonicMillis::new(100),
                HostMonotonicMillis::new(110),
                HostMonotonicMillis::new(111),
                LocalMonotonicMillis::new(120),
            ),
            Err(SyncEstimatorError::StaleCorrelationId { correlation_id: correlation })
        );

        for id in 0..MAX_PENDING_PROBES {
            estimator
                .begin_probe(
                    SyncCorrelationId::new(1_000 + id as u64),
                    LocalMonotonicMillis::new(1_000),
                )
                .expect("pending capacity not reached");
        }
        assert!(matches!(
            estimator.begin_probe(
                SyncCorrelationId::new(9_999),
                LocalMonotonicMillis::new(1_000),
            ),
            Err(SyncEstimatorError::PendingProbeLimitReached { .. })
        ));
    }

    #[test]
    fn mismatched_echo_consumes_correlation_and_fails_visibly() {
        let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())
            .expect("default config is valid");
        let correlation = SyncCorrelationId::new(8);
        estimator
            .begin_probe(correlation, LocalMonotonicMillis::new(100))
            .expect("probe registration succeeds");
        assert_eq!(
            estimator.observe_response(
                correlation,
                LocalMonotonicMillis::new(101),
                HostMonotonicMillis::new(110),
                HostMonotonicMillis::new(111),
                LocalMonotonicMillis::new(120),
            ),
            Err(SyncEstimatorError::CorrelationTimestampMismatch { correlation_id: correlation })
        );
        assert_eq!(estimator.pending_probe_count(), 0);
    }

    #[test]
    fn sample_and_drift_history_are_bounded() {
        let config = SyncEstimatorConfig {
            max_samples: 3,
            drift_history_size: 3,
            ..SyncEstimatorConfig::default()
        };
        let mut estimator = ClockSyncEstimator::new(config).expect("bounded config is valid");
        for index in 0..5_u64 {
            observe(
                &mut estimator,
                index,
                index * 1_000,
                index * 1_000 + 10 + index,
                index * 1_000 + 11 + index,
                index * 1_000 + 20,
            )
            .expect("ordered sample");
        }
        assert_eq!(estimator.snapshot().accepted_sample_count, 3);
        assert!(estimator.snapshot().skew_ppm.is_finite());
    }

    #[test]
    fn confidence_thresholds_match_android_baseline() {
        assert_eq!(super::classify_confidence(20.0, 2.0), SyncConfidence::Excellent);
        assert_eq!(super::classify_confidence(50.0, 5.0), SyncConfidence::Good);
        assert_eq!(super::classify_confidence(90.0, 12.0), SyncConfidence::Fair);
        assert_eq!(super::classify_confidence(90.1, 0.0), SyncConfidence::Poor);
    }

    #[test]
    fn decisions_distinguish_initial_periodic_drift_and_wait() {
        let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default())
            .expect("default config is valid");
        assert_eq!(
            estimator.decision(LocalMonotonicMillis::new(0), SyncSnapshot::default()),
            Ok(SyncDecision::InitialProbeRequired)
        );
        let observation = observe(&mut estimator, 1, 1_000, 1_010, 1_011, 1_020)
            .expect("accepted sample");
        assert_eq!(
            estimator.decision(LocalMonotonicMillis::new(2_000), observation.snapshot),
            Ok(SyncDecision::Wait)
        );
        assert_eq!(
            estimator.decision(LocalMonotonicMillis::new(3_020), observation.snapshot),
            Ok(SyncDecision::PeriodicProbeRequired)
        );
        let drifted = SyncSnapshot {
            offset_ms: 18.1,
            ..observation.snapshot
        };
        assert_eq!(
            estimator.decision(LocalMonotonicMillis::new(2_000), drifted),
            Ok(SyncDecision::DriftProbeRequired)
        );
        assert_eq!(
            estimator.decision(LocalMonotonicMillis::new(999), observation.snapshot),
            Err(SyncEstimatorError::LocalClockMovedBackwards)
        );
    }

    #[test]
    fn invalid_configuration_is_rejected_before_allocating() {
        assert_eq!(
            ClockSyncEstimator::new(SyncEstimatorConfig {
                max_samples: 0,
                ..SyncEstimatorConfig::default()
            }),
            Err(SyncEstimatorError::InvalidConfiguration)
        );
        assert_eq!(
            ClockSyncEstimator::new(SyncEstimatorConfig {
                max_accepted_rtt_ms: f64::NAN,
                ..SyncEstimatorConfig::default()
            }),
            Err(SyncEstimatorError::InvalidConfiguration)
        );
    }
}
