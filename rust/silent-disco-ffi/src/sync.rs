use core::fmt;
use std::error::Error;

use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig, SyncEstimatorError, SyncSnapshot, SyncTimestampError,
};

/// Binding-facing synchronization configuration with fixed-width fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiSyncEstimatorConfig {
    pub max_samples: u32,
    pub max_accepted_rtt_ms: f64,
}

impl Default for FfiSyncEstimatorConfig {
    fn default() -> Self {
        Self {
            max_samples: 12,
            max_accepted_rtt_ms: 200.0,
        }
    }
}

/// One four-timestamp synchronization exchange supplied by a platform adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfiSyncExchange {
    pub t1_local_send_ms: u64,
    pub t2_host_receive_ms: u64,
    pub t3_host_send_ms: u64,
    pub t4_local_receive_ms: u64,
}

/// Binding-facing synchronization snapshot produced entirely by Rust.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiSyncSnapshot {
    pub offset_ms: f64,
    pub round_trip_time_ms: f64,
    pub jitter_ms: f64,
    pub skew_ppm: f64,
    pub confidence_code: u16,
    pub accepted_sample_count: u32,
}

/// Result of one correlated synchronization observation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FfiSyncObservation {
    pub round_trip_time_ms: f64,
    pub offset_ms: f64,
    pub accepted: bool,
    pub snapshot: FfiSyncSnapshot,
}

/// Explicit synchronization failure exposed to binding adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfiSyncError {
    InvalidConfiguration,
    CorrelationIdExhausted,
    SampleCountOverflow,
    DuplicateCorrelationId,
    PendingProbeLimitReached,
    StaleCorrelationId,
    CorrelationTimestampMismatch,
    LocalClockMovedBackwards,
    HostClockMovedBackwards,
    HostProcessingExceedsRoundTrip,
}

impl fmt::Display for FfiSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "sync estimator configuration is invalid",
            Self::CorrelationIdExhausted => "sync correlation ID space is exhausted",
            Self::SampleCountOverflow => "sync accepted-sample count exceeds the FFI field width",
            Self::DuplicateCorrelationId => "sync correlation ID is already pending",
            Self::PendingProbeLimitReached => "sync pending-probe limit was reached",
            Self::StaleCorrelationId => "sync correlation ID is stale or unknown",
            Self::CorrelationTimestampMismatch => {
                "sync response timestamp does not match its correlation ID"
            }
            Self::LocalClockMovedBackwards => "local monotonic clock moved backwards",
            Self::HostClockMovedBackwards => "host monotonic clock moved backwards",
            Self::HostProcessingExceedsRoundTrip => {
                "host processing duration exceeds the measured local round trip"
            }
        })
    }
}

impl Error for FfiSyncError {}

impl From<SyncEstimatorError> for FfiSyncError {
    fn from(error: SyncEstimatorError) -> Self {
        match error {
            SyncEstimatorError::InvalidConfiguration => Self::InvalidConfiguration,
            SyncEstimatorError::DuplicateCorrelationId { .. } => Self::DuplicateCorrelationId,
            SyncEstimatorError::PendingProbeLimitReached { .. } => Self::PendingProbeLimitReached,
            SyncEstimatorError::StaleCorrelationId { .. } => Self::StaleCorrelationId,
            SyncEstimatorError::CorrelationTimestampMismatch { .. } => {
                Self::CorrelationTimestampMismatch
            }
            SyncEstimatorError::Timestamp(SyncTimestampError::LocalClockMovedBackwards)
            | SyncEstimatorError::LocalClockMovedBackwards => Self::LocalClockMovedBackwards,
            SyncEstimatorError::Timestamp(SyncTimestampError::HostClockMovedBackwards) => {
                Self::HostClockMovedBackwards
            }
            SyncEstimatorError::Timestamp(
                SyncTimestampError::HostProcessingExceedsRoundTrip,
            ) => Self::HostProcessingExceedsRoundTrip,
        }
    }
}

/// Stateful synchronization estimator with binding-friendly records.
#[derive(Debug)]
pub struct FfiSyncEstimator {
    estimator: ClockSyncEstimator,
    next_correlation_id: u64,
    last_observation: Option<FfiSyncObservation>,
}

impl FfiSyncEstimator {
    /// Creates a bounded Rust estimator from fixed-width binding fields.
    ///
    /// # Errors
    ///
    /// Returns [`FfiSyncError::InvalidConfiguration`] when the sample count or
    /// RTT threshold is outside the core's validated limits.
    pub fn new(config: FfiSyncEstimatorConfig) -> Result<Self, FfiSyncError> {
        let max_samples = usize::try_from(config.max_samples)
            .map_err(|_| FfiSyncError::InvalidConfiguration)?;
        let estimator = ClockSyncEstimator::new(SyncEstimatorConfig {
            max_samples,
            max_accepted_rtt_ms: config.max_accepted_rtt_ms,
            ..SyncEstimatorConfig::default()
        })
        .map_err(FfiSyncError::from)?;
        Ok(Self {
            estimator,
            next_correlation_id: 1,
            last_observation: None,
        })
    }

    /// Processes one exchange and updates the Rust-owned estimator history.
    ///
    /// # Errors
    ///
    /// Returns an explicit binding error for correlation exhaustion, impossible
    /// timestamp ordering, or any estimator failure. No failed sample is
    /// represented as an accepted observation.
    pub fn observe(
        &mut self,
        exchange: FfiSyncExchange,
    ) -> Result<FfiSyncObservation, FfiSyncError> {
        let correlation_id = SyncCorrelationId::new(self.next_correlation_id);
        let next_correlation_id = self
            .next_correlation_id
            .checked_add(1)
            .ok_or(FfiSyncError::CorrelationIdExhausted)?;
        let local_send = LocalMonotonicMillis::new(exchange.t1_local_send_ms);
        self.estimator
            .begin_probe(correlation_id, local_send)
            .map_err(FfiSyncError::from)?;
        let observation = self
            .estimator
            .observe_response(
                correlation_id,
                local_send,
                HostMonotonicMillis::new(exchange.t2_host_receive_ms),
                HostMonotonicMillis::new(exchange.t3_host_send_ms),
                LocalMonotonicMillis::new(exchange.t4_local_receive_ms),
            )
            .map_err(FfiSyncError::from)?;
        let result = FfiSyncObservation {
            round_trip_time_ms: observation.sample.round_trip_time_ms,
            offset_ms: observation.sample.offset_ms,
            accepted: observation.accepted,
            snapshot: convert_snapshot(observation.snapshot)?,
        };
        self.next_correlation_id = next_correlation_id;
        self.last_observation = Some(result);
        Ok(result)
    }

    /// Returns the latest successfully calculated observation, when present.
    #[must_use]
    pub const fn last_observation(&self) -> Option<FfiSyncObservation> {
        self.last_observation
    }

    /// Returns the current Rust-owned synchronization snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`FfiSyncError::SampleCountOverflow`] if a future core change
    /// violates the fixed-width accepted-count contract.
    pub fn snapshot(&self) -> Result<FfiSyncSnapshot, FfiSyncError> {
        convert_snapshot(self.estimator.snapshot())
    }
}

fn convert_snapshot(snapshot: SyncSnapshot) -> Result<FfiSyncSnapshot, FfiSyncError> {
    let accepted_sample_count = u32::try_from(snapshot.accepted_sample_count)
        .map_err(|_| FfiSyncError::SampleCountOverflow)?;
    Ok(FfiSyncSnapshot {
        offset_ms: snapshot.offset_ms,
        round_trip_time_ms: snapshot.round_trip_time_ms,
        jitter_ms: snapshot.jitter_ms,
        skew_ppm: snapshot.skew_ppm,
        confidence_code: snapshot.confidence.stable_code(),
        accepted_sample_count,
    })
}

#[cfg(test)]
mod tests {
    use super::{FfiSyncEstimator, FfiSyncEstimatorConfig, FfiSyncError, FfiSyncExchange};

    const TOLERANCE: f64 = 1.0e-9;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= TOLERANCE,
            "expected {actual} to be within {TOLERANCE} of {expected}"
        );
    }

    #[test]
    fn binding_records_delegate_fixture_behavior_to_core() {
        let mut estimator =
            FfiSyncEstimator::new(FfiSyncEstimatorConfig::default()).expect("valid config");
        let first = estimator
            .observe(FfiSyncExchange {
                t1_local_send_ms: 1_000,
                t2_host_receive_ms: 1_012,
                t3_host_send_ms: 1_014,
                t4_local_receive_ms: 1_026,
            })
            .expect("first fixture sample");
        assert!(first.accepted);
        assert_close(first.round_trip_time_ms, 24.0);
        assert_close(first.offset_ms, 0.0);

        let second = estimator
            .observe(FfiSyncExchange {
                t1_local_send_ms: 2_000,
                t2_host_receive_ms: 2_015,
                t3_host_send_ms: 2_017,
                t4_local_receive_ms: 2_022,
            })
            .expect("second fixture sample");
        assert!(second.accepted);

        let rejected = estimator
            .observe(FfiSyncExchange {
                t1_local_send_ms: 3_000,
                t2_host_receive_ms: 3_100,
                t3_host_send_ms: 3_101,
                t4_local_receive_ms: 3_400,
            })
            .expect("high RTT sample remains a valid observation");
        assert!(!rejected.accepted);
        assert_close(rejected.round_trip_time_ms, 399.0);
        assert_close(rejected.offset_ms, -99.5);

        let snapshot = estimator.snapshot().expect("bounded snapshot");
        assert_close(snapshot.offset_ms, 5.0);
        assert_close(snapshot.round_trip_time_ms, 20.0);
        assert_close(snapshot.jitter_ms, 0.0);
        assert_eq!(snapshot.confidence_code, 5);
        assert_eq!(snapshot.accepted_sample_count, 2);
    }

    #[test]
    fn rejects_invalid_configuration_and_timestamp_ordering() {
        assert!(matches!(
            FfiSyncEstimator::new(FfiSyncEstimatorConfig {
                max_samples: 0,
                ..FfiSyncEstimatorConfig::default()
            }),
            Err(FfiSyncError::InvalidConfiguration)
        ));

        let mut estimator =
            FfiSyncEstimator::new(FfiSyncEstimatorConfig::default()).expect("valid config");
        assert_eq!(
            estimator.observe(FfiSyncExchange {
                t1_local_send_ms: 100,
                t2_host_receive_ms: 110,
                t3_host_send_ms: 111,
                t4_local_receive_ms: 99,
            }),
            Err(FfiSyncError::LocalClockMovedBackwards)
        );
        assert!(estimator.last_observation().is_none());
    }
}
