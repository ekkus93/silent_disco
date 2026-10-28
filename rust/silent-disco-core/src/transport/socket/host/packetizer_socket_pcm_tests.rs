//! Packetizer-to-production-socket PCM proof with a boundary inside a datagram.
//! This is real loopback TCP/UDP, not a hardware DAC or native callback test.
use std::sync::Arc;

use crate::audio::{AudioFormat, AudioSampleFormat, DecodedPcmChunk, Packetizer};
use crate::domain::{MonotonicMillis, SampleIndex, StreamId};
use crate::protocol::{ControlMessage, ProtocolFrame, TrackBoundary};
use crate::transport::test_support::{
    id_device, id_session, join_request, wait_for_authorized, wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportFactory,
};

use super::SocketHostTransport;

fn join_listener(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let session = id_session("packetizer-socket-exact-pcm");
    let device = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect production listener");
    listener
        .send_control(&join_request(&session, &device, name))
        .expect("send join");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(host, &device);
    listener
}

fn expect_wire_frame(listener: &mut dyn ListenerTransportNode, frame: &ProtocolFrame) {
    let channel = match frame {
        ProtocolFrame::Audio(_) => TransportChannel::Audio,
        _ => TransportChannel::Control,
    };
    wait_for_frame(listener, channel, |received| received == frame);
}

#[test]
fn two_tracks_crossing_packet_boundary_preserve_exact_pcm_and_late_join_cache() {
    let session = id_session("packetizer-socket-exact-pcm");
    let stream = StreamId::new("packetizer-socket-stream").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP/UDP host");
    let mut first = join_listener(&mut host, &clock, "pcm-first");

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
    let start = packetizer.stream_start_message();
    let ProtocolFrame::Control(ref start_control) = start else {
        panic!("packetizer must produce StreamStart");
    };
    assert_eq!(
        host.broadcast_control(start_control)
            .expect("broadcast start")
            .report
            .successful_peers,
        1,
    );
    expect_wire_frame(first.as_mut(), &start);

    // Track A ends halfway through packet #1. Track B must fill the carry
    // without silence, duplicated samples or a second StreamStart.
    let first_pcm: Vec<i16> = (0..360)
        .flat_map(|i| {
            let sample = i16::try_from(i).expect("fixture index fits i16");
            [sample - 200, 200 - sample]
        })
        .collect();
    let second_pcm: Vec<i16> = (0..360)
        .flat_map(|i| {
            let sample = i16::try_from(i).expect("fixture index fits i16");
            [sample + 1_000, -1_000 - sample]
        })
        .collect();
    let first_outcome = packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: first_pcm.clone(),
            end_of_stream: false,
        })
        .expect("packetize first track");
    assert_eq!(first_outcome.frames.len(), 1);
    assert_eq!(packetizer.next_contiguous_sample_index(), 360);
    for frame in &first_outcome.frames {
        host.broadcast_audio(frame).expect("send first packet");
        expect_wire_frame(first.as_mut(), frame);
    }

    let boundary = ProtocolFrame::Control(ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream,
        first_sample_index: SampleIndex::new(360),
        entry_id: "track-b".to_owned(),
        display_name: "Track B".to_owned(),
    }));
    let ProtocolFrame::Control(ref boundary_control) = boundary else {
        panic!("boundary fixture");
    };
    host.broadcast_control(boundary_control)
        .expect("broadcast one sample-indexed boundary");
    expect_wire_frame(first.as_mut(), &boundary);

    // Late join receives one original stream anchor and the still-future
    // boundary. It must not receive already-sent packet zero.
    let mut late = join_listener(&mut host, &clock, "pcm-late");
    expect_wire_frame(late.as_mut(), &start);
    expect_wire_frame(late.as_mut(), &boundary);

    let second_outcome = packetizer
        .push_contiguous_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0), // decoder-local index
            frames: second_pcm.clone(),
            end_of_stream: true,
        })
        .expect("packetize second track with carry");
    assert!(second_outcome.end_of_stream);
    assert_eq!(second_outcome.frames.len(), 2);
    assert_eq!(packetizer.next_contiguous_sample_index(), 720);

    for frame in &second_outcome.frames {
        assert_eq!(
            host.broadcast_audio(frame)
                .expect("broadcast contiguous PCM")
                .report
                .successful_peers,
            2,
        );
        expect_wire_frame(first.as_mut(), frame);
        expect_wire_frame(late.as_mut(), frame);
    }

    let frames: Vec<_> = first_outcome
        .frames
        .into_iter()
        .chain(second_outcome.frames)
        .collect();
    let mut decoded = Vec::new();
    for (sequence, frame) in frames.iter().enumerate() {
        let ProtocolFrame::Audio(packet) = frame else {
            panic!("packetizer emitted non-audio");
        };
        assert_eq!(packet.sequence.get(), sequence as u64);
        assert_eq!(packet.first_sample_index.get(), sequence as u64 * 240);
        assert_eq!(
            packet.host_presentation_time_ms.get(),
            1_000 + sequence as u64 * 5
        );
        assert_eq!(packet.samples_per_packet, 240);
        assert_eq!(packet.payload.len(), 240 * 2 * 2);
        decoded.extend(
            packet
                .payload
                .chunks_exact(2)
                .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]])),
        );
    }
    assert_eq!(decoded, [first_pcm, second_pcm].concat());
    first.shutdown().expect("shutdown first");
    late.shutdown().expect("shutdown late");
    host.shutdown().expect("shutdown host");
}

#[test]
fn real_socket_pause_resume_stop_and_replay_preserve_multi_listener_pcm_generation() {
    use crate::protocol::{Pause, Stop, StreamStart};

    let session = id_session("packetizer-socket-exact-pcm");
    let stream = StreamId::new("pause-resume-original").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let mut first = join_listener(&mut host, &clock, "pause-first");
    let format = AudioFormat {
        sample_rate_hz: 48_000,
        channels: 2,
        sample_format: AudioSampleFormat::PcmS16Le,
    };
    let original = Packetizer::new(
        session.clone(),
        stream.clone(),
        format,
        5,
        MonotonicMillis::new(1_000),
    )
    .expect("original packetizer");
    let ProtocolFrame::Control(ControlMessage::StreamStart(start)) =
        original.stream_start_message()
    else {
        panic!("expected original stream start");
    };
    host.broadcast_control(&ControlMessage::StreamStart(start.clone()))
        .expect("start original stream");
    expect_wire_frame(
        first.as_mut(),
        &ProtocolFrame::Control(ControlMessage::StreamStart(start.clone())),
    );

    let boundary = TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(240),
        entry_id: "second-track".to_owned(),
        display_name: "Second track".to_owned(),
    };
    host.broadcast_control(&ControlMessage::TrackBoundary(boundary.clone()))
        .expect("queue boundary");
    expect_wire_frame(
        first.as_mut(),
        &ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary.clone())),
    );

    let pause = Pause {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_050),
    };
    host.broadcast_control(&ControlMessage::Pause(pause.clone()))
        .expect("pause");
    expect_wire_frame(
        first.as_mut(),
        &ProtocolFrame::Control(ControlMessage::Pause(pause.clone())),
    );

    let mut late = join_listener(&mut host, &clock, "pause-late");
    for control in [
        ControlMessage::StreamStart(start.clone()),
        ControlMessage::TrackBoundary(boundary.clone()),
        ControlMessage::Pause(pause),
    ] {
        expect_wire_frame(late.as_mut(), &ProtocolFrame::Control(control));
    }

    let resumed = StreamStart {
        host_start_time_ms: MonotonicMillis::new(1_200),
        ..start
    };
    host.broadcast_control(&ControlMessage::StreamStart(resumed.clone()))
        .expect("resume same stream");
    for listener in [&mut first, &mut late] {
        expect_wire_frame(
            listener.as_mut(),
            &ProtocolFrame::Control(ControlMessage::StreamStart(resumed.clone())),
        );
    }
    let mut resumed_late = join_listener(&mut host, &clock, "resumed-late");
    for control in [
        ControlMessage::StreamStart(resumed),
        ControlMessage::TrackBoundary(boundary),
    ] {
        expect_wire_frame(resumed_late.as_mut(), &ProtocolFrame::Control(control));
    }

    let stop = Stop {
        session_id: session.clone(),
        stream_id: stream,
        host_stop_time_ms: MonotonicMillis::new(1_250),
    };
    host.broadcast_control(&ControlMessage::Stop(stop.clone()))
        .expect("stop original stream");
    for listener in [&mut first, &mut late, &mut resumed_late] {
        expect_wire_frame(
            listener.as_mut(),
            &ProtocolFrame::Control(ControlMessage::Stop(stop.clone())),
        );
    }
    assert!(
        host.playback_controls
            .lock()
            .expect("playback cache")
            .snapshot_at(MonotonicMillis::new(1_300))
            .is_empty(),
        "stopped host must not replay retired start or boundary"
    );
    let mut after_stop = join_listener(&mut host, &clock, "after-stop");

    let replay_stream = StreamId::new("replay-generation").expect("replay stream");
    let mut replay = Packetizer::new(
        session,
        replay_stream,
        format,
        5,
        MonotonicMillis::new(2_000),
    )
    .expect("replay packetizer");
    let replay_start = replay.stream_start_message();
    let ProtocolFrame::Control(ref replay_control) = replay_start else {
        panic!("expected replay stream start");
    };
    assert_eq!(
        host.broadcast_control(replay_control)
            .expect("start replay")
            .report
            .successful_peers,
        4
    );
    for listener in [&mut first, &mut late, &mut resumed_late, &mut after_stop] {
        expect_wire_frame(listener.as_mut(), &replay_start);
    }
    let samples: Vec<i16> = (0..240)
        .flat_map(|i| {
            let sample = i16::try_from(i).expect("sample");
            [sample + 500, -sample - 500]
        })
        .collect();
    let outcome = replay
        .push_contiguous_chunk(DecodedPcmChunk {
            format,
            first_sample_index: SampleIndex::new(0),
            frames: samples.clone(),
            end_of_stream: true,
        })
        .expect("packetize replay");
    assert_eq!(outcome.frames.len(), 1);
    let packet = &outcome.frames[0];
    assert_eq!(
        host.broadcast_audio(packet)
            .expect("broadcast replay audio")
            .report
            .successful_peers,
        4
    );
    for listener in [&mut first, &mut late, &mut resumed_late, &mut after_stop] {
        expect_wire_frame(listener.as_mut(), packet);
    }
    let ProtocolFrame::Audio(audio) = packet else {
        panic!("expected replay audio");
    };
    assert_eq!(audio.sequence.get(), 0);
    assert_eq!(audio.first_sample_index.get(), 0);
    let decoded: Vec<i16> = audio
        .payload
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    assert_eq!(decoded, samples);
    first.shutdown().expect("first shutdown");
    late.shutdown().expect("late shutdown");
    resumed_late.shutdown().expect("resumed late shutdown");
    after_stop.shutdown().expect("post-stop shutdown");
    host.shutdown().expect("host shutdown");
}
