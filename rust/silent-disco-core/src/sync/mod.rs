mod estimator;
mod types;

pub use estimator::{
    ClockSyncEstimator, MAX_DRIFT_HISTORY_SAMPLES, MAX_ESTIMATOR_SAMPLES, MAX_PENDING_PROBES,
    SyncDecision, SyncEstimatorConfig, SyncEstimatorError, SyncObservation, SyncSnapshot,
    classify_confidence,
};
pub use types::{
    HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId, SyncExchange, SyncSample,
    SyncTimestampError,
};

#[cfg(test)]
mod fixture_tests;
