//! Real TCP recovery replay of the host's authoritative control cache.
//! No synthetic UI state or new native output is created for resync.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, ResyncNotice, Stop, TrackBoundary};
use crate::transport::test_support::{
    id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportEvent,
    TransportFactory,
};

use super::SocketHostTransport;

fn join(
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
        .expect("send join request");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(host, &device);
    listener
}

fn expect_controls(listener: &dyn ListenerTransportNode, expected: &[ControlMessage]) {
    for control in expected {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "missing replayed TCP control");
            if let TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(received),
                ..
            } = listener.recv_event(remaining).expect("read listener control")
            {
                assert_eq!(&received, control, "replay order and contents");
                break;
            }
        }
    }
}

#[test]
fn resync_replays_only_to_admitted_requester_and_stop_disarms_recovery() {
    let session = id_session("resync-authoritative-snapshot");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()), clock.clone(),
    )
    .expect("bind host");
    let factory = SocketTransportFactory;
    let first_id = id_device("resync-first");
    let mut first = join(&mut host, factory, &clock, &session, "resync-first");
    let mut second = join(&mut host, factory, &clock, &session, "resync-second");
    let start = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(start_info) = &start else {
        panic!("start fixture");
    };
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: start_info.stream_id.clone(),
        first_sample_index: SampleIndex::new(48_000),
        entry_id: "next".to_owned(),
        display_name: "Next track".to_owned(),
    });
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: start_info.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_200),
    });
    let expected = [start, boundary, pause];
    for control in &expected {
        assert_eq!(host.broadcast_control(control).expect("broadcast").report.successful_peers, 2);
    }
    expect_controls(first.as_ref(), &expected);
    expect_controls(second.as_ref(), &expected);

    first.send_control(&ControlMessage::ResyncNotice(ResyncNotice {
        session_id: session.clone(),
        listener_id: first_id.clone(),
        reason: "missing control after reconnect".to_owned(),
    }))
    .expect("authorized listener requests recovery");
    wait_for_control_from(&mut host, &first_id, |message| {
        matches!(message, ControlMessage::ResyncNotice(_))
    });
    let before = host.counters().control_frames_sent;
    let delivery = host.replay_cached_playback_to_peer(&first_id).expect("targeted recovery");
    assert_eq!(delivery.report.successful_peers, 1);
    assert!(delivery.report.bytes_sent > 0);
    assert_eq!(host.counters().control_frames_sent - before, 3);
    expect_controls(first.as_ref(), &expected);

    // Stop invalidates the cache: recovery must never resurrect the old stream.
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: start_info.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_500),
    });
    host.broadcast_control(&stop).expect("stop playback");
    expect_controls(first.as_ref(), std::slice::from_ref(&stop));
    expect_controls(second.as_ref(), std::slice::from_ref(&stop));
    let stopped = host.counters().control_frames_sent;
    assert_eq!(host.replay_cached_playback_to_peer(&first_id).expect("idle recovery").report.intended_peers, 0);
    assert_eq!(host.counters().control_frames_sent, stopped);
    assert!(host.replay_cached_playback_to_peer(&id_device("unknown")).is_err());

    first.shutdown().expect("close first");
    second.shutdown().expect("close second");
    host.shutdown().expect("close host");
}
