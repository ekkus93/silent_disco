//! Playback state-machine coverage: stop/pause/resume command legality,
//! duplicate/stale-command rejection (Block 27.3), and visible failure
//! reporting for a pump that cannot finish stopping or a source that fails
//! to parse or read cleanly (Block 28.2).

use super::fixtures::{
    corrupt_wav_bytes, stage_long_source, stage_source, stage_wav_source,
    truncated_body_full_header_wav,
};
use super::harness::{
    TEST_TIMEOUT, join_and_approve_listener, real_private_lan_address, start_host_session,
    wait_for_audio, wait_for_control, wait_snapshot,
};
use crate::platform::start_playback;
use silent_disco_core::domain::PlaybackState;
use silent_disco_core::protocol::ControlMessage;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// `stop_playback` must report a pump that could not finish stopping, rather
/// than returning `Ok` regardless.
///
/// The pump's exit is what broadcasts `Stop` and transitions the actor to
/// `Stopped`, and all three of its shutdown steps -- plus a panicking pump
/// thread -- were discarded (`drop(pump.join())`, `drop(handle.submit_audio_event(..))`).
/// A caller could therefore be told the stream stopped while the session was
/// still `Playing`, with nothing anywhere reporting otherwise.
#[test]
fn stop_playback_reports_a_pump_that_could_not_complete_its_shutdown() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);
    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });

    // Take the actor away, so the pump's closing `Stopped` transition cannot
    // be delivered. Stopping must surface that instead of claiming success.
    actor.shutdown().expect("actor shutdown");

    let error = network
        .stop_playback()
        .expect_err("stopping must report that the pump could not finish");
    // Which layer reports it (the actor, here) matters less than that the
    // failure is structured and reaches the caller at all.
    assert!(
        !error.code.is_empty(),
        "the failure must carry a stable code"
    );
    assert!(
        !error.message.is_empty(),
        "the reported failure must say what went wrong"
    );

    listener.shutdown().expect("listener shutdown");
    // The actor was deliberately taken away, so the host shutdown reports that
    // too rather than claiming success -- the same property, one layer up.
    network
        .shutdown()
        .expect_err("shutdown must report the missing actor");
}

/// End-of-stream, then an explicit stop: the actor must still end at
/// `PlaybackState::Stopped`.
///
/// This is the manual device test's song-change step in miniature. There, a
/// 40s source is stopped at 40s, so the pump has usually already exited on
/// its own when `stop_playback` arrives -- and the actor was observed never
/// reaching `Stopped` even though `stop_playback` returned success.
#[test]
fn stopping_after_the_source_has_already_ended_still_leaves_the_actor_stopped() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);
    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });

    // Let the short source run out, so the pump exits on its own.
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::Stop(_))
    });

    // Now stop explicitly, exactly as a song change does.
    network.stop_playback().expect("stop playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped
    });

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Position must reflect real playback progress, and a source finishing on
/// its own must be distinguishable from an explicit stop.
///
/// `stage_long_source` is 3 real seconds -- comfortably enough stream-time
/// for the pump's throttled position reports to advance well past their
/// first value, and short enough that waiting for natural completion is a
/// reasonable test cost.
#[test]
fn playback_reports_advancing_position_and_natural_completion() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);
    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });
    let _first_audio = wait_for_audio(&mut *listener);

    // Real time has already passed by this point (receiving that first audio
    // frame took a real round trip), so position may already be well past
    // zero -- that race is exactly why the reset-to-zero invariant itself is
    // covered at the actor level in
    // host_block12_actor_lifecycle::playback_position_and_natural_completion_are_tracked_authoritatively
    // rather than asserted here. This test's job is proving the pump
    // actually wires real advancing values end to end.
    assert!(
        !handle
            .current_snapshot()
            .expect("current snapshot")
            .stream_ended_naturally
    );

    // Position must genuinely advance while the source keeps playing --
    // not merely be nonzero once, but keep climbing.
    let first_advance = wait_snapshot(&handle, |snapshot| snapshot.playback_position_ms > 0);
    let further_advance = wait_snapshot(&handle, |snapshot| {
        snapshot.playback_position_ms > first_advance.playback_position_ms
    });
    assert!(further_advance.playback_position_ms > first_advance.playback_position_ms);
    assert!(!further_advance.stream_ended_naturally);

    // The source then finishes on its own, which must be visible as
    // something other than the generic state an explicit stop produces.
    let ended = wait_snapshot(&handle, |snapshot| snapshot.stream_ended_naturally);
    assert_eq!(ended.playback_state, PlaybackState::Stopped);
    assert!(
        ended.playback_position_ms > 0,
        "the position at natural completion must still reflect real playback progress"
    );

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Block 27.3 "stale command rejection": a duplicate/stale Play click while
/// a stream is already active must be rejected explicitly, not accepted as
/// a second concurrent stream. The desktop TODO's Block 27.3 note records
/// the decision that today's invalid-state checks (this one included)
/// already satisfy that requirement, so this locks the behavior in rather
/// than adding new revision-tracking machinery.
#[test]
fn starting_playback_twice_is_rejected_as_a_duplicate_command() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry).expect("first start succeeds");
    let error = start_playback::start(&handle, &network, &registry)
        .expect_err("a second start while one is active must be rejected");
    assert!(
        !error.code.is_empty(),
        "the failure must carry a stable code"
    );

    network.stop_playback().expect("stop playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Stopped
    });
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// A duplicate/stale Stop click after playback has already stopped must be
/// rejected explicitly rather than silently reported as another success --
/// the exact "duplicate click" scenario the Block 27.3 TODO note names.
#[test]
fn stopping_playback_twice_is_rejected_not_silently_successful() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry).expect("start playback");
    network.stop_playback().expect("first stop succeeds");
    let error = network
        .stop_playback()
        .expect_err("a second stop after playback already ended must be rejected");
    assert!(
        !error.code.is_empty(),
        "the failure must carry a stable code"
    );

    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Pause/resume/stop before anything is playing must all be rejected
/// explicitly -- none may silently succeed or silently no-op.
#[test]
fn pause_resume_stop_before_playback_started_are_all_rejected() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, _registry) = stage_source(&temp);
    let (actor, _handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    network
        .pause_playback()
        .expect_err("pausing with nothing playing must be rejected");
    network
        .resume_playback()
        .expect_err("resuming with nothing playing must be rejected");
    network
        .stop_playback()
        .expect_err("stopping with nothing playing must be rejected");

    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Block 28.2 "corrupt source fixture fails visibly": a source too damaged
/// to even parse (here, a WAV truncated before its header is complete) must
/// fail synchronously and visibly at the `start_playback` orchestration
/// level -- a structured `Err`, plus the actor snapshot reporting
/// `PlaybackState::Error` -- not a silent success that later streams
/// nothing. `start_after_buffering`'s `?` on `prepare_staged_audio_source`
/// is what should produce this; this test is the regression lock for it,
/// device-independent per Block 28.2's own split (see `docs/
/// AUDIO_PLAYBACK_STATE_2026-08-10.md` D1).
#[test]
fn starting_playback_with_a_corrupt_source_fails_visibly_at_the_orchestration_level() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_wav_source(&temp, "corrupt-source", corrupt_wav_bytes());
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    let error = start_playback::start(&handle, &network, &registry)
        .expect_err("a corrupt source fixture must fail visibly, not silently succeed");
    assert!(
        !error.code.is_empty(),
        "the failure must carry a stable code"
    );
    assert!(
        !error.message.is_empty(),
        "the reported failure must say what went wrong"
    );

    // The failure must be visible in the authoritative snapshot too, not
    // only in the direct return value -- a caller that only polls state
    // (e.g. a UI that missed the direct error) must still see it.
    // `submit_audio_event` only queues the transition; the actor applies it
    // on its own thread, so this must poll rather than check immediately
    // (same reasoning as `resuming_while_already_playing_does_not_corrupt_position`).
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Error
    });

    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Block 28.2 "host source read failure does not claim continued normal
/// streaming": unlike the corrupt-fixture case above, this source's WAV
/// header parses fine and declares 3 real seconds of audio, but the file is
/// truncated to only ~0.1s of actual sample data -- so `start_playback`
/// itself succeeds (the shared decoder only inspects the header up front;
/// see `StreamingDecodeHandle::open`), and the read failure only surfaces
/// once the packetizer worker actually decodes past the truncation point,
/// exactly the "started fine, host source failed mid-stream" scenario this
/// item guards. `run_pump`'s non-cancelled error branch is what should:
/// (a) stop the actor leaving it visibly `Playing` forever, and
/// (b) surface the real failure through `stop_playback` rather than
/// reporting a clean stop.
#[test]
fn a_host_source_read_failure_mid_stream_does_not_claim_continued_normal_streaming() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_wav_source(
        &temp,
        "mid-stream-read-failure",
        truncated_body_full_header_wav(),
    );
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry)
        .expect("a header that parses fine must start playback, even though its body is short");

    // The pump must notice the read failure and exit on its own -- polling
    // `playback_is_active` is exactly what `start_playback::start` itself
    // relies on to detect a finished stream, so it is the right signal here
    // too. This must go false without anyone calling `stop_playback`.
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline && network.playback_is_active().expect("playback state") {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !network
            .playback_is_active()
            .expect("playback state after the read failure"),
        "the pump must exit on its own after the source read failure, not sit claiming an \
         active stream forever"
    );

    // `playback_is_active` going false only means the pump thread's closure
    // returned; the `PlaybackStateChanged(Stopped)` it queued on the way out
    // is still applied asynchronously by the actor's own thread, so this
    // must poll rather than check `current_snapshot()` immediately (same
    // reasoning as the corrupt-fixture test above).
    let snapshot = wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state != PlaybackState::Playing
    });
    assert!(
        !snapshot.stream_ended_naturally,
        "a mid-stream read failure is not a clean end-of-file and must not be reported as one"
    );

    // The failure itself must reach a caller, not be swallowed as a normal
    // stop just because the pump had already exited by the time this runs.
    let error = network
        .stop_playback()
        .expect_err("stopping after a source read failure must surface that failure");
    assert!(
        !error.code.is_empty(),
        "the failure must carry a stable code"
    );
    assert!(
        !error.message.is_empty(),
        "the reported failure must say what went wrong"
    );

    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Locks in the Block 27.3 zero-recipient decision: starting playback with
/// no connected listeners is allowed (the existing delivery-health banner
/// from 27.2 is the only signal), not blocked outright. See the desktop
/// TODO's Block 27.3 note for the product reasoning.
#[test]
fn starting_playback_with_zero_listeners_is_allowed() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_source(&temp);
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry)
        .expect("starting with zero connected listeners must succeed");

    network.stop_playback().expect("stop playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Stopped
    });
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Exploratory: resume is implemented by resubmitting `Playing`, and the
/// actor only treats that as "continue from a pause" when the prior state
/// was exactly `Paused` (see `runtime/actor_runtime/state/audio.rs`) --
/// otherwise it resets position to zero, on the theory that a `Playing`
/// submission from any other state is a genuinely new stream. A stale
/// Resume command arriving while already playing (not paused) would hit
/// that "otherwise" branch. This test exists to find out whether that is
/// real before Block 27.3 is called closed on the strength of "today's
/// checks are enough".
#[test]
fn resuming_while_already_playing_does_not_corrupt_position() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let (actor, handle, _receiver, _advertisement, network, _endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    start_playback::start(&handle, &network, &registry).expect("start playback");
    let advanced = wait_snapshot(&handle, |snapshot| snapshot.playback_position_ms > 0);

    // Not paused -- this is exactly the "stale/duplicate Resume" case.
    network
        .resume_playback()
        .expect("resume while already playing is accepted by today's checks");

    // `submit_audio_event` only queues the event; the actor applies it on
    // its own thread. A reset (if it happens) lands the moment the actor
    // processes this specific event, not on the ~250ms position-report
    // throttle, so poll tightly across a window rather than checking once
    // immediately after the call -- a single immediate read would likely
    // observe the pre-resume snapshot and pass vacuously either way.
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut minimum_observed = advanced.playback_position_ms;
    while Instant::now() < deadline {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        minimum_observed = minimum_observed.min(snapshot.playback_position_ms);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        minimum_observed >= advanced.playback_position_ms,
        "a resume command while already playing must not roll position back to zero \
         (was {}, dropped to {} at some point in the following 500ms)",
        advanced.playback_position_ms,
        minimum_observed
    );

    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}
