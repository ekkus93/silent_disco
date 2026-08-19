use super::{
    ACQUISITION_ADAPTIVE_CEILING_MS, ACQUISITION_HARD_CEILING_MS, ClockSyncEstimator,
    MAX_PENDING_PROBES, PENDING_PROBE_MAX_AGE_MS, SyncDecision, SyncEstimatorConfig,
    SyncEstimatorError, SyncSnapshot,
};
use crate::{
    domain::SyncConfidence,
    sync::{HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId},
};

const FLOAT_TOLERANCE: f64 = 1.0e-9;

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= FLOAT_TOLERANCE,
        "expected {actual} to be within {FLOAT_TOLERANCE} of {expected}"
    );
}

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
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");
    assert!(
        observe(&mut estimator, 1, 1_000, 1_012, 1_014, 1_026)
            .expect("first sample")
            .accepted
    );
    assert!(
        observe(&mut estimator, 2, 2_000, 2_015, 2_017, 2_022)
            .expect("second sample")
            .accepted
    );
    assert!(
        !observe(&mut estimator, 3, 3_000, 3_100, 3_101, 3_400)
            .expect("high RTT sample is valid but rejected")
            .accepted
    );

    let snapshot = estimator.snapshot();
    assert_close(snapshot.offset_ms, 5.0);
    assert_close(snapshot.round_trip_time_ms, 20.0);
    assert_close(snapshot.jitter_ms, 0.0);
    assert_eq!(snapshot.confidence, SyncConfidence::Excellent);
    assert_eq!(snapshot.accepted_sample_count, 2);
}

#[test]
fn correlation_ids_are_bounded_unique_and_single_use() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");
    let correlation = SyncCorrelationId::new(7);
    estimator
        .begin_probe(correlation, LocalMonotonicMillis::new(100))
        .expect("first registration succeeds");
    assert_eq!(
        estimator.begin_probe(correlation, LocalMonotonicMillis::new(100)),
        Err(SyncEstimatorError::DuplicateCorrelationId {
            correlation_id: correlation
        })
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
        Err(SyncEstimatorError::StaleCorrelationId {
            correlation_id: correlation
        })
    );

    for id in 0..MAX_PENDING_PROBES {
        estimator
            .begin_probe(
                SyncCorrelationId::new(
                    1_000 + u64::try_from(id).expect("pending-probe index fits u64"),
                ),
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

/// The regression this exists to prevent: before eviction, filling
/// `pending` with lost responses bricked `begin_probe` for the rest of
/// the stream, since nothing ever removed an entry except a matching
/// `observe_response`. A probe attempt after the age threshold must
/// recover on its own, not require every prior probe to eventually
/// answer.
#[test]
fn stale_pending_probes_are_evicted_so_probing_recovers_from_sustained_loss() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");
    for id in 0..MAX_PENDING_PROBES {
        estimator
            .begin_probe(
                SyncCorrelationId::new(u64::try_from(id).expect("index fits u64")),
                LocalMonotonicMillis::new(1_000),
            )
            .expect("pending capacity not reached");
    }
    assert_eq!(estimator.pending_probe_count(), MAX_PENDING_PROBES);

    // Still fully stuck one millisecond before the age threshold.
    assert!(matches!(
        estimator.begin_probe(
            SyncCorrelationId::new(20_000),
            LocalMonotonicMillis::new(1_000 + PENDING_PROBE_MAX_AGE_MS - 1),
        ),
        Err(SyncEstimatorError::PendingProbeLimitReached { .. })
    ));

    // At the threshold, every one of the 64 lost probes is stale and a
    // fresh probe succeeds -- this is "recovers", not "shrinks slowly".
    estimator
        .begin_probe(
            SyncCorrelationId::new(20_001),
            LocalMonotonicMillis::new(1_000 + PENDING_PROBE_MAX_AGE_MS),
        )
        .expect("probing recovers once the lost probes have aged out");
    assert_eq!(
        estimator.pending_probe_count(),
        1,
        "eviction must drop every stale entry, not just make room for one"
    );
}

/// Eviction must not be trigger-happy: a probe still within its answer
/// window has to survive a later `begin_probe` call, or a real,
/// in-flight response would arrive to find its correlation ID already
/// gone.
#[test]
fn a_probe_within_its_age_window_survives_a_later_begin_probe() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");
    let first = SyncCorrelationId::new(1);
    estimator
        .begin_probe(first, LocalMonotonicMillis::new(1_000))
        .expect("first probe registers");

    estimator
        .begin_probe(
            SyncCorrelationId::new(2),
            LocalMonotonicMillis::new(1_000 + PENDING_PROBE_MAX_AGE_MS - 1),
        )
        .expect("second probe registers just under the age threshold");

    // The still-young first probe must still be answerable.
    estimator
        .observe_response(
            first,
            LocalMonotonicMillis::new(1_000),
            HostMonotonicMillis::new(1_010),
            HostMonotonicMillis::new(1_011),
            LocalMonotonicMillis::new(1_020),
        )
        .expect("a response within the age window must still find its correlation ID");
}

#[test]
fn mismatched_echo_consumes_correlation_and_fails_visibly() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");
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
        Err(SyncEstimatorError::CorrelationTimestampMismatch {
            correlation_id: correlation
        })
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
    assert_eq!(
        super::classify_confidence(20.0, 2.0),
        SyncConfidence::Excellent
    );
    assert_eq!(super::classify_confidence(50.0, 5.0), SyncConfidence::Good);
    assert_eq!(super::classify_confidence(90.0, 12.0), SyncConfidence::Fair);
    assert_eq!(super::classify_confidence(90.1, 0.0), SyncConfidence::Poor);
}

#[test]
fn decisions_distinguish_initial_periodic_drift_and_wait() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");
    assert_eq!(
        estimator.decision(LocalMonotonicMillis::new(0), SyncSnapshot::default()),
        Ok(SyncDecision::InitialProbeRequired)
    );
    let observation =
        observe(&mut estimator, 1, 1_000, 1_010, 1_011, 1_020).expect("accepted sample");
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
    assert!(matches!(
        ClockSyncEstimator::new(SyncEstimatorConfig {
            max_samples: 0,
            ..SyncEstimatorConfig::default()
        }),
        Err(SyncEstimatorError::InvalidConfiguration)
    ));
    assert!(matches!(
        ClockSyncEstimator::new(SyncEstimatorConfig {
            max_accepted_rtt_ms: f64::NAN,
            ..SyncEstimatorConfig::default()
        }),
        Err(SyncEstimatorError::InvalidConfiguration)
    ));
}

#[test]
fn acquisition_adapts_only_after_repeated_measured_rejections() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");

    for (correlation, t1) in [(1, 0), (2, 250), (3, 500)] {
        let observation = observe(
            &mut estimator,
            correlation,
            t1,
            t1 + 125,
            t1 + 125,
            t1 + 250,
        )
        .expect("valid high-RTT acquisition sample");
        assert!(!observation.accepted);
        assert_close(observation.acquisition.effective_rtt_limit_ms, 200.0);
    }

    let accepted =
        observe(&mut estimator, 4, 750, 875, 875, 1_000).expect("adaptive acquisition sample");
    assert!(
        accepted.accepted,
        "measured 250ms path should acquire after bounded adaptation"
    );
    assert_eq!(accepted.acquisition.rejected_sample_count, 3);
    assert_eq!(accepted.acquisition.elapsed_ms, 1_000);
    assert_close(accepted.acquisition.effective_rtt_limit_ms, 375.0);
    assert!(accepted.acquisition.degraded_lock);
    assert_eq!(accepted.snapshot.accepted_sample_count, 1);
}

#[test]
fn acquisition_never_exceeds_its_bounded_ceiling_and_steady_state_returns_to_strict_rtt() {
    let mut estimator =
        ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("default config is valid");

    for (correlation, t1) in [(1, 0), (2, 250), (3, 500)] {
        assert!(
            !observe(
                &mut estimator,
                correlation,
                t1,
                t1 + 125,
                t1 + 125,
                t1 + 250,
            )
            .expect("valid rejected acquisition sample")
            .accepted
        );
    }

    let too_slow =
        observe(&mut estimator, 4, 750, 1_150, 1_150, 1_550).expect("valid very-high-RTT sample");
    assert!(!too_slow.accepted);
    assert!(too_slow.acquisition.effective_rtt_limit_ms <= ACQUISITION_ADAPTIVE_CEILING_MS);

    let hard_ceiling_lock = observe(&mut estimator, 5, 2_000, 2_400, 2_400, 2_800)
        .expect("hard-ceiling acquisition sample");
    assert!(hard_ceiling_lock.accepted);
    assert_close(
        hard_ceiling_lock.acquisition.effective_rtt_limit_ms,
        ACQUISITION_HARD_CEILING_MS,
    );
    assert!(hard_ceiling_lock.acquisition.degraded_lock);

    let steady_state = observe(&mut estimator, 6, 3_000, 3_125, 3_125, 3_250)
        .expect("steady-state high-RTT sample");
    assert!(
        !steady_state.accepted,
        "steady state must restore the strict 200ms gate"
    );
    assert_close(steady_state.acquisition.effective_rtt_limit_ms, 200.0);
    assert!(steady_state.acquisition.degraded_lock);
    assert_eq!(steady_state.snapshot.accepted_sample_count, 1);
}
