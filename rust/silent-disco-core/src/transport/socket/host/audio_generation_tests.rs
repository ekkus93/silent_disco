//! Real TCP/UDP regression for producer packets racing authoritative
//! Pause, replacement `StreamStart`, Stop, and replay to a live listener.
use std::sync::Arc;

use crate::domain::{MonotonicMillis, StreamId};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop};
use crate::transport::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock, SocketTransportFactory, TransportChannel, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn tcp_pause_stop_and_replay_fence_producer_udp_without_losing_new_generation_audio() {
    let session = id_session("socket-generation-audio-fence");
    let device = id_device("socket-audio-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production host");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("connect listener");
    listener
        .send_control(&join_request(&session, &device, "socket-audio-listener"))
        .expect("join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize");
    wait_for_authorized(&mut host, &device);

    let original = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(mut active) = original.clone() else {
        panic!("start fixture");
    };
    let first_packet = audio_frame(&session, 0);
    assert_eq!(
        host.broadcast_control(&original)
            .expect("start playback")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(original.clone())
    });
    assert_eq!(
        host.broadcast_audio(&first_packet)
            .expect("first generation audio")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
        frame == &first_packet
    });

    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: active.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_100),
    });
    host.broadcast_control(&pause).expect("pause");
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(pause.clone())
    });
    assert_eq!(
        host.broadcast_audio(&audio_frame(&session, 1))
            .expect("paused producer packet")
            .report
            .intended_peers,
        0,
        "paused audio must not leave the host UDP socket"
    );
    active.host_start_time_ms = MonotonicMillis::new(1_200);
    let resume = ControlMessage::StreamStart(active.clone());
    host.broadcast_control(&resume).expect("resume");
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(resume.clone())
    });
    let second_packet = audio_frame(&session, 1);
    assert_eq!(
        host.broadcast_audio(&second_packet)
            .expect("resumed audio")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
        frame == &second_packet
    });

    // Replacement stream with a new identity must evict delayed packets
    // even when they belong to the same authorized transport session.
    active.stream_id = StreamId::new("replacement-audio-stream").expect("replacement");
    active.host_start_time_ms = MonotonicMillis::new(2_000);
    let replace = ControlMessage::StreamStart(active.clone());
    host.broadcast_control(&replace).expect("replace stream");
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(replace.clone())
    });
    assert_eq!(
        host.broadcast_audio(&audio_frame(&session, 2))
            .expect("retired stream audio")
            .report
            .intended_peers,
        0
    );
    let mut current_packet = audio_frame(&session, 2);
    let ProtocolFrame::Audio(ref mut pcm) = current_packet else {
        panic!("audio fixture");
    };
    pcm.stream_id = active.stream_id.clone();
    assert_eq!(
        host.broadcast_audio(&current_packet)
            .expect("replacement audio")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
        frame == &current_packet
    });

    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: active.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_500),
    });
    host.broadcast_control(&stop).expect("Stop");
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(stop.clone())
    });
    for frame in [&first_packet, &current_packet] {
        assert_eq!(
            host.broadcast_audio(frame)
                .expect("reject UDP after Stop")
                .report
                .intended_peers,
            0,
            "retired producer cannot deliver frames after Stop"
        );
    }

    // Fresh Play with a valid StreamStart reinstates only the new producer.
    // It may intentionally reuse the previous stream ID; Stop must not
    // permanently disable the host's UDP route.
    active.host_start_time_ms = MonotonicMillis::new(3_000);
    let replay = ControlMessage::StreamStart(active);
    host.broadcast_control(&replay).expect("fresh Play");
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(replay.clone())
    });
    assert_eq!(
        host.broadcast_audio(&current_packet)
            .expect("new generation audio")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
        frame == &current_packet
    });

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}
