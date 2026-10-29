//! Real TCP/UDP pause/rejoin/replay regression across an audible midqueue boundary.
//! Exercises production socket control snapshots, not physical output or TLS-v4.
use std::sync::Arc;

use crate::audio::{AudioFormat, AudioSampleFormat, DecodedPcmChunk, Packetizer};
use crate::domain::{MonotonicMillis, SampleIndex, StreamId};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop, TrackBoundary};
use crate::transport::test_support::{
    id_device, id_session, join_request, wait_for_authorized, wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportFactory,
};

use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let session = id_session("paused-midqueue-real-socket");
    let device = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect listener");
    listener
        .send_control(&join_request(&session, &device, name))
        .expect("send join");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize");
    wait_for_authorized(host, &device);
    listener
}

fn receive(listener: &mut dyn ListenerTransportNode, frame: &ProtocolFrame) {
    let channel = if matches!(frame, ProtocolFrame::Audio(_)) {
        TransportChannel::Audio
    } else {
        TransportChannel::Control
    };
    wait_for_frame(listener, channel, |received| received == frame);
}

#[test]
fn pause_at_midqueue_replays_frozen_title_then_reanchors_pcm_and_retires_on_stop() {
    let session = id_session("paused-midqueue-real-socket");
    let stream = StreamId::new("paused-midqueue-stream").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production host");
    let mut first = join(&mut host, &clock, "first");
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 2,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let mut packetizer = Packetizer::new(
        session.clone(),
        stream.clone(),
        format,
        5,
        MonotonicMillis::new(1_000),
    )
    .expect("packetizer");
    let ProtocolFrame::Control(ControlMessage::StreamStart(mut start)) =
        packetizer.stream_start_message()
    else {
        panic!("start frame");
    };
    start.current_entry_id = Some("track-a".to_owned());
    start.current_display_name = Some("Track A".to_owned());
    let start_control = ControlMessage::StreamStart(start.clone());
    host.broadcast_control(&start_control).expect("start");
    receive(first.as_mut(), &ProtocolFrame::Control(start_control));

    let boundaries: Vec<_> = [(360, "b"), (480, "c")]
        .into_iter()
        .map(|(index, name)| {
            ControlMessage::TrackBoundary(TrackBoundary {
                session_id: session.clone(),
                stream_id: stream.clone(),
                first_sample_index: SampleIndex::new(index),
                entry_id: format!("track-{name}"),
                display_name: format!("Track {}", name.to_ascii_uppercase()),
            })
        })
        .collect();
    for boundary in &boundaries {
        host.broadcast_control(boundary).expect("boundary");
        receive(first.as_mut(), &ProtocolFrame::Control(boundary.clone()));
    }

    let mut packets = Vec::new();
    let mut expected_pcm = Vec::new();
    for (frames, base) in [(360_usize, -300_i16), (120, 1_000), (240, 2_000)] {
        let samples: Vec<i16> = (0..frames)
            .flat_map(|index| {
                let sample = base + i16::try_from(index).expect("sample index");
                [sample, -sample]
            })
            .collect();
        expected_pcm.extend(&samples);
        let outcome = packetizer
            .push_contiguous_chunk(DecodedPcmChunk {
                format,
                first_sample_index: SampleIndex::new(0),
                frames: samples,
                end_of_stream: frames == 240,
            })
            .expect("packetize contiguous track");
        assert_eq!(outcome.frames.len(), 1);
        packets.push(outcome.frames.into_iter().next().expect("packet"));
    }
    assert!(packetizer.is_finished());
    assert_eq!(packetizer.next_contiguous_sample_index(), 720);
    for frame in packets.iter().take(2) {
        assert_eq!(
            host.broadcast_audio(frame).expect("audio").report.successful_peers,
            1
        );
        receive(first.as_mut(), frame);
    }

    // At 1008 ms B is audible, while C (sample 480) is still upcoming.
    clock.set(1_008);
    let mut midqueue = join(&mut host, &clock, "midqueue");
    let mut current_b = start.clone();
    current_b.current_entry_id = Some("track-b".to_owned());
    current_b.current_display_name = Some("Track B".to_owned());
    receive(
        midqueue.as_mut(),
        &ProtocolFrame::Control(ControlMessage::StreamStart(current_b.clone())),
    );
    receive(midqueue.as_mut(), &ProtocolFrame::Control(boundaries[1].clone()));

    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_008),
    });
    host.broadcast_control(&pause).expect("pause");
    for listener in [&mut first, &mut midqueue] {
        receive(listener.as_mut(), &ProtocolFrame::Control(pause.clone()));
    }
    clock.set(1_020);
    let mut paused_join = join(&mut host, &clock, "paused-join");
    for control in [
        ControlMessage::StreamStart(current_b.clone()),
        boundaries[1].clone(),
        pause,
    ] {
        receive(paused_join.as_mut(), &ProtocolFrame::Control(control));
    }
    assert_eq!(
        host.broadcast_audio(&packets[2])
            .expect("suppress audio while paused")
            .report
            .intended_peers,
        0
    );

    // A 192 ms pause moves the original 1000 ms anchor to 1192 ms.
    // Sample 480 must be presented at 1202 ms, not 1010 or 1200.
    clock.set(1_200);
    let mut resumed = current_b;
    resumed.host_start_time_ms = MonotonicMillis::new(1_192);
    let resume = ControlMessage::StreamStart(resumed);
    host.broadcast_control(&resume).expect("resume");
    for listener in [&mut first, &mut midqueue, &mut paused_join] {
        receive(listener.as_mut(), &ProtocolFrame::Control(resume.clone()));
    }
    let ProtocolFrame::Audio(ref mut final_packet) = packets[2] else {
        panic!("audio packet");
    };
    final_packet.host_presentation_time_ms = MonotonicMillis::new(1_202);
    assert_eq!(
        host.broadcast_audio(&packets[2])
            .expect("resumed audio")
            .report
            .successful_peers,
        3
    );
    for listener in [&mut first, &mut midqueue, &mut paused_join] {
        receive(listener.as_mut(), &packets[2]);
    }
    clock.set(1_203);
    let mut after_boundary = join(&mut host, &clock, "after-boundary");
    let mut current_c = start;
    current_c.host_start_time_ms = MonotonicMillis::new(1_192);
    current_c.current_entry_id = Some("track-c".to_owned());
    current_c.current_display_name = Some("Track C".to_owned());
    receive(
        after_boundary.as_mut(),
        &ProtocolFrame::Control(ControlMessage::StreamStart(current_c)),
    );

    let mut decoded = Vec::new();
    for (index, frame) in packets.iter().enumerate() {
        let ProtocolFrame::Audio(packet) = frame else {
            panic!("audio packet");
        };
        assert_eq!(packet.sequence.get(), index as u64);
        assert_eq!(packet.first_sample_index.get(), index as u64 * 240);
        assert_eq!(
            packet.host_presentation_time_ms.get(),
            [1_000, 1_005, 1_202][index]
        );
        decoded.extend(
            packet
                .payload
                .chunks_exact(2)
                .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]])),
        );
    }
    assert_eq!(decoded, expected_pcm);

    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream,
        host_stop_time_ms: MonotonicMillis::new(1_220),
    });
    host.broadcast_control(&stop).expect("stop");
    for listener in [&mut first, &mut midqueue, &mut paused_join, &mut after_boundary] {
        receive(listener.as_mut(), &ProtocolFrame::Control(stop.clone()));
    }
    assert!(
        host.playback_controls
            .lock()
            .expect("cache")
            .snapshot_at(MonotonicMillis::new(1_300))
            .is_empty(),
        "Stop must not replay the paused title or future boundary"
    );
    let mut after_stop = join(&mut host, &clock, "after-stop");
    let replay = Packetizer::new(
        session,
        StreamId::new("paused-midqueue-replay").expect("new stream"),
        format,
        5,
        MonotonicMillis::new(2_000),
    )
    .expect("fresh packetizer");
    let replay_start = replay.stream_start_message();
    let ProtocolFrame::Control(ref replay_control) = replay_start else {
        panic!("replay start");
    };
    assert_eq!(
        host.broadcast_control(replay_control)
            .expect("new generation")
            .report
            .successful_peers,
        5
    );
    for listener in [
        &mut first,
        &mut midqueue,
        &mut paused_join,
        &mut after_boundary,
        &mut after_stop,
    ] {
        receive(listener.as_mut(), &replay_start);
        listener.shutdown().expect("listener shutdown");
    }
    host.shutdown().expect("host shutdown");
}
