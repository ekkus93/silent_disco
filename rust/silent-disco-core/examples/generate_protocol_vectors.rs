use std::{error::Error, fs, path::Path};

use silent_disco_core::{
    domain::{DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId},
    protocol::{
        AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, Disconnect,
        FRAME_HEADER_BYTES, FRAME_HEADER_LENGTH, Heartbeat, Hello, JoinApproval, JoinRejection,
        JoinRequest, MAX_AUDIO_DATAGRAM_BYTES, MAX_CONTROL_PAYLOAD_BYTES, MessageKind, PROTOCOL_MAGIC,
        PROTOCOL_VERSION, Pause, ProtocolError, ProtocolFrame, ResyncNotice, Stop, StreamStart,
        SyncRequest, SyncResponse, crc32, encode_frame,
    },
};

fn main() -> Result<(), Box<dyn Error>> {
    let output_dir = Path::new("silent-disco-core/testdata/protocol/v2");
    fs::create_dir_all(output_dir)?;

    let session = SessionId::new("session-1")?;
    let stream = StreamId::new("stream-1")?;
    let listener = DeviceId::new("listener-1")?;

    let control_frames = vec![
        (
            "hello",
            ProtocolFrame::Control(ControlMessage::Hello(Hello {
                session_id: session.clone(),
                session_name: "Golden Session".into(),
                host_name: "Golden Host".into(),
                approval_required: true,
            })),
        ),
        (
            "join_request",
            ProtocolFrame::Control(ControlMessage::JoinRequest(JoinRequest {
                session_id: session.clone(),
                device: DeviceIdentity {
                    device_id: listener.clone(),
                    display_name: "Golden Listener".into(),
                },
                invite_code: Some("482913".into()),
            })),
        ),
        (
            "join_approval",
            ProtocolFrame::Control(ControlMessage::JoinApproval(JoinApproval {
                session_id: session.clone(),
                listener_id: listener.clone(),
                trusted_for_future: true,
            })),
        ),
        (
            "join_rejection",
            ProtocolFrame::Control(ControlMessage::JoinRejection(JoinRejection {
                session_id: session.clone(),
                listener_id: listener.clone(),
                reason: "host rejected request".into(),
            })),
        ),
        (
            "heartbeat",
            ProtocolFrame::Control(ControlMessage::Heartbeat(Heartbeat {
                session_id: session.clone(),
                listener_id: listener.clone(),
                sent_at_elapsed_ms: MonotonicMillis::new(1_234_567),
            })),
        ),
        (
            "stream_start",
            ProtocolFrame::Control(ControlMessage::StreamStart(StreamStart {
                session_id: session.clone(),
                stream_id: stream.clone(),
                host_start_time_ms: MonotonicMillis::new(2_000_000),
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 480,
            })),
        ),
        (
            "pause",
            ProtocolFrame::Control(ControlMessage::Pause(Pause {
                session_id: session.clone(),
                stream_id: stream.clone(),
                host_pause_time_ms: MonotonicMillis::new(2_010_000),
            })),
        ),
        (
            "stop",
            ProtocolFrame::Control(ControlMessage::Stop(Stop {
                session_id: session.clone(),
                stream_id: stream.clone(),
                host_stop_time_ms: MonotonicMillis::new(2_020_000),
            })),
        ),
        (
            "disconnect",
            ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
                session_id: session.clone(),
                listener_id: listener.clone(),
                reason: "listener left".into(),
            })),
        ),
        (
            "resync_notice",
            ProtocolFrame::Control(ControlMessage::ResyncNotice(ResyncNotice {
                session_id: session.clone(),
                listener_id: listener.clone(),
                reason: "periodic resync".into(),
            })),
        ),
    ];
    write_valid_vectors(output_dir.join("control_vectors.txt"), control_frames)?;

    let sync_frames = vec![
        (
            "sync_request",
            ProtocolFrame::SyncRequest(SyncRequest {
                session_id: session.clone(),
                correlation_id: 0x0102_0304_0506_0708,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(3_000_000),
            }),
        ),
        (
            "sync_response",
            ProtocolFrame::SyncResponse(SyncResponse {
                session_id: session.clone(),
                correlation_id: 0x0102_0304_0506_0708,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(3_000_000),
                t2_host_receive_elapsed_ms: MonotonicMillis::new(4_000_000),
                t3_host_send_elapsed_ms: MonotonicMillis::new(4_000_003),
            }),
        ),
    ];
    write_valid_vectors(output_dir.join("sync_vectors.txt"), sync_frames)?;

    let audio = ProtocolFrame::Audio(AudioDatagram {
        session_id: session.clone(),
        stream_id: stream.clone(),
        sequence: PacketSequence::new(42),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 4,
        first_sample_index: SampleIndex::new(16),
        host_presentation_time_ms: MonotonicMillis::new(5_000_000),
        payload: (0_u8..16).collect(),
    });
    write_valid_vectors(output_dir.join("audio_vectors.txt"), vec![("audio", audio.clone())])?;

    let max_session = SessionId::new("s".repeat(128))?;
    let max_listener = DeviceId::new("d".repeat(128))?;
    let boundary_frames = vec![
        (
            "max_hello_text_and_identifier",
            ProtocolFrame::Control(ControlMessage::Hello(Hello {
                session_id: max_session.clone(),
                session_name: "n".repeat(128),
                host_name: "h".repeat(128),
                approval_required: false,
            })),
        ),
        (
            "max_join_request_fields",
            ProtocolFrame::Control(ControlMessage::JoinRequest(JoinRequest {
                session_id: max_session.clone(),
                device: DeviceIdentity {
                    device_id: max_listener.clone(),
                    display_name: "l".repeat(128),
                },
                invite_code: Some("7".repeat(64)),
            })),
        ),
        (
            "max_reason",
            ProtocolFrame::Control(ControlMessage::JoinRejection(JoinRejection {
                session_id: max_session,
                listener_id: max_listener,
                reason: "r".repeat(256),
            })),
        ),
        ("max_audio_datagram", maximum_audio_frame(&session, &stream)?),
    ];
    write_valid_vectors(output_dir.join("boundary_vectors.txt"), boundary_frames)?;

    let hello_bytes = encode_frame(&ProtocolFrame::Control(ControlMessage::Hello(Hello {
        session_id: session.clone(),
        session_name: "Malformed Base".into(),
        host_name: "Host".into(),
        approval_required: true,
    })))?;
    let audio_bytes = encode_frame(&audio)?;
    let mut malformed = Vec::new();

    let mut bytes = hello_bytes.clone();
    bytes[0] ^= 0xff;
    malformed.push(("bad_magic", "invalid_magic", bytes));

    let mut bytes = hello_bytes.clone();
    bytes[4..6].copy_from_slice(&3_u16.to_be_bytes());
    malformed.push(("unsupported_version", "unsupported_version", bytes));

    let mut bytes = hello_bytes.clone();
    bytes[6..8].copy_from_slice(&999_u16.to_be_bytes());
    malformed.push(("unknown_kind", "unsupported_kind", bytes));

    let mut bytes = hello_bytes.clone();
    bytes[8..10].copy_from_slice(&0x8000_u16.to_be_bytes());
    malformed.push(("unsupported_flags", "unsupported_flags", bytes));

    malformed.push((
        "truncated_header",
        "truncated",
        hello_bytes[..FRAME_HEADER_BYTES - 1].to_vec(),
    ));
    malformed.push((
        "truncated_payload",
        "length_mismatch",
        hello_bytes[..hello_bytes.len() - 1].to_vec(),
    ));

    let mut bytes = hello_bytes.clone();
    bytes.push(0);
    malformed.push(("trailing_byte", "trailing_bytes", bytes));

    let mut bytes = hello_bytes[..FRAME_HEADER_BYTES].to_vec();
    bytes[12..16].copy_from_slice(&u32::try_from(MAX_CONTROL_PAYLOAD_BYTES + 1)?.to_be_bytes());
    malformed.push(("oversized_control", "oversized", bytes));

    let mut bytes = hello_bytes.clone();
    let last = bytes.len() - 1;
    bytes[last] = 2;
    malformed.push(("invalid_boolean", "invalid_boolean", bytes));

    let mut bytes = audio_bytes.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    malformed.push(("audio_integrity", "integrity_mismatch", bytes));

    let mut bytes = audio_bytes;
    let codec_offset = FRAME_HEADER_BYTES
        + 2
        + session.as_str().len()
        + 2
        + stream.as_str().len()
        + 8;
    bytes[codec_offset] = 0xff;
    malformed.push(("unsupported_audio_codec", "unsupported_codec", bytes));

    write_malformed_vectors(output_dir.join("malformed_vectors.txt"), malformed)?;
    Ok(())
}

fn maximum_audio_frame(
    session: &SessionId,
    stream: &StreamId,
) -> Result<ProtocolFrame, Box<dyn Error>> {
    let mut best = None;
    for samples_per_packet in 1_u32..=4_096 {
        let payload_length = usize::try_from(samples_per_packet)?.checked_mul(2).ok_or(
            ProtocolError::InvalidField {
                field: "samples_per_packet",
            },
        )?;
        let payload = (0..payload_length)
            .map(|index| index.to_le_bytes()[0].wrapping_mul(31))
            .collect();
        let frame = ProtocolFrame::Audio(AudioDatagram {
            session_id: session.clone(),
            stream_id: stream.clone(),
            sequence: PacketSequence::new(999),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 1,
            samples_per_packet,
            first_sample_index: SampleIndex::new(10_000),
            host_presentation_time_ms: MonotonicMillis::new(6_000_000),
            payload,
        });
        match encode_frame(&frame) {
            Ok(bytes) if bytes.len() <= MAX_AUDIO_DATAGRAM_BYTES => best = Some(frame),
            Err(ProtocolError::PayloadTooLarge { .. }) => break,
            Ok(_) => break,
            Err(error) => return Err(error.into()),
        }
    }
    best.ok_or_else(|| "failed to construct bounded audio datagram".into())
}

fn write_valid_vectors(
    path: impl AsRef<Path>,
    vectors: Vec<(&'static str, ProtocolFrame)>,
) -> Result<(), Box<dyn Error>> {
    let mut output = String::from("# name|kind|frame_crc32|payload_crc32|hex\n");
    for (name, frame) in vectors {
        let bytes = encode_frame(&frame)?;
        let payload = &bytes[FRAME_HEADER_BYTES..];
        output.push_str(&format!(
            "{name}|{}|{:08x}|{:08x}|{}\n",
            frame.kind().stable_name(),
            crc32(&bytes),
            crc32(payload),
            encode_hex(&bytes)
        ));
    }
    fs::write(path, output)?;
    Ok(())
}

fn write_malformed_vectors(
    path: impl AsRef<Path>,
    vectors: Vec<(&'static str, &'static str, Vec<u8>)>,
) -> Result<(), Box<dyn Error>> {
    let mut output = String::from("# name|expected_error|hex\n");
    for (name, expected_error, bytes) in vectors {
        output.push_str(&format!(
            "{name}|{expected_error}|{}\n",
            encode_hex(&bytes)
        ));
    }
    fs::write(path, output)?;
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

const _: () = {
    assert!(FRAME_HEADER_LENGTH as usize == FRAME_HEADER_BYTES);
    assert!(PROTOCOL_MAGIC.len() == 4);
    assert!(PROTOCOL_VERSION == 2);
    assert!(MessageKind::Audio.code() == 200);
};
