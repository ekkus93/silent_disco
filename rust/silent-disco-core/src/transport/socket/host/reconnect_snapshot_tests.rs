//! Real authenticated TCP replay across Pause, Resume, Stop and fresh Play.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{MonotonicMillis, SampleIndex, StreamId};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop, TrackBoundary};
use crate::transport::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportEvent,
    TransportFactory,
};

use super::SocketHostTransport;

fn connect_authorized(
    host: &mut SocketHostTransport,
    factory: SocketTransportFactory,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect listener");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("join request");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(host, &device);
    listener
}

fn assert_tcp_controls(listener: &mut dyn ListenerTransportNode, expected: &[ControlMessage]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    for control in expected {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "missing ordered TCP replay/control frame"
            );
            if let TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(received),
                ..
            } = listener.recv_event(remaining).expect("listener event")
            {
                assert!(received == *control, "unexpected TCP control order");
                break;
            }
        }
    }
}

#[test]
fn real_tcp_reconnect_replays_paused_and_resumed_queue_then_fences_stop_replay() {
    let session = id_session("reconnect-pause-resume-stop-replay");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let factory = SocketTransportFactory;
    let original = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(start) = original.clone() else {
        panic!("fixture StreamStart");
    };
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: start.stream_id.clone(),
        first_sample_index: SampleIndex::new(48_000),
        entry_id: "upcoming".to_owned(),
        display_name: "Upcoming track".to_owned(),
    });
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: start.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_500),
    });
    for control in [&original, &boundary, &pause] {
        host.broadcast_control(control).expect("cache playback");
    }

    // Listener A joins while paused: it must receive the original timeline,
    // the pending boundary and Pause, in that exact TCP order.
    let mut a = connect_authorized(&mut host, factory, &clock, &session, "reconnect-a");
    assert_tcp_controls(
        a.as_mut(),
        &[original.clone(), boundary.clone(), pause.clone()],
    );
    let sent_before_stale = host.counters().control_frames_sent;
    host.broadcast_control(&original)
        .expect("reject delayed original while paused");
    assert_eq!(host.counters().control_frames_sent, sent_before_stale);

    let mut resumed_start = start.clone();
    resumed_start.host_start_time_ms = MonotonicMillis::new(1_100);
    resumed_start.current_display_name = Some("Now resumed".to_owned());
    let resume = ControlMessage::StreamStart(resumed_start);
    host.broadcast_control(&resume)
        .expect("real reanchored Resume");
    assert_tcp_controls(a.as_mut(), std::slice::from_ref(&resume));

    // Listener B joins after Resume: its snapshot must have the shifted
    // timeline and upcoming boundary, but never the old Pause.
    let mut b = connect_authorized(&mut host, factory, &clock, &session, "reconnect-b");
    assert_tcp_controls(b.as_mut(), &[resume.clone(), boundary.clone()]);
    let later_pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: start.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(2_500),
    });
    host.broadcast_control(&later_pause).expect("pause again");
    assert_tcp_controls(a.as_mut(), std::slice::from_ref(&later_pause));
    assert_tcp_controls(b.as_mut(), std::slice::from_ref(&later_pause));
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: start.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(3_000),
    });
    host.broadcast_control(&stop).expect("stop stream");
    assert_tcp_controls(a.as_mut(), std::slice::from_ref(&stop));
    assert_tcp_controls(b.as_mut(), std::slice::from_ref(&stop));

    // Stop must leave no stale replay for listener C. Nor may a delayed
    // original start resurrect the stopped generation.
    let sent_at_stop = host.counters().control_frames_sent;
    let mut c = connect_authorized(&mut host, factory, &clock, &session, "reconnect-c");
    assert_eq!(host.counters().control_frames_sent, sent_at_stop);
    host.broadcast_control(&original)
        .expect("reject retired generation");
    assert_eq!(host.counters().control_frames_sent, sent_at_stop);

    let ControlMessage::StreamStart(mut fresh) = stream_start(&session, 4_000) else {
        panic!("fresh StreamStart");
    };
    fresh.stream_id = StreamId::new("replay-generation").expect("new stream ID");
    let play = ControlMessage::StreamStart(fresh);
    host.broadcast_control(&play).expect("fresh Play");
    for listener in [&mut a, &mut b, &mut c] {
        assert_tcp_controls(listener.as_mut(), std::slice::from_ref(&play));
    }
    let mut d = connect_authorized(&mut host, factory, &clock, &session, "reconnect-d");
    assert_tcp_controls(d.as_mut(), std::slice::from_ref(&play));

    for listener in [&mut a, &mut b, &mut c, &mut d] {
        listener.shutdown().expect("listener shutdown");
    }
    host.shutdown().expect("host shutdown");
}

#[test]
fn same_identity_reconnect_replaces_retired_routes_and_replays_only_current_controls() {
    let session = id_session("same-identity-reconnect-controls");
    let device = id_device("returning-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP/UDP host");
    let factory = SocketTransportFactory;
    let start = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(mut resumed) = start.clone() else {
        panic!("start fixture");
    };
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: resumed.stream_id.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "second-song".to_owned(),
        display_name: "Second song".to_owned(),
    });
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: resumed.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_010),
    });
    let mut first = connect_authorized(&mut host, factory, &clock, &session, "returning-listener");
    for control in [&start, &boundary, &pause] {
        assert_eq!(
            host.broadcast_control(control)
                .expect("broadcast active queue state")
                .report
                .successful_peers,
            1,
        );
    }
    assert_tcp_controls(
        first.as_mut(),
        &[start.clone(), boundary.clone(), pause.clone()],
    );

    // Retiring the old TCP identity must also retire its UDP route before
    // the same device can authorize a new socket. No ghost audio delivery.
    let first_peer_number = host
        .routes
        .lock()
        .expect("first route")
        .get(&device)
        .expect("first peer route")
        .peer
        .number;
    host.disconnect_peer(&device)
        .expect("retire first connection");
    first.shutdown().expect("close first socket");
    let packet = audio_frame(&session, 0);
    assert_eq!(
        host.broadcast_audio(&packet)
            .expect("no UDP route after disconnect")
            .report
            .intended_peers,
        0,
    );

    let mut second = connect_authorized(&mut host, factory, &clock, &session, "returning-listener");
    assert_tcp_controls(
        second.as_mut(),
        &[start.clone(), boundary.clone(), pause.clone()],
    );
    // Deterministically replay a delayed cleanup from the retired reader.
    // It must not revoke the replacement socket's TCP/UDP authorization.
    assert!(!super::super::host_workers::remove_route_if_current(
        &host.routes,
        &device,
        first_peer_number,
    ));
    assert!(
        host.routes
            .lock()
            .expect("replacement route")
            .get(&device)
            .expect("replacement must remain authorized")
            .peer
            .active
            .load(std::sync::atomic::Ordering::Acquire),
    );
    resumed.host_start_time_ms = MonotonicMillis::new(1_200);
    let resume = ControlMessage::StreamStart(resumed.clone());
    assert_eq!(
        host.broadcast_control(&resume)
            .expect("resume live stream")
            .report
            .successful_peers,
        1,
    );
    assert_tcp_controls(second.as_mut(), std::slice::from_ref(&resume));
    assert_eq!(
        host.broadcast_audio(&packet)
            .expect("UDP to reconnected identity")
            .report
            .successful_peers,
        1,
    );
    wait_for_frame(second.as_mut(), TransportChannel::Audio, |frame| {
        frame == &packet
    });

    // Reconnecting again after Resume must replay the new anchor and queue,
    // not the earlier Pause or the retired original start.
    host.disconnect_peer(&device)
        .expect("retire second connection");
    second.shutdown().expect("close second socket");
    let mut third = connect_authorized(&mut host, factory, &clock, &session, "returning-listener");
    assert_tcp_controls(third.as_mut(), &[resume.clone(), boundary.clone()]);
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: resumed.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_420),
    });
    host.broadcast_control(&stop).expect("stop current stream");
    assert_tcp_controls(third.as_mut(), std::slice::from_ref(&stop));
    host.disconnect_peer(&device)
        .expect("retire stopped connection");
    third.shutdown().expect("close stopped socket");

    let sent_at_stop = host.counters().control_frames_sent;
    let mut fourth = connect_authorized(&mut host, factory, &clock, &session, "returning-listener");
    assert_eq!(
        host.counters().control_frames_sent,
        sent_at_stop,
        "Stop clears the snapshot even for a returning device",
    );
    host.broadcast_control(&start).expect("drop delayed start");
    host.broadcast_control(&boundary)
        .expect("drop delayed boundary");
    let mut delayed_play = resumed.clone();
    delayed_play.stream_id = StreamId::new("stale-play-variant").expect("stream ID");
    delayed_play.host_start_time_ms = MonotonicMillis::new(1_300);
    host.broadcast_control(&ControlMessage::StreamStart(delayed_play))
        .expect("drop delayed Play from before Stop");
    assert_eq!(host.counters().control_frames_sent, sent_at_stop);

    // Fresh Play may intentionally reuse the stream ID with a newer anchor.
    // Its TCP start and UDP payload must reach only the newly authorized route.
    resumed.host_start_time_ms = MonotonicMillis::new(2_000);
    let play = ControlMessage::StreamStart(resumed.clone());
    assert_eq!(
        host.broadcast_control(&play)
            .expect("fresh Play")
            .report
            .successful_peers,
        1,
    );
    assert_tcp_controls(fourth.as_mut(), std::slice::from_ref(&play));
    assert_eq!(
        host.broadcast_audio(&packet)
            .expect("fresh UDP route")
            .report
            .successful_peers,
        1,
    );
    wait_for_frame(fourth.as_mut(), TransportChannel::Audio, |frame| {
        frame == &packet
    });

    // Reusing the stream ID must not let delayed controls from before Stop
    // interrupt this new Play or poison a late-join snapshot.
    let before_stale = host.counters().control_frames_sent;
    assert_eq!(
        host.broadcast_control(&pause)
            .expect("drop retired Pause")
            .report
            .intended_peers,
        0,
    );
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("drop retired Stop")
            .report
            .intended_peers,
        0,
    );
    assert_eq!(host.counters().control_frames_sent, before_stale);

    let fresh_pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: resumed.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(2_200),
    });
    host.broadcast_control(&fresh_pause)
        .expect("new generation Pause");
    assert_tcp_controls(fourth.as_mut(), std::slice::from_ref(&fresh_pause));
    let mut fifth = connect_authorized(&mut host, factory, &clock, &session, "late-replay");
    assert_tcp_controls(fifth.as_mut(), &[play, fresh_pause]);
    fifth.shutdown().expect("late listener shutdown");
    fourth.shutdown().expect("final listener shutdown");
    host.shutdown().expect("host shutdown");
}
