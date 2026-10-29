//! Real TCP/UDP multi-peer playback control ordering under a stalled listener.
//! This exercises the production socket readers/writers, not a synthetic channel.
use std::sync::Arc;
use std::time::Duration;

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{ControlMessage, Heartbeat, Pause, ProtocolFrame, TrackBoundary};
use crate::transport::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, stream_stop,
    wait_for_authorized, wait_for_control_from, wait_for_frame, wait_until,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportEvent,
    TransportFactory,
};

use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
    event_capacity: usize,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let mut config =
        ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint());
    config.event_queue_capacity = event_capacity;
    let listener = SocketTransportFactory
        .connect_listener(config, clock.clone())
        .expect("connect real listener TCP/UDP");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("send real TCP join");
    wait_for_control_from(host, &device, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize real listener routes");
    wait_for_authorized(host, &device);
    listener
}

fn expect_ordered_controls(
    listener: &mut dyn ListenerTransportNode,
    controls: &[ControlMessage],
) {
    for expected in controls {
        let event = listener
            .recv_event(Duration::from_secs(5))
            .expect("authoritative TCP event must arrive");
        assert!(
            matches!(
                event,
                TransportEvent::FrameReceived {
                    channel: TransportChannel::Control,
                    frame: ProtocolFrame::Control(ref actual),
                    ..
                } if actual == expected
            ),
            "authoritative controls must retain exact order without telemetry interleaving"
        );
    }
}

#[test]
fn multi_peer_telemetry_pressure_preserves_boundary_pause_stop_and_fresh_replay() {
    let session = id_session("multi-peer-control-pressure-replay");
    let slow_id = id_device("stalled-control-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP/UDP host");
    let mut slow = join(&mut host, &clock, &session, "stalled-control-listener", 1);
    let mut fast = join(&mut host, &clock, &session, "fast-control-listener", 64);

    let ControlMessage::StreamStart(mut started) = stream_start(&session, 1_000) else {
        panic!("StreamStart fixture");
    };
    started.samples_per_packet = 2;
    let start = ControlMessage::StreamStart(started.clone());
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        first_sample_index: SampleIndex::new(4),
        entry_id: "second".to_owned(),
        display_name: "Second track".to_owned(),
    });
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_100),
    });
    let stop = stream_stop(&session, 1_200);

    assert_eq!(
        host.broadcast_control(&start)
            .expect("broadcast Start")
            .report
            .successful_peers,
        2
    );
    wait_until(Duration::from_secs(5), || {
        slow.counters().control_frames_received >= 1
    });
    // Start occupies the slow listener's single event slot. Flood only this
    // TCP route with lossy heartbeat telemetry; the other peer must remain
    // independent, and the stalled reader must not silently drop later Stop.
    for index in 0..32 {
        host.send_control(
            &slow_id,
            &ControlMessage::Heartbeat(Heartbeat {
                session_id: session.clone(),
                listener_id: slow_id.clone(),
                sent_at_elapsed_ms: MonotonicMillis::new(index),
            }),
        )
        .expect("send real TCP telemetry");
    }
    wait_until(Duration::from_secs(5), || {
        slow.counters().control_frames_received >= 33
            && slow.counters().queue_overflows > 0
    });

    for control in [&boundary, &pause, &stop] {
        assert_eq!(
            host.broadcast_control(control)
                .expect("broadcast authoritative TCP control")
                .report
                .successful_peers,
            2
        );
    }
    let expected = [start.clone(), boundary, pause, stop];
    expect_ordered_controls(slow.as_mut(), &expected);
    expect_ordered_controls(fast.as_mut(), &expected);
    assert_eq!(slow.counters().delivery_failures, 0);
    assert_eq!(fast.counters().delivery_failures, 0);

    // A Stop clears the late-join cache and fences retired PCM even when the
    // slow listener had not been collecting controls. Replace its TCP/UDP
    // route, then start a new generation with the same stream ID.
    assert_eq!(
        host.broadcast_audio(&audio_frame(&session, 0))
            .expect("retired audio is suppressed")
            .report
            .intended_peers,
        0
    );
    host.disconnect_peer(&slow_id)
        .expect("retire stalled listener");
    slow.shutdown().expect("close old listener");
    let mut replacement = join(&mut host, &clock, &session, "stalled-control-listener", 64);
    started.host_start_time_ms = MonotonicMillis::new(2_000);
    let replay = ControlMessage::StreamStart(started);
    assert_eq!(
        host.broadcast_control(&replay)
            .expect("broadcast fresh Play")
            .report
            .successful_peers,
        2
    );
    for listener in [&mut fast, &mut replacement] {
        wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(replay.clone())
        });
    }
    let mut first_pcm = audio_frame(&session, 0);
    let ProtocolFrame::Audio(ref mut packet) = first_pcm else {
        panic!("audio fixture");
    };
    packet.host_presentation_time_ms = MonotonicMillis::new(2_000);
    assert_eq!(
        host.broadcast_audio(&first_pcm)
            .expect("send fresh UDP PCM")
            .report
            .successful_peers,
        2
    );
    for listener in [&mut fast, &mut replacement] {
        wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
            frame == &first_pcm
        });
    }
    fast.shutdown().expect("close fast listener");
    replacement.shutdown().expect("close replacement listener");
    host.shutdown().expect("close host");
}
