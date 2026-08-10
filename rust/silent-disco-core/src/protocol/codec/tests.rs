use super::{
    DecodePolicy, ParseFailureClass, ProtocolDecoder, ProtocolError, crc32, decode_frame,
    decode_header, encode_frame,
};
use crate::{
    domain::{
        DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId, SyncConfidence,
    },
    protocol::{
        AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, FRAME_HEADER_BYTES,
        FRAME_HEADER_LENGTH, Hello, JoinRequest, MAX_CONTROL_PAYLOAD_BYTES, MessageKind,
        PROTOCOL_MAGIC, PROTOCOL_VERSION, ProtocolFrame, SyncRequest, SyncResponse,
        SynchronizationReport,
    },
};

fn session_id() -> SessionId {
    SessionId::new("session-1").expect("test session identifier")
}

fn stream_id() -> StreamId {
    StreamId::new("stream-1").expect("test stream identifier")
}

fn device_id() -> DeviceId {
    DeviceId::new("device-1").expect("test device identifier")
}

fn hello_frame() -> ProtocolFrame {
    ProtocolFrame::Control(ControlMessage::Hello(Hello {
        session_id: session_id(),
        session_name: "Test Session".into(),
        host_name: "Host Phone".into(),
        approval_required: true,
    }))
}

#[test]
fn header_is_fixed_width_and_network_ordered() {
    let encoded = encode_frame(&hello_frame()).expect("hello frame encodes");
    assert_eq!(&encoded[..4], &PROTOCOL_MAGIC);
    assert_eq!(
        u16::from_be_bytes([encoded[4], encoded[5]]),
        PROTOCOL_VERSION
    );
    assert_eq!(
        u16::from_be_bytes([encoded[6], encoded[7]]),
        MessageKind::Hello.code()
    );
    assert_eq!(
        u16::from_be_bytes([encoded[10], encoded[11]]) as usize,
        FRAME_HEADER_BYTES
    );
    let header = decode_header(&encoded).expect("header decodes");
    assert_eq!(header.kind, MessageKind::Hello);
}

#[test]
fn control_sync_and_audio_round_trip_canonically() {
    let frames = vec![
        hello_frame(),
        ProtocolFrame::Control(ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id(),
            device: DeviceIdentity {
                device_id: device_id(),
                display_name: "Listener".into(),
            },
            invite_code: Some("123456".into()),
            sync_port: 41_101,
            audio_port: 41_102,
        })),
        ProtocolFrame::SyncRequest(SyncRequest {
            session_id: session_id(),
            correlation_id: 7,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1_000),
        }),
        ProtocolFrame::SyncResponse(SyncResponse {
            session_id: session_id(),
            correlation_id: 7,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1_000),
            t2_host_receive_elapsed_ms: MonotonicMillis::new(2_000),
            t3_host_send_elapsed_ms: MonotonicMillis::new(2_001),
        }),
        // Negative offset and fractional values on purpose: this is the
        // only control message carrying `f64` fields (D2, `docs/
        // AUDIO_PLAYBACK_STATE_2026-08-10.md`), and a listener genuinely
        // running ahead of the host's clock reports a negative offset.
        ProtocolFrame::Control(ControlMessage::SynchronizationReport(
            SynchronizationReport {
                session_id: session_id(),
                listener_id: device_id(),
                confidence: SyncConfidence::Excellent,
                offset_ms: -12.375,
                round_trip_ms: 24.5,
                drift_ppm: -3.125,
            },
        )),
        ProtocolFrame::Audio(AudioDatagram {
            session_id: session_id(),
            stream_id: stream_id(),
            sequence: PacketSequence::new(9),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 2,
            first_sample_index: SampleIndex::new(4),
            host_presentation_time_ms: MonotonicMillis::new(3_000),
            payload: vec![0, 1, 2, 3, 4, 5, 6, 7],
        }),
    ];

    for frame in frames {
        let encoded = encode_frame(&frame).expect("frame encodes");
        let decoded = decode_frame(&encoded).expect("frame decodes");
        assert!(decoded == frame);
        assert_eq!(encode_frame(&decoded), Ok(encoded));
    }
}

#[test]
fn rejects_unknown_version_kind_flags_and_oversized_length_from_header_only() {
    let mut bytes = encode_frame(&hello_frame()).expect("hello frame encodes");
    bytes[4..6].copy_from_slice(&3_u16.to_be_bytes());
    assert!(matches!(
        decode_frame(&bytes),
        Err(ProtocolError::UnsupportedVersion { version: 3 })
    ));

    let mut bytes = encode_frame(&hello_frame()).expect("hello frame encodes");
    bytes[6..8].copy_from_slice(&999_u16.to_be_bytes());
    assert!(matches!(
        decode_frame(&bytes),
        Err(ProtocolError::UnsupportedMessageKind { kind: 999 })
    ));

    let mut bytes = encode_frame(&hello_frame()).expect("hello frame encodes");
    bytes[8..10].copy_from_slice(&0x8000_u16.to_be_bytes());
    assert!(matches!(
        decode_frame(&bytes),
        Err(ProtocolError::UnsupportedFlags { flags: 0x8000 })
    ));

    let mut header_only = [0_u8; FRAME_HEADER_BYTES];
    header_only[..4].copy_from_slice(&PROTOCOL_MAGIC);
    header_only[4..6].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    header_only[6..8].copy_from_slice(&MessageKind::Hello.code().to_be_bytes());
    header_only[10..12].copy_from_slice(&FRAME_HEADER_LENGTH.to_be_bytes());
    header_only[12..16].copy_from_slice(
        &u32::try_from(MAX_CONTROL_PAYLOAD_BYTES + 1)
            .expect("test length")
            .to_be_bytes(),
    );
    assert!(matches!(
        decode_frame(&header_only),
        Err(ProtocolError::PayloadTooLarge { .. })
    ));
}

#[test]
fn rejects_truncation_trailing_bytes_and_integrity_failure() {
    let encoded = encode_frame(&hello_frame()).expect("hello frame encodes");
    assert!(matches!(
        decode_frame(&encoded[..encoded.len() - 1]),
        Err(ProtocolError::LengthMismatch { .. })
    ));
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        decode_frame(&trailing),
        Err(ProtocolError::TrailingBytes)
    ));

    let audio = ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id(),
        stream_id: stream_id(),
        sequence: PacketSequence::new(1),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 1,
        samples_per_packet: 2,
        first_sample_index: SampleIndex::new(0),
        host_presentation_time_ms: MonotonicMillis::new(100),
        payload: vec![1, 2, 3, 4],
    });
    let mut encoded = encode_frame(&audio).expect("audio frame encodes");
    let last = encoded.len() - 1;
    encoded[last] ^= 0xff;
    assert!(matches!(
        decode_frame(&encoded),
        Err(ProtocolError::IntegrityMismatch)
    ));
}

#[test]
fn decoder_policy_counts_each_failure_class() {
    let expected = session_id();
    let other = SessionId::new("session-other").expect("other session identifier");
    let unauthorized = ProtocolFrame::Control(ControlMessage::Hello(Hello {
        session_id: other,
        session_name: "Other Session".into(),
        host_name: "Other Host".into(),
        approval_required: false,
    }));
    let mut decoder = ProtocolDecoder::default();
    let unauthorized_bytes = encode_frame(&unauthorized).expect("unauthorized frame encodes");
    assert!(matches!(
        decoder.decode(
            &unauthorized_bytes,
            DecodePolicy {
                expected_session_id: Some(&expected),
                minimum_audio_sequence: None,
            },
        ),
        Err(ProtocolError::UnauthorizedSession)
    ));

    let stale = ProtocolFrame::Audio(AudioDatagram {
        session_id: expected.clone(),
        stream_id: stream_id(),
        sequence: PacketSequence::new(4),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 1,
        samples_per_packet: 1,
        first_sample_index: SampleIndex::new(0),
        host_presentation_time_ms: MonotonicMillis::new(0),
        payload: vec![0, 0],
    });
    let stale_bytes = encode_frame(&stale).expect("stale frame encodes");
    assert!(matches!(
        decoder.decode(
            &stale_bytes,
            DecodePolicy {
                expected_session_id: Some(&expected),
                minimum_audio_sequence: Some(PacketSequence::new(5)),
            },
        ),
        Err(ProtocolError::StaleAudioSequence)
    ));

    assert_eq!(decoder.counters().unauthorized, 1);
    assert_eq!(decoder.counters().stale, 1);
    assert_eq!(
        ProtocolError::StaleAudioSequence.classification(),
        ParseFailureClass::Stale
    );
}

#[test]
fn crc32_matches_standard_check_value() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[test]
fn arbitrary_inputs_do_not_panic_or_allocate_from_untrusted_lengths() {
    let mut state = 0x1234_5678_9abc_def0_u64;
    for length in 0..=512 {
        let mut bytes = vec![0_u8; length];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state.to_le_bytes()[0];
        }
        let _ = decode_frame(&bytes);
    }
}
