//! Real TCP/UDP qualification for paused multi-track reconnect and Stop/Play.
//! These are production socket routes, not mocked transport events.
use std::sync::Arc;

use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop, TrackBoundary};
use crate::transport::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportFactory,
};

use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect real listener socket");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("request admission");
    wait_for_control_from(host, &device, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize UDP/TCP routes");
    wait_for_authorized(host, &device);
    listener
}

fn received_control(listener: &mut dyn ListenerTransportNode, expected: &ControlMessage) {
    wait_for_frame(listener, TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(expected.clone())
    });
}

fn packet(
    session: &crate::domain::SessionId,
    sequence: u64,
    presentation_ms: u64,
) -> ProtocolFrame {
    let mut frame = audio_frame(session, sequence);
    let ProtocolFrame::Audio(ref mut audio) = frame else {
        panic!("audio fixture");
    };
    audio.host_presentation_time_ms = MonotonicMillis::new(presentation_ms);
    frame
}

fn deliver_audio(
    host: &SocketHostTransport,
    listeners: &mut [&mut dyn ListenerTransportNode],
    frame: &ProtocolFrame,
) {
    assert_eq!(
        host.broadcast_audio(frame)
            .expect("broadcast production UDP")
            .report
            .successful_peers,
        u32::try_from(listeners.len()).expect("bounded listeners"),
    );
    for listener in listeners {
        wait_for_frame(*listener, TransportChannel::Audio, |received| received == frame);
    }
}

#[test]
fn paused_multitrack_reconnect_and_late_join_keep_udp_contiguous_until_stop_replay() {
    let session = id_session("socket-queue-reconnect-stop-replay");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP/UDP host");
    let ControlMessage::StreamStart(mut current) = stream_start(&session, 1_000) else {
        panic!("StreamStart fixture");
    };
    current.samples_per_packet = 2;
    let start = ControlMessage::StreamStart(current.clone());
    let boundaries: Vec<ControlMessage> = [(4, "second"), (8, "third")]
        .into_iter()
        .map(|(index, name)| {
            ControlMessage::TrackBoundary(TrackBoundary {
                session_id: session.clone(),
                stream_id: current.stream_id.clone(),
                first_sample_index: SampleIndex::new(index),
                entry_id: name.to_owned(),
                display_name: name.to_owned(),
            })
        })
        .collect();
    for control in std::iter::once(&start).chain(boundaries.iter()) {
        host.broadcast_control(control).expect("cache authoritative queue");
    }

    // Admission after playback has started replays the entire ordered queue.
    let mut first = join(&mut host, &clock, &session, "returning-queue-listener");
    for control in std::iter::once(&start).chain(boundaries.iter()) {
        received_control(first.as_mut(), control);
    }
    for sequence in 0..2 {
        let frame = packet(&session, sequence, 1_000 + sequence);
        deliver_audio(&host, &mut [first.as_mut()], &frame);
    }
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: current.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_100),
    });
    host.broadcast_control(&pause).expect("pause playlist");
    received_control(first.as_mut(), &pause);
    assert_eq!(
        host.broadcast_audio(&packet(&session, 2, 1_102))
            .expect("hold paused producer")
            .report
            .intended_peers,
        0,
        "the third track's upcoming PCM must not escape during Pause"
    );

    // Same-identity reconnection must replace the old authorized route and
    // replay Start, both boundaries, then Pause before accepting new UDP.
    let device = id_device("returning-queue-listener");
    host.disconnect_peer(&device).expect("disconnect first socket");
    first.shutdown().expect("close old socket");
    let mut returning = join(&mut host, &clock, &session, "returning-queue-listener");
    for control in std::iter::once(&start)
        .chain(boundaries.iter())
        .chain(std::iter::once(&pause))
    {
        received_control(returning.as_mut(), control);
    }
    current.host_start_time_ms = MonotonicMillis::new(1_200);
    let resume = ControlMessage::StreamStart(current.clone());
    host.broadcast_control(&resume).expect("resume same output timeline");
    received_control(returning.as_mut(), &resume);
    for sequence in 2..4 {
        let frame = packet(&session, sequence, 1_200 + sequence);
        deliver_audio(&host, &mut [returning.as_mut()], &frame);
    }

    // A separate late joiner gets the shifted anchor and the two boundaries,
    // but never the superseded Pause or an extra per-track StreamStart.
    let mut late = join(&mut host, &clock, &session, "late-queue-listener");
    for control in std::iter::once(&resume).chain(boundaries.iter()) {
        received_control(late.as_mut(), control);
    }
    let third_track = packet(&session, 4, 1_204);
    deliver_audio(
        &host,
        &mut [returning.as_mut(), late.as_mut()],
        &third_track,
    );

    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: current.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_400),
    });
    host.broadcast_control(&stop).expect("stop queue");
    received_control(returning.as_mut(), &stop);
    received_control(late.as_mut(), &stop);
    let mut stopped_join = join(&mut host, &clock, &session, "stopped-queue-listener");
    assert_eq!(
        host.broadcast_audio(&packet(&session, 5, 1_405))
            .expect("drop retired audio")
            .report
            .intended_peers,
        0,
    );

    // Play reuses the stream ID, but resets the queue's packet/sample clock
    // and reconnect snapshot. Old sequence-five UDP is rejected even when
    // its timestamp is moved beyond the new Play anchor.
    current.host_start_time_ms = MonotonicMillis::new(1_500);
    let replay = ControlMessage::StreamStart(current);
    host.broadcast_control(&replay).expect("fresh Play");
    for listener in [&mut returning, &mut late, &mut stopped_join] {
        received_control(listener.as_mut(), &replay);
    }
    assert_eq!(
        host.broadcast_audio(&packet(&session, 5, 1_505))
            .expect("reject future-dated retired PCM")
            .report
            .intended_peers,
        0,
    );
    let mut replay_first = packet(&session, 0, 1_500);
    let ProtocolFrame::Audio(ref mut audio) = replay_first else {
        panic!("replay packet");
    };
    audio.sequence = PacketSequence::new(0);
    audio.first_sample_index = SampleIndex::new(0);
    deliver_audio(
        &host,
        &mut [
            returning.as_mut(),
            late.as_mut(),
            stopped_join.as_mut(),
        ],
        &replay_first,
    );
    let mut fresh_join = join(&mut host, &clock, &session, "replay-queue-listener");
    received_control(fresh_join.as_mut(), &replay);
    let replay_second = packet(&session, 1, 1_501);
    deliver_audio(
        &host,
        &mut [
            returning.as_mut(),
            late.as_mut(),
            stopped_join.as_mut(),
            fresh_join.as_mut(),
        ],
        &replay_second,
    );

    for listener in [&mut returning, &mut late, &mut stopped_join, &mut fresh_join] {
        listener.shutdown().expect("close listener");
    }
    host.shutdown().expect("close host");
}
