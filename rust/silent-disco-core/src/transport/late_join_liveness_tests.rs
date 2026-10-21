//! Late-join replay and recurring liveness probes must coexist on the real
//! socket transport. A silent peer is evicted, but an active listener must not
//! lose its cached boundary while a slow playback/emulator test progresses.
use std::sync::Arc;

use crate::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest, SyncResponse, TrackBoundary};

use super::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame, wait_for_frame_from,
};
use super::{
    HostTransportConfig, ListenerTransportConfig, ManualTransportClock, SocketTransportFactory,
    TransportChannel, TransportClock, TransportFactory,
};

#[test]
fn late_joined_listener_keeps_replayed_boundary_while_sync_probes_refresh_liveness() {
    let session_id = id_session("late-join-live-session");
    let device_id = id_device("late-join-active-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let factory = SocketTransportFactory;
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host binds");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock.clone(),
        )
        .expect("listener connects");

    listener
        .send_control(&join_request(&session_id, &device_id, "Late listener"))
        .expect("join request");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = stream_start(&session_id, 1_000);
    let upcoming = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session_id.clone(),
        stream_id: StreamId::new("stream-1").expect("stream ID"),
        first_sample_index: SampleIndex::new(480),
        entry_id: "entry-next".to_owned(),
        display_name: "Next song".to_owned(),
    });
    // Both events precede authorization, as in production late-join replay.
    host.broadcast_control(&start).expect("cache stream start");
    host.broadcast_control(&upcoming).expect("cache boundary");
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize late listener");
    wait_for_authorized(&mut *host, &device_id);
    for expected in [start, upcoming] {
        wait_for_frame(&mut *listener, TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(expected.clone())
        });
    }

    // Cross 12 seconds of virtual host time (past the eight-second silence
    // threshold) while exercising the production two-second sync cadence.
    // Each probe is observed by the actual host socket worker before
    // advancing the clock again. Audio follows the replayed boundary without
    // an erroneous disconnect or a false "zero peers" delivery report.
    for round in 0..6_u64 {
        clock.advance(2_000);
        listener
            .send_sync_request(&SyncRequest {
                session_id: session_id.clone(),
                correlation_id: round + 1,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(clock.now().get()),
            })
            .expect("listener synchronization probe");
        wait_for_frame_from(
            &mut *host,
            TransportChannel::Synchronization,
            &device_id,
            |frame| {
                matches!(
                    frame,
                    ProtocolFrame::SyncRequest(request)
                        if request.correlation_id == round + 1
                )
            },
        );
        // Exercise the complete host-to-listener synchronization round trip,
        // not merely the inbound probe that refreshes host liveness.
        let response = sync_response(&session_id, round + 1, clock.now().get());
        let sync_delivery = host.broadcast_sync(&response).expect("host sync response");
        assert_eq!(sync_delivery.report.intended_peers, 1);
        assert_eq!(sync_delivery.report.successful_peers, 1);
        wait_for_frame(
            &mut *listener,
            TransportChannel::Synchronization,
            |received| received == &response,
        );
        let frame = audio_frame(&session_id, 240 + round);
        let sent = host.broadcast_audio(&frame).expect("host audio delivery");
        assert_eq!(sent.report.intended_peers, 1);
        assert_eq!(sent.report.successful_peers, 1);
        wait_for_frame(&mut *listener, TransportChannel::Audio, |received| {
            received == &frame
        });
    }

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn late_joined_probing_listener_survives_when_an_earlier_listener_goes_silent() {
    let session_id = id_session("late-join-mixed-liveness");
    let silent_id = id_device("late-join-silent-peer");
    let active_id = id_device("late-join-probing-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let factory = SocketTransportFactory;
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host binds");
    let mut silent = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                silent_id.clone(),
                host.endpoint(),
            ),
            clock.clone(),
        )
        .expect("first listener connects");
    silent
        .send_control(&join_request(&session_id, &silent_id, "Silent peer"))
        .expect("first join request");
    wait_for_control_from(&mut *host, &silent_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&silent_id, silent.local_routes())
        .expect("authorize first listener");
    wait_for_authorized(&mut *host, &silent_id);

    let start = stream_start(&session_id, 1_000);
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session_id.clone(),
        stream_id: StreamId::new("stream-1").expect("stream ID"),
        first_sample_index: SampleIndex::new(480),
        entry_id: "entry-next".to_owned(),
        display_name: "Next song".to_owned(),
    });
    host.broadcast_control(&start).expect("start stream");
    host.broadcast_control(&boundary)
        .expect("schedule boundary");

    // A second listener joins after both control events were broadcast.
    let mut active = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                active_id.clone(),
                host.endpoint(),
            ),
            clock.clone(),
        )
        .expect("late listener connects");
    active
        .send_control(&join_request(&session_id, &active_id, "Probing peer"))
        .expect("late join request");
    wait_for_control_from(&mut *host, &active_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&active_id, active.local_routes())
        .expect("authorize late listener");
    wait_for_authorized(&mut *host, &active_id);
    for expected in [start, boundary] {
        wait_for_frame(&mut *active, TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(expected.clone())
        });
    }

    assert_only_probing_late_joiner_survives(
        &mut *host,
        &mut *active,
        &session_id,
        &active_id,
        &clock,
    );

    active.shutdown().expect("active listener shutdown");
    silent.shutdown().expect("silent listener shutdown");
    host.shutdown().expect("host shutdown");
}

fn assert_only_probing_late_joiner_survives(
    host: &mut dyn super::HostTransportNode,
    active: &mut dyn super::ListenerTransportNode,
    session_id: &crate::domain::SessionId,
    active_id: &crate::domain::DeviceId,
    clock: &ManualTransportClock,
) {
    // The first peer never probes. Only the late joiner sends real socket
    // synchronization requests, which must refresh its own liveness without
    // accidentally keeping the silent peer authorized.
    for round in 0..6_u64 {
        clock.advance(2_000);
        active
            .send_sync_request(&SyncRequest {
                session_id: session_id.clone(),
                correlation_id: round + 1,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(clock.now().get()),
            })
            .expect("active peer sync request");
        wait_for_frame_from(
            &mut *host,
            TransportChannel::Synchronization,
            active_id,
            |frame| {
                matches!(
                    frame,
                    ProtocolFrame::SyncRequest(request)
                        if request.correlation_id == round + 1
                )
            },
        );
        // The active late joiner must receive the reply even while the
        // earlier peer is being evicted for silence.
        let response = sync_response(session_id, round + 1, clock.now().get());
        let sync_delivery = host.broadcast_sync(&response).expect("host sync response");
        assert!(sync_delivery.report.successful_peers >= 1);
        wait_for_frame(active, TransportChannel::Synchronization, |received| {
            received == &response
        });
        let frame = audio_frame(session_id, 300 + round);
        let delivery = host.broadcast_audio(&frame).expect("broadcast audio");
        if round < 3 {
            assert_eq!(delivery.report.intended_peers, 2);
            assert_eq!(delivery.report.successful_peers, 2);
        } else if round >= 4 {
            assert_eq!(
                delivery.report.intended_peers, 1,
                "only the actively probing late joiner should remain"
            );
            assert_eq!(delivery.report.successful_peers, 1);
        }
        wait_for_frame(&mut *active, TransportChannel::Audio, |received| {
            received == &frame
        });
    }
}

fn sync_response(session_id: &SessionId, correlation_id: u64, now_ms: u64) -> ProtocolFrame {
    ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session_id.clone(),
        correlation_id,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(now_ms),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(now_ms),
        t3_host_send_elapsed_ms: MonotonicMillis::new(now_ms),
    })
}

#[test]
fn late_joiner_receives_replayed_start_before_immediately_broadcast_boundary() {
    use std::time::{Duration, Instant};

    use crate::domain::StreamId;
    use crate::protocol::TrackBoundary;

    let session_id = id_session("late-join-control-order");
    let device_id = id_device("late-join-control-order-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let factory = SocketTransportFactory;
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host binds");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Late listener"))
        .expect("join request");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = stream_start(&session_id, 1_000);
    let next = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session_id.clone(),
        stream_id: StreamId::new("stream-1").expect("stream ID"),
        first_sample_index: SampleIndex::new(480),
        entry_id: "next-entry".to_owned(),
        display_name: "Next song".to_owned(),
    });
    host.broadcast_control(&start).expect("cache current stream");
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize late joiner");
    // A live boundary immediately after authorization must follow the
    // replayed StreamStart on the listener's actual TCP control channel.
    host.broadcast_control(&next).expect("broadcast live boundary");
    wait_for_authorized(&mut *host, &device_id);

    let expected = [start, next];
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while received.len() < expected.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing control frame after authorization");
        if let super::TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(message),
            ..
        } = listener.recv_event(remaining).expect("listener control event")
        {
            received.push(message);
        }
    }
    assert!(
        received == expected,
        "late-join replay must precede a live boundary without duplication"
    );

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}
