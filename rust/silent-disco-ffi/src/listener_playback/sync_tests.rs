// Synchronization-specific listener playback runtime regressions.
#![allow(clippy::float_cmp)]

use super::error::ListenerPlaybackError;
use super::runtime::ListenerPlaybackRuntime;
use crate::audio_abi::registry_test_guard;
use silent_disco_core::audio::{PlaybackPumpConfig, RenderRingConfig, SchedulerConfig};
use silent_disco_core::domain::{SessionId, StreamId};

const SAMPLES_PER_PACKET: u32 = 960;

fn scheduler_config() -> SchedulerConfig {
    let mut config = SchedulerConfig::new(
        SessionId::new("session-runtime").expect("session id"),
        StreamId::new("stream-runtime").expect("stream id"),
        48_000,
        0,
        SAMPLES_PER_PACKET,
        2,
    );
    config.startup_buffer_target_ms = 0;
    config
}

fn ring_config() -> RenderRingConfig {
    RenderRingConfig {
        capacity_frames: 48_000,
        target_fill_frames: 19_200,
    }
}

#[test]
fn an_accepted_sync_sample_unlocks_playback_and_a_rejected_one_does_not() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    // A sample whose round trip is far outside the acceptance window must
    // not reach the playback timeline -- nor the skew estimate, which is
    // exactly how a placeholder offset once poisoned it.
    runtime.begin_sync_probe(1, 0).expect("probe registered");
    let rejected = runtime
        .observe_sync_response(1, 0, 500_000, 500_001, 4_000)
        .expect("a correlated response is not an error");
    assert!(!rejected.accepted);
    assert!(!rejected.sync_locked);

    // A low-latency sample is accepted and unlocks playback.
    runtime
        .begin_sync_probe(2, 10_000)
        .expect("probe registered");
    let accepted = runtime
        .observe_sync_response(2, 10_000, 500_000, 500_002, 10_020)
        .expect("a correlated response is not an error");
    assert!(accepted.accepted);
    assert!(accepted.sync_locked);
    assert_eq!(accepted.accepted_sample_count, 1);
    // A single sample cannot support a skew regression yet.
    assert_eq!(accepted.skew_ppm, 0.0);

    runtime.stop().expect("stop succeeds");
}

#[test]
fn bounded_acquisition_metadata_crosses_the_listener_playback_boundary() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    for (correlation, t1) in [(1, 0), (2, 250), (3, 500)] {
        runtime
            .begin_sync_probe(correlation, t1)
            .expect("probe registered");
        let rejected = runtime
            .observe_sync_response(correlation, t1, t1 + 125, t1 + 125, t1 + 250)
            .expect("valid rejected sample");
        assert!(!rejected.accepted);
        assert!(!rejected.sync_locked);
    }

    runtime.begin_sync_probe(4, 750).expect("probe registered");
    let accepted = runtime
        .observe_sync_response(4, 750, 875, 875, 1_000)
        .expect("bounded acquisition sample");
    assert!(accepted.accepted);
    assert!(accepted.sync_locked);
    assert_eq!(accepted.acquisition_rejected_sample_count, 3);
    assert_eq!(accepted.acquisition_elapsed_ms, 1_000);
    assert!((accepted.acquisition_rtt_limit_ms - 375.0).abs() <= f64::EPSILON);
    assert!(accepted.degraded_lock);

    runtime.stop().expect("stop succeeds");
}

#[test]
fn an_uncorrelated_or_duplicate_sync_exchange_fails_explicitly() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    let unknown = runtime
        .observe_sync_response(99, 0, 1, 2, 3)
        .expect_err("a response with no registered probe must fail");
    assert!(matches!(unknown, ListenerPlaybackError::Sync(_)));

    runtime.begin_sync_probe(1, 0).expect("probe registered");
    let duplicate = runtime
        .begin_sync_probe(1, 0)
        .expect_err("a duplicate correlation id must fail");
    assert!(matches!(duplicate, ListenerPlaybackError::Sync(_)));

    runtime.stop().expect("stop succeeds");
}

#[test]
fn sync_calls_after_stop_fail_explicitly() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");
    runtime.stop().expect("stop succeeds");

    assert!(matches!(
        runtime.begin_sync_probe(1, 0),
        Err(ListenerPlaybackError::Stopped(_))
    ));
    assert!(matches!(
        runtime.observe_sync_response(1, 0, 1, 2, 3),
        Err(ListenerPlaybackError::Stopped(_))
    ));
}

#[test]
fn same_process_host_clock_lock_unlocks_playback_without_a_network_probe() {
    let _guard = registry_test_guard();
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config(),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("runtime starts");

    assert!(!runtime.diagnostics().sync_locked);
    let host_now = 50_000 + runtime.now_ms();
    runtime
        .lock_same_process_host_clock(host_now)
        .expect("same-process clock lock succeeds");
    assert!(runtime.diagnostics().sync_locked);

    runtime.set_volume(0.25).expect("valid live volume");
    let invalid = runtime
        .set_volume(f32::INFINITY)
        .expect_err("non-finite volume must fail");
    assert!(matches!(
        invalid,
        ListenerPlaybackError::InvalidConfiguration(_)
    ));

    runtime.stop().expect("stop succeeds");
    assert!(matches!(
        runtime.lock_same_process_host_clock(host_now),
        Err(ListenerPlaybackError::Stopped(_))
    ));
}
