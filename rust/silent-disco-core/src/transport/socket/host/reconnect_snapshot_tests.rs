//! Real authenticated TCP replay across Pause, Resume, Stop and fresh Play.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{MonotonicMillis, SampleIndex, StreamId};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop, TrackBoundary};
use crate::transport::test_support::{
    id_device, id_session, join_request, stream_start, wait_for_authorized, wait_for_control_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportEvent,
    TransportFactory,
};

use super::SocketHostTransport;

fn connect_authorized(
    host: &mut SocketHostTransport,
    factory: &SocketTransportFactory,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session.clone(),
                device.clone(),
                host.endpoint(),
            ),
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

fn assert_tcp_controls(
    listener: &mut dyn ListenerTransportNode,
    expected: &[ControlMessage],
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    for control in expected {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "missing ordered TCP replay/control frame");
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
    let mut a = connect_authorized(&mut host, &factory, &clock, &session, "reconnect-a");
    assert_tcp_controls(a.as_mut(), &[original.clone(), boundary.clone(), pause.clone()]);
    let sent_before_stale = host.counters().control_frames_sent;
    host.broadcast_control(&original)
        .expect("reject delayed original while paused");
    assert_eq!(host.counters().control_frames_sent, sent_before_stale);

    let mut resumed_start = start.clone();
    resumed_start.host_start_time_ms = MonotonicMillis::new(1_100);
    resumed_start.current_display_name = Some("Now resumed".to_owned());
    let resume = ControlMessage::StreamStart(resumed_start);
    host.broadcast_control(&resume).expect("real reanchored Resume");
    assert_tcp_controls(a.as_mut(), std::slice::from_ref(&resume));

    // Listener B joins after Resume: its snapshot must have the shifted
    // timeline and upcoming boundary, but never the old Pause.
    let mut b = connect_authorized(&mut host, &factory, &clock, &session, "reconnect-b");
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
    let mut c = connect_authorized(&mut host, &factory, &clock, &session, "reconnect-c");
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
    let mut d = connect_authorized(&mut host, &factory, &clock, &session, "reconnect-d");
    assert_tcp_controls(d.as_mut(), std::slice::from_ref(&play));

    for listener in [&mut a, &mut b, &mut c, &mut d] {
        listener.shutdown().expect("listener shutdown");
    }
    host.shutdown().expect("host shutdown");
}
