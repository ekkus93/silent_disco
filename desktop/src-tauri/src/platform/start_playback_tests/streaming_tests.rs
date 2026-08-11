//! Audio/sync delivery and pacing coverage: does a real listener actually
//! receive the stream, answer sync requests correctly, survive a pause/
//! resume re-anchor, and get the opening burst without overflowing the
//! broadcast queue.

use super::fixtures::{stage_long_source, stage_source};
use super::harness::{
    join_and_approve_listener, real_private_lan_address, start_host_session, wait_for_audio,
    wait_for_control, wait_for_stream_start, wait_for_sync_response, wait_snapshot,
};
use crate::platform::start_playback;
use silent_disco_core::domain::{MonotonicMillis, PlaybackState, SyncConfidence};
use silent_disco_core::protocol::{
    ControlMessage, ProtocolFrame, SyncRequest, SynchronizationReport,
};
use silent_disco_core::transport::{TransportChannel, TransportEvent};
use std::time::{Duration, Instant};
use tempfile::TempDir;

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
