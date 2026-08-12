//! Block 26 playback robustness regressions: pause progression and failures
//! that occur after a stream is already active.

use super::fixtures::stage_long_source;
use super::harness::{TEST_TIMEOUT, real_private_lan_address, start_host_session, wait_snapshot};
use crate::platform::start_playback;
use silent_disco_core::domain::PlaybackState;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[test]
fn pause_stops_future_presentation_progression_until_resume() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this test host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry).expect("start playback");
    let playing = wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Playing && snapshot.playback_position_ms > 0
    });
    assert!(playing.playback_position_ms > 0);

    network.pause_playback().expect("pause playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Paused
    });
    std::thread::sleep(Duration::from_millis(150));
    let paused_position = handle
        .current_snapshot()
        .expect("paused snapshot")
        .playback_position_ms;
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        handle
            .current_snapshot()
            .expect("later paused snapshot")
            .playback_position_ms,
        paused_position,
        "authoritative playback position advanced while paused"
    );

    network.resume_playback().expect("resume playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Playing
            && snapshot.playback_position_ms > paused_position
    });
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

#[test]
fn a_transport_worker_failure_mid_stream_is_reported_by_the_playback_pump() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this test host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry).expect("start playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Playing
    });
    network
        .stop_transport_worker_for_test()
        .expect("stop only the transport worker");

    let deadline = Instant::now() + TEST_TIMEOUT;
    while network
        .playback_is_active()
        .expect("read playback activity after transport failure")
    {
        assert!(
            Instant::now() < deadline,
            "playback pump kept running after its transport worker disappeared"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let error = network
        .stop_playback()
        .expect_err("joining the failed pump must surface the transport failure");
    assert_eq!(error.subsystem, "transport");
    assert!(
        error.message.contains("shutting down") || error.message.contains("unavailable"),
        "unexpected transport failure message: {}",
        error.message
    );
    let stopped = wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Stopped
    });
    assert_eq!(stopped.playback_state, PlaybackState::Stopped);

    network.shutdown().expect("finish desktop host shutdown");
    actor.shutdown().expect("actor shutdown");
}
