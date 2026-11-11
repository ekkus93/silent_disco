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

// --- Session clock handover across streams -------------------------------

use super::runtime::{SYNC_HANDOVER_MAX_AGE_MS, fresh_locked_offset};
use silent_disco_core::domain::{MonotonicMillis, PacketSequence, SampleIndex};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};
use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};

/// Host start time for handover streams: just ahead of "host now" under the
/// fixture's offset (host `500_001` at local `10_010`, so host now is about
/// `490_000` plus the runtime clock), so their first packet is still due.
const HANDOVER_HOST_START_MS: u64 = 491_000;

fn scheduler_config_for(stream: &str) -> SchedulerConfig {
    let mut config = SchedulerConfig::new(
        SessionId::new("session-runtime").expect("session id"),
        StreamId::new(stream).expect("stream id"),
        48_000,
        HANDOVER_HOST_START_MS,
        SAMPLES_PER_PACKET,
        2,
    );
    config.startup_buffer_target_ms = 0;
    config
}

fn datagram_for(stream: &str, sequence: u64) -> AudioDatagram {
    AudioDatagram {
        session_id: SessionId::new("session-runtime").expect("session id"),
        stream_id: StreamId::new(stream).expect("stream id"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: SAMPLES_PER_PACKET,
        first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
        host_presentation_time_ms: MonotonicMillis::new(HANDOVER_HOST_START_MS + sequence * 20),
        payload: (0..SAMPLES_PER_PACKET * 2)
            .flat_map(|_| 1_000_i16.to_le_bytes())
            .collect(),
    }
}

/// A first stream whose estimator has one accepted, low-latency sample.
fn locked_first_stream() -> ListenerPlaybackRuntime {
    let runtime = ListenerPlaybackRuntime::start(
        scheduler_config_for("stream-a"),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("first stream starts");
    runtime
        .begin_sync_probe(1, 10_000)
        .expect("probe registered");
    let outcome = runtime
        .observe_sync_response(1, 10_000, 500_000, 500_002, 10_020)
        .expect("correlated response");
    assert!(outcome.sync_locked, "fixture must be locked");
    runtime
}

#[test]
fn a_stream_opened_from_a_fresh_handover_is_locked_and_drops_nothing() {
    let _guard = registry_test_guard();
    let first = locked_first_stream();
    let handover = first.sync_handover();
    first.stop().expect("first stream stops");

    let next = ListenerPlaybackRuntime::start_continuing(
        scheduler_config_for("stream-b"),
        ring_config(),
        PlaybackPumpConfig::default(),
        handover,
    )
    .expect("next stream starts");
    assert!(
        next.diagnostics().sync_locked,
        "the session's lock must carry into the next stream before any new sample"
    );

    next.submit_packet(datagram_for("stream-b", 0))
        .expect("packet submitted");
    let diagnostics = next.diagnostics();
    assert_eq!(diagnostics.dropped_before_sync, 0);
    assert_eq!(diagnostics.packets_accepted, 1);
    next.stop().expect("next stream stops");
}

#[test]
fn the_next_stream_keeps_the_session_clock_and_its_estimate() {
    let _guard = registry_test_guard();
    let first = locked_first_stream();
    let first_now = first.now_ms();
    let handover = first.sync_handover();
    first.stop().expect("first stream stops");

    let next = ListenerPlaybackRuntime::start_continuing(
        scheduler_config_for("stream-b"),
        ring_config(),
        PlaybackPumpConfig::default(),
        handover,
    )
    .expect("next stream starts");
    // Same timeline: the clock does not restart, so a carried offset (host
    // minus local time) still maps host time correctly.
    assert!(next.now_ms() >= first_now);

    // The estimate continues rather than restarting: a new accepted sample
    // is counted on top of the carried one.
    next.begin_sync_probe(2, 10_100).expect("probe registered");
    let outcome = next
        .observe_sync_response(2, 10_100, 500_080, 500_082, 10_120)
        .expect("correlated response");
    assert!(outcome.accepted);
    assert_eq!(outcome.accepted_sample_count, 2);
    next.stop().expect("next stream stops");
}

#[test]
fn a_handover_without_an_accepted_sample_leaves_the_next_stream_waiting_for_sync() {
    let _guard = registry_test_guard();
    let first = ListenerPlaybackRuntime::start(
        scheduler_config_for("stream-a"),
        ring_config(),
        PlaybackPumpConfig::default(),
        0.0,
    )
    .expect("first stream starts");
    let handover = first.sync_handover();
    first.stop().expect("first stream stops");

    let next = ListenerPlaybackRuntime::start_continuing(
        scheduler_config_for("stream-b"),
        ring_config(),
        PlaybackPumpConfig::default(),
        handover,
    )
    .expect("next stream starts");
    assert!(!next.diagnostics().sync_locked);
    next.submit_packet(datagram_for("stream-b", 0))
        .expect("packet submitted");
    // Held for the lock, not played against an unknown timeline.
    assert_eq!(next.diagnostics().packets_accepted, 0);
    assert_eq!(next.diagnostics().dropped_before_sync, 0);
    next.stop().expect("next stream stops");
}

#[test]
fn a_stale_carried_estimate_does_not_unlock_a_new_stream() {
    let mut estimator = ClockSyncEstimator::new(SyncEstimatorConfig::default()).expect("estimator");
    assert_eq!(
        fresh_locked_offset(&estimator, 0),
        None,
        "no sample, no lock"
    );

    estimator
        .begin_probe(SyncCorrelationId::new(1), LocalMonotonicMillis::new(10_000))
        .expect("probe registered");
    let observation = estimator
        .observe_response(
            SyncCorrelationId::new(1),
            LocalMonotonicMillis::new(10_000),
            HostMonotonicMillis::new(500_000),
            HostMonotonicMillis::new(500_002),
            LocalMonotonicMillis::new(10_020),
        )
        .expect("correlated response");
    assert!(observation.accepted);
    let offset = observation.snapshot.offset_ms;

    assert_eq!(
        fresh_locked_offset(&estimator, 10_020 + SYNC_HANDOVER_MAX_AGE_MS),
        Some(offset)
    );
    assert_eq!(
        fresh_locked_offset(&estimator, 10_020 + SYNC_HANDOVER_MAX_AGE_MS + 1),
        None
    );
}
