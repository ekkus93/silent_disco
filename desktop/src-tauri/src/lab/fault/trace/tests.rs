use super::{
    MAX_TRANSPORT_FACTS, RecordedFaultDecision, RecordedFrameHashScope, TransportFactKind,
    TransportTrace, TransportTraceRecorder,
};
use crate::lab::clock::{LabClock, LabNodeClock};
use crate::lab::fault::{LabFaultController, LabLatencyConfig, LabLatencyTransportFactory};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
    encode_frame,
};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    TransportChannel, TransportClock, TransportErrorKind, TransportEvent, TransportFactory,
    TransportPeer, VirtualTransportFactory, VirtualTransportNetwork,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

fn audio_event(payload: Vec<u8>) -> TransportEvent {
    let samples_per_packet =
        u32::try_from(payload.len() / 4).expect("test payload sample count fits u32");
    TransportEvent::FrameReceived {
        channel: TransportChannel::Audio,
        peer: TransportPeer {
            device_id: None,
            control_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12_345),
        },
        frame: ProtocolFrame::Audio(AudioDatagram {
            session_id: SessionId::new("trace-session").expect("valid session ID"),
            stream_id: StreamId::new("trace-stream").expect("valid stream ID"),
            sequence: PacketSequence::new(9),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet,
            first_sample_index: SampleIndex::new(36),
            host_presentation_time_ms: MonotonicMillis::new(1_234),
            payload,
        }),
        received_at: MonotonicMillis::new(900),
    }
}

#[test]
fn audio_packet_records_metadata_and_hashes_but_never_raw_payload() {
    let recorder = TransportTraceRecorder::new();
    let payload = b"BLOCK41_AUDIO_SECRET".to_vec();
    let event = audio_event(payload.clone());
    let expected_encoded_length = match &event {
        TransportEvent::FrameReceived { frame, .. } => {
            u64::try_from(encode_frame(frame).expect("test frame encodes").len())
                .expect("encoded test frame length fits u64")
        }
        _ => unreachable!("helper always returns a frame"),
    };

    recorder
        .record_packet("listener1", &event)
        .expect("packet fact records");
    let trace = recorder.snapshot().expect("trace snapshot");

    assert_eq!(trace.facts.len(), 1);
    let TransportFactKind::Packet {
        receiver_node,
        channel,
        message_kind,
        encoded_length,
        frame_sha256,
        frame_hash_scope,
        audio: Some(audio),
        ..
    } = &trace.facts[0].entry
    else {
        panic!("expected one audio packet fact");
    };
    assert_eq!(receiver_node, "listener1");
    assert_eq!(channel, "audio");
    assert_eq!(message_kind, "audio");
    assert_eq!(*encoded_length, expected_encoded_length);
    assert_eq!(frame_sha256.len(), 64);
    assert_eq!(*frame_hash_scope, RecordedFrameHashScope::FullFrame);
    assert_eq!(audio.sequence, 9);
    assert_eq!(audio.first_sample_index, 36);
    assert_eq!(
        audio.payload_length,
        u64::try_from(payload.len()).expect("payload length fits u64")
    );
    assert_eq!(
        audio.payload_sha256,
        "be13ccb0e4fd2cd44ba6b37c55ed2d1b769ab92704fa7ffa49e2e93cededda7d"
    );

    let json = serde_json::to_string(&trace).expect("trace serializes");
    assert!(
        !json.contains("BLOCK41_AUDIO_SECRET"),
        "recordings must never persist raw audio bytes"
    );
    let decoded: TransportTrace = serde_json::from_str(&json).expect("trace deserializes");
    assert_eq!(decoded, trace);
}

fn join_request_event(invite_code: &str) -> TransportEvent {
    let session_id = SessionId::new("trace-secret-session").expect("valid session ID");
    let device_id =
        silent_disco_core::domain::DeviceId::new("trace-secret-listener").expect("valid device ID");
    TransportEvent::FrameReceived {
        channel: TransportChannel::Control,
        peer: TransportPeer {
            device_id: Some(device_id.clone()),
            control_address: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12_346),
        },
        frame: ProtocolFrame::Control(ControlMessage::JoinRequest(JoinRequest {
            session_id,
            device: DeviceIdentity {
                device_id,
                display_name: "Secret Trace Listener".to_owned(),
            },
            invite_code: Some(invite_code.to_owned()),
            sync_port: 12_347,
            audio_port: 12_348,
        })),
        received_at: MonotonicMillis::new(901),
    }
}

#[test]
fn secret_control_hash_redacts_invite_code_before_persisting_a_verifier() {
    let recorder = TransportTraceRecorder::new();
    recorder
        .record_packet("host1", &join_request_event("123456"))
        .expect("first packet records");
    recorder
        .record_packet("host1", &join_request_event("654321"))
        .expect("second packet records");
    let trace = recorder.snapshot().expect("trace snapshot");

    let hashes: Vec<(&str, RecordedFrameHashScope)> = trace
        .facts
        .iter()
        .filter_map(|fact| match &fact.entry {
            TransportFactKind::Packet {
                frame_sha256,
                frame_hash_scope,
                ..
            } => Some((frame_sha256.as_str(), *frame_hash_scope)),
            TransportFactKind::FaultDecision { .. } => None,
        })
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_eq!(hashes[0].0, hashes[1].0);
    assert!(
        hashes
            .iter()
            .all(|(_, scope)| *scope == RecordedFrameHashScope::RedactedSensitiveFields)
    );

    let json = serde_json::to_string(&trace).expect("trace serializes");
    assert!(!json.contains("123456"));
    assert!(!json.contains("654321"));
}

#[test]
fn transport_trace_overflow_is_counted_instead_of_silently_discarded() {
    let recorder = TransportTraceRecorder::new();
    let event = audio_event(vec![0, 1, 2, 3]);

    for _ in 0..(MAX_TRANSPORT_FACTS + 3) {
        recorder
            .record_packet("listener1", &event)
            .expect("bounded recorder remains operational");
    }
    let trace = recorder.snapshot().expect("trace snapshot");

    assert_eq!(trace.facts.len(), MAX_TRANSPORT_FACTS);
    assert_eq!(trace.dropped_count, 3);
    assert_eq!(trace.facts[0].sequence, 0);
    assert_eq!(
        trace.facts[MAX_TRANSPORT_FACTS - 1].sequence,
        u64::try_from(MAX_TRANSPORT_FACTS - 1).expect("bound fits u64")
    );
}

const POLL_TIMEOUT: Duration = Duration::from_millis(25);

type TracedEndpoints = (
    Arc<LabClock>,
    TransportTraceRecorder,
    LabFaultController,
    Box<dyn HostTransportNode>,
    Box<dyn ListenerTransportNode>,
    SessionId,
);

fn clock_handle(clock: &Arc<LabClock>) -> Arc<dyn TransportClock> {
    Arc::new(LabNodeClock::new(Arc::clone(clock), 0, 0).expect("valid test clock"))
}

fn bind_traced_listener(latency_ms: u64, loss_permille: u16) -> TracedEndpoints {
    let session_id = SessionId::new("trace-live-session").expect("valid session ID");
    let device_id =
        silent_disco_core::domain::DeviceId::new("trace-live-listener").expect("valid device ID");
    let clock = Arc::new(LabClock::new(1_000));
    let recorder = TransportTraceRecorder::new();
    let network = VirtualTransportNetwork::default();
    let mut host = VirtualTransportFactory::new(network.clone())
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock_handle(&clock),
        )
        .expect("host binds");
    let controller = LabFaultController::new_traced(
        LabLatencyConfig {
            fixed_latency_ms: latency_ms,
            jitter_ms: 0,
            seed: 77,
        },
        loss_permille,
        "listener1".to_owned(),
        recorder.clone(),
    );
    let factory = LabLatencyTransportFactory::new_dynamic(
        VirtualTransportFactory::new(network),
        Arc::clone(&clock),
        controller.clone(),
    );
    let listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock_handle(&clock),
        )
        .expect("listener connects");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT).expect("peer accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: device_id.clone(),
                display_name: "Trace Listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("join sends");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT).expect("join arrives"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            ..
        }
    ));
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("listener authorizes");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT)
            .expect("authorization arrives"),
        TransportEvent::PeerAuthorized { .. }
    ));
    (clock, recorder, controller, host, listener, session_id)
}

fn fault_decisions(trace: &TransportTrace) -> Vec<RecordedFaultDecision> {
    trace
        .facts
        .iter()
        .filter_map(|fact| match &fact.entry {
            TransportFactKind::FaultDecision { decision, .. } => Some(*decision),
            TransportFactKind::Packet { .. } => None,
        })
        .collect()
}

#[test]
fn zero_fault_audio_records_a_real_pass_decision() {
    let (_clock, recorder, _controller, mut host, mut listener, session_id) =
        bind_traced_listener(0, 0);
    host.broadcast_audio(&audio_frame_for_session(&session_id, 1))
        .expect("audio sends");
    assert!(matches!(
        listener.recv_event(POLL_TIMEOUT).expect("audio arrives"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            ..
        }
    ));

    assert_eq!(
        fault_decisions(&recorder.snapshot().expect("trace snapshot")),
        vec![RecordedFaultDecision::Pass]
    );
    listener.shutdown().expect("listener shuts down");
    host.shutdown().expect("host shuts down");
}

#[test]
fn one_hundred_percent_loss_records_the_actual_drop_decision() {
    let (_clock, recorder, _controller, mut host, mut listener, session_id) =
        bind_traced_listener(0, 1_000);
    host.broadcast_audio(&audio_frame_for_session(&session_id, 2))
        .expect("audio sends locally");
    let Err(error) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("100% loss must suppress the datagram");
    };
    assert_eq!(error.kind, TransportErrorKind::Timeout);

    assert_eq!(
        fault_decisions(&recorder.snapshot().expect("trace snapshot")),
        vec![RecordedFaultDecision::Drop]
    );
    listener.shutdown().expect("listener shuts down");
    host.shutdown().expect("host shuts down");
}

#[test]
fn latency_records_hold_then_release_at_the_original_deadline() {
    let (clock, recorder, controller, mut host, mut listener, session_id) =
        bind_traced_listener(100, 0);
    host.broadcast_audio(&audio_frame_for_session(&session_id, 3))
        .expect("audio sends");
    let Err(held) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("latency must hold the datagram");
    };
    assert_eq!(held.kind, TransportErrorKind::Timeout);
    controller.update(0, 0, 0);
    clock.advance(99).expect("advance short of deadline");
    let Err(still_held) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("packet must remain held before its deadline");
    };
    assert_eq!(still_held.kind, TransportErrorKind::Timeout);
    clock.advance(1).expect("advance to deadline");
    assert!(matches!(
        listener.recv_event(POLL_TIMEOUT).expect("packet releases"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            ..
        }
    ));

    let trace = recorder.snapshot().expect("trace snapshot");
    assert_eq!(
        fault_decisions(&trace),
        vec![RecordedFaultDecision::Hold, RecordedFaultDecision::Release]
    );
    let release = trace.facts.iter().find_map(|fact| match &fact.entry {
        TransportFactKind::FaultDecision {
            packet_fact_sequence,
            decided_at_ms,
            profile,
            decision: RecordedFaultDecision::Release,
            deadline_ms,
            ..
        } => Some((
            *packet_fact_sequence,
            *decided_at_ms,
            profile.fixed_latency_ms,
            *deadline_ms,
        )),
        _ => None,
    });
    assert_eq!(release, Some((0, 1_100, 100, Some(1_100))));
    listener.shutdown().expect("listener shuts down");
    host.shutdown().expect("host shuts down");
}

fn audio_frame_for_session(session_id: &SessionId, sequence: u64) -> ProtocolFrame {
    ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id.clone(),
        stream_id: StreamId::new("trace-live-stream").expect("valid stream ID"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 2,
        first_sample_index: SampleIndex::new(sequence.saturating_mul(2)),
        host_presentation_time_ms: MonotonicMillis::new(sequence),
        payload: vec![0, 0, 1, 0, 2, 0, 3, 0],
    })
}
