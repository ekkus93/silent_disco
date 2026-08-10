use super::file_picker::{AudioContainer, InspectedAudioSource, SelectedSourceRegistry};
use super::network::{AddressRecord, DesktopHostNetworkControl, InterfaceRecord, TestHostPorts};
use super::start_playback;
use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, MonotonicMillis, PlaybackState, SyncConfidence,
};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame, StreamStart, SyncRequest,
    SyncResponse, SynchronizationReport,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, CoreSnapshot, HostDraftPatch, InviteCodePatch,
    PlatformEffectRequest, PlatformEvent, PlatformOperationCompletion, SnapshotRevision,
    TransportEffect,
};
use silent_disco_core::transport::{
    ListenerTransportConfig, ListenerTransportNode, SystemTransportClock, TransportChannel,
    TransportEvent, TransportFactory, production_transport_factory,
};
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for state-transition waits in manual device tests only. Real
/// devices/emulators are measurably slower and more congested than the
/// automated suite's in-process loopback listener -- see
/// [`wait_snapshot_for`] for the specific `queue_overflows=930` run that
/// motivated this.
const MANUAL_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `resume_rebroadcasts_stream_start_with_the_anchor_shifted_by_the_pause_duration`
/// holds its real pause -- long enough that a millisecond of test-harness
/// scheduling jitter can't be mistaken for zero shift, short enough to keep
/// the automated suite fast.
const RESUME_TEST_PAUSE_DURATION: Duration = Duration::from_millis(500);

#[test]
fn desktop_host_streams_real_audio_and_answers_sync_requests() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; streaming playback coverage remains deterministic"
        );
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
    let first_audio = wait_for_audio(&mut *listener);
    assert_eq!(first_audio.session_id, advertisement.session_id);
    assert!(!first_audio.payload.is_empty());

    let correlation_id = 7;
    listener
        .send_sync_request(&SyncRequest {
            session_id: advertisement.session_id.clone(),
            correlation_id,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(0),
        })
        .expect("send sync request");
    let sync_response = wait_for_sync_response(&mut *listener, correlation_id);
    assert_eq!(sync_response.session_id, advertisement.session_id);
    assert!(
        sync_response.t3_host_send_elapsed_ms.get()
            >= sync_response.t2_host_receive_elapsed_ms.get()
    );

    network.stop_playback().expect("stop playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::Stop(_))
    });
    // Stopping is only genuinely done once the actor has left `Playing`.
    // `stop_playback` used to return `Ok` without this ever happening.
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped
    });

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// D2 (`docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`): a listener's
/// `SynchronizationReport` -- the only channel that can ever populate the
/// host's per-listener sync diagnostics, since the host itself never sees
/// `t4` -- must reach `HostTransportEventProcessor`, translate into
/// `AudioEvent::SynchronizationUpdated`, and land on the matching
/// listener's `snapshot.listeners[].synchronization`. Exercises the real
/// wire path end to end (encode -> loopback socket -> decode -> validate ->
/// actor), not just the actor-level handling already covered by
/// `synchronization_updated_populates_top_level_and_per_listener_summary`
/// in `host_block12_actor_lifecycle.rs`.
#[test]
fn a_listener_synchronization_report_populates_the_hosts_per_listener_diagnostics() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this CI host; skipping");
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, _registry) = stage_source(&temp);
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
    // `join_and_approve_listener` only waits for the listener to observe
    // its own `JoinApproval` control frame -- the actor's own
    // `snapshot.listeners` update is a separate, asynchronously-applied
    // transport effect, so this must poll rather than read
    // `current_snapshot()` immediately (same reasoning as the D1 tests'
    // `submit_audio_event` race).
    let connected = wait_snapshot(&handle, |snapshot| !snapshot.listeners.is_empty());
    let device_id = connected
        .listeners
        .first()
        .expect("the approved listener is present")
        .device_id
        .clone();
    assert!(
        connected
            .listeners
            .first()
            .expect("listener present")
            .synchronization
            .is_none(),
        "a freshly joined listener must start with no synchronization summary"
    );

    listener
        .send_control(&ControlMessage::SynchronizationReport(
            SynchronizationReport {
                session_id: advertisement.session_id.clone(),
                listener_id: device_id.clone(),
                confidence: SyncConfidence::Excellent,
                offset_ms: -8.5,
                round_trip_ms: 16.25,
                drift_ppm: 2.5,
            },
        ))
        .expect("send synchronization report");

    let updated = wait_snapshot(&handle, |snapshot| {
        snapshot
            .listeners
            .iter()
            .any(|entry| entry.device_id == device_id && entry.synchronization.is_some())
    });
    let synchronization = updated
        .listeners
        .iter()
        .find(|entry| entry.device_id == device_id)
        .and_then(|entry| entry.synchronization)
        .expect("synchronization summary present after wait_snapshot's own predicate");
    assert_eq!(synchronization.confidence, SyncConfidence::Excellent);
    assert!((synchronization.offset_ms - (-8.5)).abs() < f64::EPSILON);
    assert!((synchronization.round_trip_ms - 16.25).abs() < f64::EPSILON);
    assert!((synchronization.drift_ppm - 2.5).abs() < f64::EPSILON);

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Guards the pause/resume presentation-timeline fix: the packetizer keeps
/// computing every frame's presentation time from a fixed anchor set once at
/// stream start, with no way to know real time kept moving while a pause
/// stopped the pump from draining it -- so, without `resume_playback`
/// re-broadcasting `StreamStart` with an anchor shifted forward by the real
/// pause duration, a resumed listener (and the pump's own send-ahead pacing)
/// would read every subsequent frame as already late. Confirmed on a real
/// Android device as the "started fine, fell apart into popping/crackling
/// partway through" symptom -- this is the deterministic, hardware-free
/// regression test for the fix, run over loopback rather than real Wi-Fi.
#[test]
fn resume_rebroadcasts_stream_start_with_the_anchor_shifted_by_the_pause_duration() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; pause/resume anchor coverage remains \
             deterministic"
        );
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
    let original_start = wait_for_stream_start(&mut *listener);
    let _ = wait_for_audio(&mut *listener);

    network.pause_playback().expect("pause playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == PlaybackState::Paused
    });

    std::thread::sleep(RESUME_TEST_PAUSE_DURATION);

    network.resume_playback().expect("resume playback");
    let reanchored_start = wait_for_stream_start(&mut *listener);

    assert_eq!(
        reanchored_start.stream_id, original_start.stream_id,
        "a resume must re-anchor the same stream, not announce a new one -- a new stream_id \
         tears down and reopens the listener's audio engine on every resume"
    );
    let shift_ms = reanchored_start
        .host_start_time_ms
        .get()
        .saturating_sub(original_start.host_start_time_ms.get());
    assert!(
        shift_ms >= u64::try_from(RESUME_TEST_PAUSE_DURATION.as_millis()).expect("fits u64"),
        "the re-anchored StreamStart's host_start_time_ms must shift forward by at least the \
         real pause duration ({}ms), but only shifted by {shift_ms}ms -- every subsequent frame \
         would read as already late against the old anchor",
        RESUME_TEST_PAUSE_DURATION.as_millis(),
    );

    network.stop_playback().expect("stop playback");
    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Guards `BROADCAST_FRAME_QUEUE_CAPACITY` in `host_transport.rs`: the pump
/// deliberately bursts out an entire `SEND_AHEAD_HORIZON_MS` (1000ms, ~200
/// packets at the 5ms default) of already-packetized audio with no pacing
/// at all at stream start, so the bounded broadcast queue it feeds must be
/// sized to absorb that whole burst, not just "a momentary stall" -- a
/// queue too small for the burst it is guaranteed to receive drops frames
/// on every single stream start. Confirmed on a real device (LG G6,
/// 2026-08-09) as audible cracking/popping/static right at the beginning
/// of every stream, reported by a human listener, with `queue_overflows`
/// climbing from 0 to 59 in the first 15 seconds and then staying exactly
/// flat for the rest of the run -- i.e. concentrated entirely in the
/// opening burst, not spread across steady-state playback.
#[test]
fn the_opening_burst_does_not_overflow_the_broadcast_queue() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; opening-burst coverage remains \
             deterministic"
        );
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
    // Draining real frames (rather than only sleeping) matters here: an
    // idle listener that never reads its socket could itself become the
    // bottleneck this test is trying to rule out on the *sender* side.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let _ = listener.recv_event(Duration::from_millis(50));
    }

    let active = network
        .active_host_session()
        .expect("network state")
        .expect("active host session");
    assert_eq!(
        active.broadcast.queue_overflows, 0,
        "the opening send-ahead-horizon burst overflowed the broadcast queue \
         (attempted={}, queue_peak={})",
        active.broadcast.frames_attempted, active.broadcast.queue_peak_depth,
    );

    network.stop_playback().expect("stop playback");
    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

/// Guards the send-ahead horizon fix in `playback_streamer.rs`: the pump
/// used to pace strictly one packet per `packet_duration_ms` real
/// milliseconds, so a whole short source's worth of packets took roughly
/// `(packet_count - 1) * packet_duration_ms` of real time to arrive. Since
/// `pcm_wav()` is only 100ms (5 packets at 20ms each), the old pacing
/// guaranteed at least 80ms between the first and last packet. The fix lets
/// the pump burst out everything already within the send-ahead horizon
/// immediately, so all 5 packets of this short source should arrive far
/// faster than that.
#[test]
fn desktop_host_bursts_a_short_source_instead_of_pacing_one_packet_per_tick() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; streaming playback coverage remains deterministic"
        );
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

    let first_audio = wait_for_audio(&mut *listener);
    let burst_start = Instant::now();
    let mut last_sequence = first_audio.sequence.get();
    let mut packet_count = 1;
    let mut last_packet_at = burst_start;
    // This 100ms/5-packet source reaches natural end-of-file almost
    // immediately once burst-sent, so the real `Stop` broadcast (from the
    // pump's own natural-EOF exit path, not from an explicit stop_playback()
    // call below) can arrive interleaved with these remaining audio frames --
    // watch for it here instead of discarding it, or a later explicit wait
    // for it would time out waiting for a second one that never comes. Audio
    // and control are separate channels with no cross-channel ordering
    // guarantee, so keep draining both until quiescent rather than stopping
    // as soon as `Stop` is seen, which could cut off a still-in-flight
    // audio frame.
    let mut saw_stop = false;
    loop {
        match listener.recv_event(Duration::from_millis(60)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(datagram),
                ..
            }) => {
                assert!(
                    datagram.sequence.get() > last_sequence,
                    "audio sequence must strictly increase"
                );
                last_sequence = datagram.sequence.get();
                packet_count += 1;
                last_packet_at = Instant::now();
            }
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Stop(_)),
                ..
            }) => {
                saw_stop = true;
            }
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout =>
            {
                break;
            }
            Err(error) => panic!("listener transport failed: {error}"),
        }
    }
    let burst_elapsed = last_packet_at - burst_start;
    assert!(
        packet_count >= 5,
        "expected all 5 packets of the 100ms test source, got {packet_count}"
    );
    assert!(
        burst_elapsed < Duration::from_millis(60),
        "remaining packets after the first took {burst_elapsed:?}; the old \
         one-packet-per-tick pacing would need at least ~80ms (4 gaps * 20ms) \
         for this 100ms source -- the send-ahead horizon should burst them \
         out far faster than that"
    );

    // The pump may have already exited and broadcast `Stop` on its own
    // (natural EOF, caught above) before this call ever runs; `stop_playback`
    // is still safe and necessary to clear the network layer's playback slot
    // either way, but only wait for a fresh `Stop` if we haven't seen one yet.
    network.stop_playback().expect("stop playback");
    if !saw_stop {
        wait_for_control(&mut *listener, |message| {
            matches!(message, ControlMessage::Stop(_))
        });
    }

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

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

fn stage_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    stage_wav_source(temp, "desktop-block-playback-source", pcm_wav())
}

/// Comfortably longer than [`SEND_AHEAD_HORIZON_MS`]-worth of playback (via
/// `playback_streamer::SEND_AHEAD_HORIZON_MS`, not directly importable from
/// this test module) so a mid-stream check (a sync request/response
/// round trip, an explicit `stop_playback()`) genuinely happens before
/// natural end-of-file, unlike the short `stage_source` fixture, which now
/// bursts out entirely and reaches natural EOF almost immediately.
fn stage_long_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    stage_wav_source(temp, "desktop-block-playback-long-source", long_pcm_wav())
}

fn stage_wav_source(
    temp: &TempDir,
    source_id: &str,
    wav_bytes: Vec<u8>,
) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    let source_path = temp.path().join("source.wav");
    fs::write(&source_path, wav_bytes).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(source_id, "source.wav", Some(byte_length), None)
        .expect("descriptor");
    let registry = SelectedSourceRegistry::new();
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register staged source");
    (descriptor, registry)
}

/// Drives a real actor through role selection, host draft, and session
/// creation, then binds a real desktop host transport on the given local
/// interface address and completes the advertising handshake.
#[allow(clippy::type_complexity)]
fn start_host_session(
    descriptor: AudioSourceDescriptor,
    interface_name: String,
    interface_index: u32,
    address: Ipv4Addr,
) -> (
    CoreActorRuntime,
    silent_disco_core::runtime::CoreActorHandle,
    Receiver<CoreNotification>,
    silent_disco_core::runtime::SessionAdvertisement,
    Arc<DesktopHostNetworkControl>,
    silent_disco_core::runtime::NetworkEndpoint,
) {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-playback-host").expect("host id")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    next_snapshot(&receiver, 0);
    submit(
        &handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(&receiver, 1);
    submit(
        &handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Streaming playback host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(&receiver, 2);
    submit(&handle, 2, CoreCommand::CreateHostSession);
    next_snapshot(&receiver, 3);
    let advertisement_effect = next_effect(&receiver);
    let PlatformEffectRequest::StartAdvertising(advertisement) = advertisement_effect.request
    else {
        panic!("expected start advertising effect");
    };

    let network = Arc::new(DesktopHostNetworkControl::with_components(
        Arc::new(FixedInterfaceProvider::new(
            interface_name,
            interface_index,
            address,
        )),
        Arc::new(production_transport_factory()),
        TestHostPorts::default(),
    ));
    let endpoint = network
        .start_host(&advertisement, handle.clone())
        .expect("start desktop host transport");
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertisement_effect.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising completion");
    next_snapshot(&receiver, 4);
    (actor, handle, receiver, advertisement, network, endpoint)
}

/// Connects a real loopback-bound listener, joins, and drives the approval
/// through a real `CoreCommand::ApproveJoin` -> `TransportEffect` ->
/// `authorize_peer_ports` round trip -- exercising the same wiring a real
/// listener's ports depend on before any sync/audio frame can reach it.
fn join_and_approve_listener(
    address: Ipv4Addr,
    endpoint: silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
    handle: &silent_disco_core::runtime::CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    network: &DesktopHostNetworkControl,
) -> Box<dyn ListenerTransportNode> {
    let listener_id = DeviceId::new("desktop-playback-listener").expect("listener id");
    let mut listener = production_transport_factory()
        .connect_listener(
            ListenerTransportConfig {
                local_address: IpAddr::V4(address),
                ..ListenerTransportConfig::loopback(
                    advertisement.session_id.clone(),
                    listener_id.clone(),
                    endpoint,
                )
            },
            Arc::new(SystemTransportClock::default()),
        )
        .expect("listener connects to desktop host");
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: advertisement.session_id.clone(),
            device: DeviceIdentity {
                device_id: listener_id.clone(),
                display_name: "Streaming listener".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("send join request");
    wait_for_hello(&mut *listener);

    let joined = wait_snapshot(handle, |snapshot| {
        !snapshot.pending_join_requests.is_empty()
    });
    let request_id = joined.pending_join_requests[0].request_id.clone();
    submit(
        handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    listener
}

/// Not part of the automated suite: binds a real desktop host on this
/// machine's real LAN address, prints a real connection payload, and waits
/// for an actual external listener (e.g. a phone on the same Wi-Fi network,
/// pasting the printed payload into the app's "Connect manually" screen) to
/// join before streaming a first long "song" (an ascending C major scale),
/// exercising a mid-song pause/resume, then switching mid-session to a
/// second, audibly distinct "song" (a descending scale) -- the same
/// stop -> update draft -> start sequence a real user changing tracks would
/// trigger, including a fresh stream ID for the second song. Covers Block
/// 28.1's full one-listener checklist for the WAV container. Run explicitly
/// with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_plays_a_song_change -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_a_song_change() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor_a = stage_melody_source(
        &temp,
        &registry,
        "song-a",
        &C_MAJOR_SCALE_HZ,
        1.0,
        40,
        AudioContainer::Wav,
    );
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor_a, interface_name, interface_index, address);

    print_connection_payload(address, &endpoint, &advertisement);
    wait_for_real_join_and_approve(&handle, &receiver, &network);

    eprintln!(
        "=== song 1/2: \"song-a\", an ascending C major scale (do re mi fa so la ti do) -- starting playback ==="
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, "song-a-started");
    eprintln!("song-a playing for 15s...");
    std::thread::sleep(Duration::from_secs(15));

    exercise_pause_resume(&handle, &network, "song-a");

    eprintln!("song-a playing for 20 more seconds...");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, "song-a-before-stop");

    eprintln!("=== switching songs: stopping song-a ===");
    network.stop_playback().expect("stop playback");
    wait_snapshot_for(
        &handle,
        |snapshot| snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped,
        MANUAL_TEST_TIMEOUT,
    );

    let descending_scale: Vec<f64> = C_MAJOR_SCALE_HZ.iter().rev().copied().collect();
    let descriptor_b = stage_melody_source(
        &temp,
        &registry,
        "song-b",
        &descending_scale,
        1.0,
        40,
        AudioContainer::Wav,
    );
    let current = handle.current_snapshot().expect("current snapshot");
    submit(
        &handle,
        current.revision.get(),
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor_b.clone()),
            remember_approved_devices: None,
        }),
    );
    // `wait_snapshot`'s fast `TEST_TIMEOUT` (10s) is tuned for the loopback
    // suite; a real device's slower, more congested actor can take longer
    // than that to reflect a draft update, confirmed here (not assumed) by
    // a run against the LG G6 timing out at this exact call with the
    // update still pending -- `wait_snapshot_for`'s longer
    // `MANUAL_TEST_TIMEOUT` is what every other manual-test wait in this
    // file already uses.
    wait_snapshot_for(
        &handle,
        |snapshot| {
            snapshot
                .host_draft
                .audio_source
                .as_ref()
                .is_some_and(|source| source.source_id == descriptor_b.source_id)
        },
        MANUAL_TEST_TIMEOUT,
    );

    eprintln!(
        "=== song 2/2: \"song-b\", a descending C major scale (do ti la so fa mi re do) -- starting playback ==="
    );
    start_playback::start(&handle, &network, &registry).expect("start playback");
    eprintln!("song-b playing for 40s...");
    std::thread::sleep(Duration::from_secs(40));
    print_diagnostics(&handle, &network, "song-b-before-stop");

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

/// FLAC variant of the Block 28.1 one-listener checklist (join, approve,
/// start, pause/resume, stop, diagnostics) -- a single melody rather than a
/// song change, since format decoding (not track-switching, already proven
/// by the WAV variant) is what this run exists to prove. Requires `ffmpeg`
/// on `PATH` to encode the fixture; see [`encode_with_ffmpeg`] for why the
/// desktop app itself cannot produce this file. Run explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_plays_flac -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_flac() {
    run_manual_single_format_session("flac-song", AudioContainer::Flac, "FLAC");
}

/// MP3 variant of the Block 28.1 one-listener checklist -- see
/// [`manual_real_android_listener_plays_flac`]. Run explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener_plays_mp3 -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_mp3() {
    run_manual_single_format_session("mp3-song", AudioContainer::Mp3, "MP3");
}

fn run_manual_single_format_session(source_id: &str, container: AudioContainer, label: &str) {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor = stage_melody_source(
        &temp,
        &registry,
        source_id,
        &C_MAJOR_SCALE_HZ,
        1.0,
        40,
        container,
    );
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    eprintln!("=== {label} manual device test ===");
    print_connection_payload(address, &endpoint, &advertisement);
    wait_for_real_join_and_approve(&handle, &receiver, &network);

    eprintln!("=== starting {label} playback: ascending C major scale ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, &format!("{label}-started"));
    eprintln!("playing for 15s...");
    std::thread::sleep(Duration::from_secs(15));

    exercise_pause_resume(&handle, &network, label);

    eprintln!("playing for 20 more seconds...");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, &format!("{label}-before-stop"));

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    wait_snapshot_for(
        &handle,
        |snapshot| snapshot.playback_state == PlaybackState::Stopped,
        MANUAL_TEST_TIMEOUT,
    );
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("{label} done.");
}

/// Fully automated, no human required: two real Android emulators, driven
/// entirely via `adb`/`uiautomator` (see [`automate_manual_connect`]), join
/// and are approved into the same desktop-hosted session and hear the same
/// stream. This is the project's actual success criterion -- listeners
/// hearing the same audio in sync, plural -- which has never been
/// exercised even against real hardware (see `memory.md`). An emulator is
/// not a substitute for the physical-device acceptance criteria Block 29
/// still needs, but this proves the desktop host's multi-listener path
/// (admission, broadcast fan-out, per-listener sync, delivery accounting)
/// against two genuinely independent OS processes/network stacks instead
/// of the automated suite's single in-process loopback listener.
///
/// Requires two already-running emulators with the app already installed,
/// reachable via the given `adb` serials, and `ffmpeg`-free (WAV only, to
/// keep the two-listener join timing predictable) -- override the serials
/// below if your local setup differs. Run explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_two_emulator_listeners_play_together -- --ignored --nocapture`
#[test]
#[ignore = "requires two running Android emulators with the app installed, driven via adb"]
fn manual_two_emulator_listeners_play_together() {
    const FIRST_SERIAL: &str = "emulator-5554";
    const SECOND_SERIAL: &str = "emulator-5556";

    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor = stage_melody_source(
        &temp,
        &registry,
        "two-listener-song",
        &C_MAJOR_SCALE_HZ,
        1.0,
        60,
        AudioContainer::Wav,
    );
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);
    let payload = connection_payload_json(address, &endpoint, &advertisement);
    eprintln!("connection payload: {payload}");

    eprintln!("=== driving {FIRST_SERIAL} through Connect manually ===");
    automate_manual_connect(FIRST_SERIAL, &payload);
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    let after_first = handle.current_snapshot().expect("current snapshot");
    eprintln!(
        "=== {FIRST_SERIAL} approved -- listeners now: {:?} ===",
        after_first
            .listeners
            .iter()
            .map(|l| l.device_id.as_str().to_owned())
            .collect::<Vec<_>>()
    );

    eprintln!("=== driving {SECOND_SERIAL} through Connect manually ===");
    automate_manual_connect(SECOND_SERIAL, &payload);
    wait_for_real_join_and_approve(&handle, &receiver, &network);
    let after_second = handle.current_snapshot().expect("current snapshot");
    eprintln!(
        "=== {SECOND_SERIAL} approved -- listeners now: {:?} ===",
        after_second
            .listeners
            .iter()
            .map(|l| l.device_id.as_str().to_owned())
            .collect::<Vec<_>>()
    );

    let ready = handle.current_snapshot().expect("current snapshot");
    assert_eq!(
        ready.listeners.len(),
        2,
        "expected exactly two connected listeners, got {}",
        ready.listeners.len()
    );

    eprintln!("=== starting playback for both listeners ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    print_diagnostics(&handle, &network, "two-listeners-started");
    eprintln!("playing for 20s...");
    std::thread::sleep(Duration::from_secs(20));
    print_diagnostics(&handle, &network, "two-listeners-mid");

    exercise_pause_resume(&handle, &network, "two-listeners");

    eprintln!("playing for 20 more seconds...");
    std::thread::sleep(Duration::from_secs(20));
    let final_snapshot = print_diagnostics(&handle, &network, "two-listeners-before-stop");
    assert_eq!(
        final_snapshot.listeners.len(),
        2,
        "a listener silently dropped out during playback"
    );

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    wait_snapshot_for(
        &handle,
        |snapshot| snapshot.playback_state == PlaybackState::Stopped,
        MANUAL_TEST_TIMEOUT,
    );
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

fn connection_payload_json(
    address: Ipv4Addr,
    endpoint: &silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
) -> String {
    format!(
        "{{\"hostAddress\":\"{address}\",\"controlPort\":{},\"syncPort\":{},\"audioPort\":{},\"sessionId\":\"{}\",\"protocolVersion\":{},\"inviteCodeRequired\":false,\"expiresAtMs\":null}}",
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        advertisement.session_id.as_str(),
        advertisement.protocol_version,
    )
}

fn print_connection_payload(
    address: Ipv4Addr,
    endpoint: &silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
) {
    eprintln!(
        "=== paste this connection payload into the Android app's Connect manually screen ==="
    );
    eprintln!(
        "{}",
        connection_payload_json(address, endpoint, advertisement)
    );
}

/// Backslash-escapes the JSON special characters `adb shell input text`
/// otherwise silently drops. Confirmed empirically: an unescaped payload
/// arrived in the app's `EditText` missing every `{`, `}`, `"`, and `,`,
/// which the app then correctly rejected as invalid JSON -- a good example
/// of the app doing the right thing with bad input, but useless for driving
/// it automatically. Escaping fixes the input path, not the app.
fn escape_for_adb_input_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if matches!(ch, '{' | '}' | '"' | ',') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Drives the app's "Connect manually" flow on a real Android
/// emulator/device via `adb` + `uiautomator dump` text lookup -- not
/// hardcoded coordinates, so it tolerates whichever permission dialogs
/// happen to already be granted on `serial` (first-run location/nearby-
/// device prompts vs. an emulator that already granted them). Manual-test-
/// only external dependency, same pattern as [`encode_with_ffmpeg`]:
/// requires `adb` and `uiautomator` (bundled with the Android SDK) on
/// `PATH`, and the app already installed on `serial`. Panics loudly on
/// failure (missing `adb`, or the automation script itself exiting
/// non-zero) rather than silently giving up partway through.
fn automate_manual_connect(serial: &str, payload_json: &str) {
    let escaped = escape_for_adb_input_text(payload_json);
    let status = Command::new("bash")
        .arg("-c")
        .arg(AUTOMATE_CONNECT_SCRIPT)
        .arg("automate_connect") // becomes $0 inside the script
        .arg(serial)
        .arg(&escaped)
        .status();
    let status = match status {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => panic!(
            "bash is not on PATH -- required to run the adb UI-automation script for {serial}"
        ),
        Err(error) => panic!("failed to run adb UI-automation script for {serial}: {error}"),
    };
    assert!(
        status.success(),
        "adb UI automation failed for {serial} -- see the script's own output above for which \
         step failed (adb/uiautomator on PATH? app installed on {serial}?)"
    );
}

/// Reusable across every manual multi-device test: force-stops and
/// relaunches the app, navigates role selection -> "Find a session",
/// tolerantly taps through whichever of the nearby-access/location/nearby-
/// devices dialogs actually appear (in order, skipping any that don't),
/// reaches "Connect manually", types the escaped payload, dismisses the
/// keyboard (it otherwise visually covers the Connect button -- confirmed
/// empirically, not a guess), and taps Connect.
const AUTOMATE_CONNECT_SCRIPT: &str = r#"
set -euo pipefail
SERIAL="$1"
PAYLOAD="$2"
DUMP="$(mktemp)"
trap 'rm -f "$DUMP"' EXIT

dump_ui() {
  adb -s "$SERIAL" shell uiautomator dump /sdcard/window_dump.xml >/dev/null 2>&1
  adb -s "$SERIAL" pull /sdcard/window_dump.xml "$DUMP" >/dev/null 2>&1
}

tap_exact_text() {
  local target="$1"
  local bounds
  bounds=$(grep -o "text=\"$target\"[^>]*bounds=\"\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]\"" "$DUMP" \
    | grep -o '\[[0-9]*,[0-9]*\]\[[0-9]*,[0-9]*\]' | head -1) || true
  if [ -z "$bounds" ]; then
    return 1
  fi
  local x1 y1 x2 y2
  x1=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\1/')
  y1=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\2/')
  x2=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\3/')
  y2=$(echo "$bounds" | sed -E 's/\[([0-9]+),([0-9]+)\]\[([0-9]+),([0-9]+)\]/\4/')
  local cx=$(( (x1 + x2) / 2 ))
  local cy=$(( (y1 + y2) / 2 ))
  echo "  [$SERIAL] tapping \"$target\" at $cx,$cy"
  adb -s "$SERIAL" shell input tap "$cx" "$cy"
  return 0
}

try_tap_any() {
  local deadline=$((SECONDS + 10))
  while [ $SECONDS -lt $deadline ]; do
    dump_ui
    for candidate in "$@"; do
      if tap_exact_text "$candidate"; then
        sleep 1
        return 0
      fi
    done
    sleep 1
  done
  echo "  [$SERIAL] none of [$*] appeared within 10s; continuing"
  return 0
}

echo "[$SERIAL] relaunching app"
adb -s "$SERIAL" shell am force-stop com.ekkus.silentdisco
adb -s "$SERIAL" shell monkey -p com.ekkus.silentdisco -c android.intent.category.LAUNCHER 1 >/dev/null
sleep 3

echo "[$SERIAL] role screen -> Find a session"
try_tap_any "Find a session"
echo "[$SERIAL] optional nearby-access app dialog"
try_tap_any "Continue"
echo "[$SERIAL] optional system location dialog"
try_tap_any "While using the app"
echo "[$SERIAL] optional system nearby-devices dialog"
try_tap_any "Allow"
echo "[$SERIAL] nearby-session screen -> Connect manually"
try_tap_any "Connect manually"

dump_ui
echo "[$SERIAL] tapping payload field"
tap_exact_text "Connection payload" || echo "  [$SERIAL] payload field not found by exact text; trying anyway"
sleep 1
adb -s "$SERIAL" shell input text "$PAYLOAD"
sleep 1
adb -s "$SERIAL" shell input keyevent KEYCODE_BACK
sleep 1

echo "[$SERIAL] waiting for the payload validation summary to render"
deadline=$((SECONDS + 10))
while [ $SECONDS -lt $deadline ]; do
  dump_ui
  if grep -q 'text="Protocol version: ' "$DUMP"; then
    break
  fi
  sleep 1
done

echo "[$SERIAL] tapping Connect"
tap_exact_text "Connect" || { echo "[$SERIAL] Connect button not found"; exit 1; }
sleep 2
echo "[$SERIAL] automation done"
"#;

/// Polls for a real external device's join request (unlike the automated
/// suite's `join_and_approve_listener`, which connects a loopback listener
/// itself) and approves it. Shared by every manual device test in this
/// module.
fn wait_for_real_join_and_approve(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    network: &DesktopHostNetworkControl,
) {
    eprintln!("waiting up to 8 minutes for a real join request...");
    let deadline = Instant::now() + Duration::from_mins(8);
    let joined = loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if !snapshot.pending_join_requests.is_empty() {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a real join request"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let request = &joined.pending_join_requests[0];
    eprintln!(
        "real join request received from device_id={} display_name={}",
        request.device_id.as_str(),
        request.display_name
    );
    let request_id = request.request_id.clone();
    submit(
        handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");

    // `dispatch_transport_effect` only enqueues the send onto the transport
    // worker and returns -- it does not block until the real delivery
    // completes. The worker reports that back asynchronously as a
    // `TransportEvent::DeliveryCompleted` fact, which is what actually
    // moves this device from `pending_join_requests` into `listeners`
    // (`record_approved_listener`). Returning as soon as dispatch merely
    // *queues* the send, without waiting for that fact to land, is exactly
    // the gap that let two real listeners both report "approved" while the
    // snapshot briefly had zero, then one, connected listener -- confirmed
    // empirically against two real Android emulators before this fix.
    let device_id = request.device_id.clone();
    wait_snapshot_for(
        handle,
        |snapshot| snapshot.listeners.iter().any(|l| l.device_id == device_id),
        MANUAL_TEST_TIMEOUT,
    );
    eprintln!("approved and authorized.");
}

/// Pauses playback, holds it long enough for a human to notice the silence,
/// resumes, and prints diagnostics at each step. Block 28.1's "exercise
/// pause/resume/stop" checklist item, safe now that this same session's
/// Block 27.3 work fixed both the duplicate-Start and duplicate-Resume
/// bugs it found while implementing that checklist item on the automated
/// side.
fn exercise_pause_resume(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    network: &DesktopHostNetworkControl,
    label: &str,
) {
    eprintln!("=== pausing -- you should hear the audio stop ===");
    network.pause_playback().expect("pause playback");
    wait_snapshot_for(
        handle,
        |snapshot| snapshot.playback_state == PlaybackState::Paused,
        MANUAL_TEST_TIMEOUT,
    );
    print_diagnostics(handle, network, &format!("{label}-paused"));
    std::thread::sleep(Duration::from_secs(1));

    eprintln!("=== resuming -- audio should continue from where it paused, not restart ===");
    network.resume_playback().expect("resume playback");
    wait_snapshot_for(
        handle,
        |snapshot| snapshot.playback_state == PlaybackState::Playing,
        MANUAL_TEST_TIMEOUT,
    );
    print_diagnostics(handle, network, &format!("{label}-resumed"));
}

/// Prints everything the desktop host itself can observe for Block 28.1's
/// "record sync, RTT, packet-loss, and underrun diagnostics" checklist
/// item. Packet loss, underruns, and concealment are observed on the
/// *listener* (the Android device), not the host -- this only prints what
/// the host side actually knows (sync offset/RTT/confidence per listener,
/// plus broadcast delivery and queue pressure from Block 26.3); read the
/// rest from the Android app's own diagnostics screen.
fn print_diagnostics(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    network: &DesktopHostNetworkControl,
    label: &str,
) -> CoreSnapshot {
    let snapshot = handle.current_snapshot().expect("current snapshot");
    for listener in &snapshot.listeners {
        match listener.synchronization {
            Some(sync) => eprintln!(
                "[{label}] listener={} sync confidence={:?} offset_ms={:.2} rtt_ms={:.2} \
                 drift_ppm={:.2}",
                listener.display_name,
                sync.confidence,
                sync.offset_ms,
                sync.round_trip_ms,
                sync.drift_ppm,
            ),
            None => eprintln!(
                "[{label}] listener={} has not yet completed a sync exchange",
                listener.display_name
            ),
        }
    }
    if let Ok(Some(active)) = network.active_host_session() {
        let broadcast = active.broadcast;
        eprintln!(
            "[{label}] broadcast: attempted={} fully_delivered={} partially_delivered={} \
             without_recipients={} queue_depth={} queue_peak={} queue_overflows={}",
            broadcast.frames_attempted,
            broadcast.frames_fully_delivered,
            broadcast.frames_partially_delivered,
            broadcast.frames_without_recipients,
            broadcast.queue_depth,
            broadcast.queue_peak_depth,
            broadcast.queue_overflows,
        );
    }
    eprintln!(
        "[{label}] packet loss / underrun / concealment are listener-side -- read them from the \
         Android app's own diagnostics screen, not from this desktop log."
    );
    snapshot
}

/// The eight notes of an ascending C major scale ("do re mi fa so la ti
/// do"), a sequence a listener can recognize and judge for smoothness by
/// ear far more easily than one sustained tone -- a dropped, repeated, or
/// glitched note is unmistakable, where a gap in a continuous tone can
/// blend into the tone's own texture.
/// Linear fade-in/fade-out applied at each note boundary in
/// [`melody_pcm_wav`], long enough to remove the phase-reset amplitude
/// discontinuity without being long enough to noticeably shorten the note.
const NOTE_FADE_SECONDS: f64 = 0.005;

const C_MAJOR_SCALE_HZ: [f64; 8] = [
    261.63, // C4
    293.66, // D4
    329.63, // E4
    349.23, // F4
    392.00, // G4
    440.00, // A4
    493.88, // B4
    523.25, // C5
];

fn stage_melody_source(
    temp: &TempDir,
    registry: &SelectedSourceRegistry,
    source_id: &str,
    notes_hz: &[f64],
    note_seconds: f64,
    total_seconds: u32,
    container: AudioContainer,
) -> AudioSourceDescriptor {
    let wav_bytes = melody_pcm_wav(notes_hz, note_seconds, total_seconds);
    let (extension, bytes) = match container {
        AudioContainer::Wav => ("wav", wav_bytes),
        AudioContainer::Flac => ("flac", encode_with_ffmpeg(&wav_bytes, "flac")),
        AudioContainer::Mp3 => ("mp3", encode_with_ffmpeg(&wav_bytes, "mp3")),
    };
    let source_path = temp.path().join(format!("{source_id}.{extension}"));
    fs::write(&source_path, &bytes).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(
        format!("desktop-block-playback-manual-{source_id}"),
        format!("{source_id}.{extension}"),
        Some(byte_length),
        None,
    )
    .expect("descriptor");
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            container,
        ))
        .expect("register staged source");
    descriptor
}

/// Encodes PCM WAV bytes to the given container via a real `ffmpeg`
/// subprocess. The desktop app only ever *decodes* audio (`symphonia` has
/// no encoder -- see `docs/DESKTOP_BLOCK18_DECODER_DECISION.md`), so
/// producing a FLAC/MP3 fixture long enough for a human to judge by ear
/// needs a real external encoder. Manual-test-only dependency: never used
/// by the automated suite, and this panics with a clear message rather than
/// silently skipping if `ffmpeg` is not on `PATH`.
fn encode_with_ffmpeg(wav_bytes: &[u8], extension: &str) -> Vec<u8> {
    let codec = match extension {
        "flac" => "flac",
        "mp3" => "libmp3lame",
        other => panic!("no ffmpeg codec mapped for extension: {other}"),
    };
    let temp = TempDir::new().expect("ffmpeg temp dir");
    let input_path = temp.path().join("input.wav");
    fs::write(&input_path, wav_bytes).expect("write ffmpeg input");
    let output_path = temp.path().join(format!("output.{extension}"));

    let result = Command::new("ffmpeg")
        .arg("-y")
        .args(["-loglevel", "error"])
        .arg("-i")
        .arg(&input_path)
        .args(["-c:a", codec])
        .arg(&output_path)
        .output();
    let output = match result {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => panic!(
            "ffmpeg is not on PATH -- install it to run the {extension} variant of this manual \
             device test (the desktop app only decodes audio, so a real external encoder is \
             needed to produce a fixture long enough to judge by ear)"
        ),
        Err(error) => panic!("failed to run ffmpeg: {error}"),
    };
    assert!(
        output.status.success(),
        "ffmpeg failed to encode {extension}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read(&output_path).expect("read ffmpeg output")
}

/// Cycles through `notes_hz`, holding each for `note_seconds`, until
/// `total_seconds` of audio have been generated. Each note restarts its sine
/// wave at phase zero rather than gliding from the previous note, so a
/// listener hears a clean, deliberate transition, not a bend.
fn melody_pcm_wav(notes_hz: &[f64], note_seconds: f64, total_seconds: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let frame_count = sample_rate * total_seconds;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames_per_note = (f64::from(sample_rate) * note_seconds) as u32;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    // Each note restarts its sine phase at 0, and the previous note almost
    // never ends on a zero-crossing -- without a fade, that's a real,
    // audible click at every note boundary baked into this fixture's own
    // audio, indistinguishable from a genuine playback-pipeline defect.
    // A short linear fade-in/fade-out at each note's edges removes that
    // amplitude discontinuity so any clicks heard on a real device are
    // attributable to the playback pipeline, not this synthetic source.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fade_frames = (f64::from(sample_rate) * NOTE_FADE_SECONDS) as u32;
    for index in 0..frame_count {
        let note_index =
            usize::try_from(index / frames_per_note).expect("note index") % notes_hz.len();
        let frequency_hz = notes_hz[note_index];
        let index_within_note = index % frames_per_note;
        let time_within_note = f64::from(index_within_note) / f64::from(sample_rate);
        let envelope = if index_within_note < fade_frames {
            f64::from(index_within_note) / f64::from(fade_frames)
        } else if index_within_note >= frames_per_note - fade_frames {
            f64::from(frames_per_note - index_within_note) / f64::from(fade_frames)
        } else {
            1.0
        };
        let sample =
            (time_within_note * frequency_hz * std::f64::consts::TAU).sin() * 12_000.0 * envelope;
        #[allow(clippy::cast_possible_truncation)]
        let sample = sample as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn pcm_wav() -> Vec<u8> {
    square_wave_pcm_wav(4_410)
}

/// 3 real seconds -- comfortably longer than the playback pump's send-ahead
/// horizon, so playback is still genuinely running (not yet at natural
/// end-of-file) by the time a mid-stream check runs.
fn long_pcm_wav() -> Vec<u8> {
    square_wave_pcm_wav(44_100 * 3)
}

/// Truncated before the 44-byte WAV header is even complete, so the shared
/// decoder cannot parse it at all -- `StreamingDecodeHandle::open` fails
/// synchronously. See
/// `starting_playback_with_a_corrupt_source_fails_visibly_at_the_orchestration_level`.
fn corrupt_wav_bytes() -> Vec<u8> {
    let mut bytes = pcm_wav();
    bytes.truncate(20);
    bytes
}

/// A WAV whose header parses fine and declares 3 real seconds of data (via
/// `long_pcm_wav`), but whose body is cut to only ~0.1s of actual bytes --
/// the header's declared `data` chunk size is left untouched, so the shared
/// decoder only discovers the shortfall once it reads past the truncation
/// point. Confirmed empirically (see `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`
/// D1 for context): `StreamingDecodeHandle::open` succeeds and the failure
/// surfaces later as `DecodeErrorKind::CorruptInput` while draining chunks --
/// exactly the "host source read failure" this fixture exists to trigger.
/// See `a_host_source_read_failure_mid_stream_does_not_claim_continued_normal_streaming`.
fn truncated_body_full_header_wav() -> Vec<u8> {
    let mut bytes = long_pcm_wav();
    const WAV_HEADER_BYTES: usize = 44;
    const SURVIVING_DATA_BYTES: usize = 4_410 * 2; // ~0.1s at 44.1kHz mono 16-bit
    bytes.truncate(WAV_HEADER_BYTES + SURVIVING_DATA_BYTES);
    bytes
}

fn square_wave_pcm_wav(frame_count: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frame_count {
        let sample = if index % 64 < 32 {
            8_000_i16
        } else {
            -8_000_i16
        };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Finds a real, currently-active private-LAN IPv4 interface on this machine,
/// mirroring `network_tests.rs`'s bind-conflict test -- a real production
/// socket bind requires an address genuinely assigned to a local interface,
/// so this cannot be faked the way `network_tests.rs`'s simulated-transport
/// tests fake interface records.
/// The interface production would bind, not a re-derived guess.
///
/// This used to hand-roll the filter, which accepted more than production's
/// `classify` does -- notably container bridges, which are private, up, and
/// physical by every predicate used here. Requiring `interface.default` made
/// it pick the right one on this machine by luck rather than by rule; the
/// same filter without that clause, in `network_tests.rs`, picked a Docker
/// bridge often enough to fail three runs in four. Delegating means a device
/// test can never advertise an address a phone cannot reach.
fn real_private_lan_address() -> Option<(String, u32, Ipv4Addr)> {
    super::network::first_bindable_private_lan_address()
}

struct FixedInterfaceProvider {
    record: InterfaceRecord,
}

impl FixedInterfaceProvider {
    fn new(name: String, index: u32, address: Ipv4Addr) -> Self {
        Self {
            record: InterfaceRecord {
                name,
                index,
                up: true,
                running: true,
                oper_up: true,
                loopback: false,
                point_to_point: false,
                tun: false,
                physical: true,
                default_route: false,
                addresses: vec![AddressRecord {
                    address: IpAddr::V4(address),
                    prefix_length: 24,
                }],
            },
        }
    }
}

impl super::network::NetworkInterfaceProvider for FixedInterfaceProvider {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, super::network::DesktopNetworkError> {
        Ok(vec![self.record.clone()])
    }
}

fn submit(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    revision: u64,
    command: CoreCommand,
) {
    handle
        .submit_command(
            CoreCommandRequest::new(SnapshotRevision::new(revision), command).expect("command"),
        )
        .expect("submit command");
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot)) if snapshot.revision.get() >= minimum => {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for snapshot"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn next_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::PlatformEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn next_transport_effect(receiver: &Receiver<CoreNotification>) -> TransportEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for transport effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn wait_snapshot(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
) -> CoreSnapshot {
    wait_snapshot_for(handle, predicate, TEST_TIMEOUT)
}

/// As [`wait_snapshot`], but with an explicit timeout instead of the fast
/// loopback-test default. Real devices/emulators are measurably slower and
/// more congested than the in-process loopback listener the automated
/// suite uses -- a manual run against a real Android emulator observed
/// `queue_overflows=930` under sustained playback and a stop-transition
/// that took longer than `TEST_TIMEOUT` (10s) to land, even though it did
/// land moments later. Manual device tests should use a longer timeout
/// here rather than papering over that by loosening the fast tests' shared
/// constant.
fn wait_snapshot_for(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
    timeout: Duration,
) -> CoreSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for actor state; observed playback_state={:?} \
             host_lifecycle={:?} revision={} pending_joins={} listeners={}",
            snapshot.playback_state,
            snapshot.host_lifecycle,
            snapshot.revision.get(),
            snapshot.pending_join_requests.len(),
            snapshot.listeners.len(),
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_hello(listener: &mut dyn ListenerTransportNode) {
    wait_for_control(listener, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
}

fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    matches_expected: impl Fn(&ControlMessage) -> bool,
) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(message),
                ..
            }) if matches_expected(&message) => return,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for expected control message"
        );
    }
}

/// Like [`wait_for_control`], but returns the matched `StreamStart` payload
/// instead of discarding it -- callers that need to inspect
/// `host_start_time_ms` (e.g. confirming a resume's re-anchor actually
/// shifted it) can't do that with `wait_for_control` alone.
fn wait_for_stream_start(listener: &mut dyn ListenerTransportNode) -> StreamStart {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::StreamStart(start)),
                ..
            }) => return start,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a StreamStart control message"
        );
    }
}

fn wait_for_audio(
    listener: &mut dyn ListenerTransportNode,
) -> silent_disco_core::protocol::AudioDatagram {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(datagram),
                ..
            }) => return datagram,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for an audio datagram"
        );
    }
}

fn wait_for_sync_response(
    listener: &mut dyn ListenerTransportNode,
    correlation_id: u64,
) -> SyncResponse {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                frame: ProtocolFrame::SyncResponse(response),
                ..
            }) if response.correlation_id == correlation_id => return response,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a sync response"
        );
    }
}
