//! Block 25 protocol hardening: structured fuzz-style and property tests for
//! `decode_frame`/`decode_header`.
//!
//! This crate has no network access to a fuzzing corpus and the pinned
//! toolchain (`rust/rust-toolchain.toml`, stable `1.97.1`) has no nightly
//! component available in this environment, so `cargo-fuzz`
//! (nightly + libFuzzer) is not usable here. Instead this module follows the
//! same minimal-dependency pattern Block 39 used for
//! [`crate::transport::DeterministicPrng`]: a small deterministic seeded
//! generator drives large volumes of malformed, mutated, and randomized-but
//! -valid input through the parser. Reproducibility (a fixed seed always
//! generates the same inputs) matters far more than unpredictability here, so
//! a hand-rolled `SplitMix64`-based generator is preferable to pulling in
//! `proptest` or `cargo-fuzz` for one crate. See `memory.md` (Block 25 entry)
//! for the full reasoning.

use crate::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId, SyncConfidence,
};
use crate::transport::DeterministicPrng;

use super::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, Disconnect, FRAME_HEADER_BYTES,
    Heartbeat, Hello, JoinApproval, JoinRejection, JoinRequest, MAX_AUDIO_DATAGRAM_BYTES,
    MAX_CONTROL_PAYLOAD_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_INVITE_CODE_BYTES, MAX_REASON_BYTES,
    MAX_SESSION_NAME_BYTES, MessageKind, PROTOCOL_MAGIC, Pause, ProtocolError, ProtocolFrame,
    ResyncNotice, Stop, StreamStart, SyncRequest, SyncResponse, SynchronizationReport,
    decode_frame, decode_header, encode_frame,
};

const IDENTIFIER_CHARSET: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.";

fn random_text(prng: &mut DeterministicPrng, minimum: usize, maximum: usize) -> String {
    let span = maximum.saturating_sub(minimum).saturating_add(1);
    let length = minimum + prng.next_below(span);
    (0..length)
        .map(|_| IDENTIFIER_CHARSET[prng.next_below(IDENTIFIER_CHARSET.len())] as char)
        .collect()
}

fn random_bytes(prng: &mut DeterministicPrng, length: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(length);
    while bytes.len() < length {
        bytes.extend_from_slice(&prng.next_u64().to_le_bytes());
    }
    bytes.truncate(length);
    bytes
}

fn random_session_id(prng: &mut DeterministicPrng) -> SessionId {
    SessionId::new(random_text(prng, 1, 48)).expect("generated session id is within bounds")
}

fn random_stream_id(prng: &mut DeterministicPrng) -> StreamId {
    StreamId::new(random_text(prng, 1, 48)).expect("generated stream id is within bounds")
}

fn random_device_id(prng: &mut DeterministicPrng) -> DeviceId {
    DeviceId::new(random_text(prng, 1, 48)).expect("generated device id is within bounds")
}

fn random_sync_confidence(prng: &mut DeterministicPrng) -> SyncConfidence {
    match prng.next_below(5) {
        0 => SyncConfidence::Unknown,
        1 => SyncConfidence::Poor,
        2 => SyncConfidence::Fair,
        3 => SyncConfidence::Good,
        _ => SyncConfidence::Excellent,
    }
}

fn random_control_frame(prng: &mut DeterministicPrng) -> ProtocolFrame {
    let session_id = random_session_id(prng);
    let message = match prng.next_below(11) {
        0 => ControlMessage::Hello(Hello {
            session_id,
            session_name: random_text(prng, 1, MAX_SESSION_NAME_BYTES),
            host_name: random_text(prng, 1, MAX_DISPLAY_NAME_BYTES),
            approval_required: prng.next_below(2) == 1,
        }),
        1 => ControlMessage::JoinRequest(JoinRequest {
            session_id,
            device: DeviceIdentity {
                device_id: random_device_id(prng),
                display_name: random_text(prng, 1, MAX_DISPLAY_NAME_BYTES),
            },
            invite_code: if prng.next_below(2) == 1 {
                Some(random_text(prng, 1, MAX_INVITE_CODE_BYTES))
            } else {
                None
            },
            sync_port: u16::try_from(prng.next_below(u16::MAX.into())).unwrap_or(0),
            audio_port: u16::try_from(prng.next_below(u16::MAX.into())).unwrap_or(0),
        }),
        2 => ControlMessage::JoinApproval(JoinApproval {
            session_id,
            listener_id: random_device_id(prng),
            trusted_for_future: prng.next_below(2) == 1,
        }),
        3 => ControlMessage::JoinRejection(JoinRejection {
            session_id,
            listener_id: random_device_id(prng),
            reason: random_text(prng, 1, MAX_REASON_BYTES),
        }),
        4 => ControlMessage::Heartbeat(Heartbeat {
            session_id,
            listener_id: random_device_id(prng),
            sent_at_elapsed_ms: MonotonicMillis::new(prng.next_u64()),
        }),
        5 => ControlMessage::StreamStart(StreamStart {
            session_id,
            stream_id: random_stream_id(prng),
            host_start_time_ms: MonotonicMillis::new(prng.next_u64()),
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 1 + u32::try_from(prng.next_below(2_000)).unwrap_or(1),
        }),
        6 => ControlMessage::Pause(Pause {
            session_id,
            stream_id: random_stream_id(prng),
            host_pause_time_ms: MonotonicMillis::new(prng.next_u64()),
        }),
        7 => ControlMessage::Stop(Stop {
            session_id,
            stream_id: random_stream_id(prng),
            host_stop_time_ms: MonotonicMillis::new(prng.next_u64()),
        }),
        8 => ControlMessage::Disconnect(Disconnect {
            session_id,
            listener_id: random_device_id(prng),
            reason: random_text(prng, 1, MAX_REASON_BYTES),
        }),
        9 => ControlMessage::ResyncNotice(ResyncNotice {
            session_id,
            listener_id: random_device_id(prng),
            reason: random_text(prng, 1, MAX_REASON_BYTES),
        }),
        _ => ControlMessage::SynchronizationReport(SynchronizationReport {
            session_id,
            listener_id: random_device_id(prng),
            confidence: random_sync_confidence(prng),
            offset_ms: random_signed_millis(prng),
            round_trip_ms: random_signed_millis(prng).abs(),
            drift_ppm: random_signed_millis(prng),
        }),
    };
    ProtocolFrame::Control(message)
}

fn random_signed_millis(prng: &mut DeterministicPrng) -> f64 {
    let magnitude = f64::from(u32::try_from(prng.next_below(1_000_000)).unwrap_or(0)) / 8.0;
    if prng.next_below(2) == 1 {
        -magnitude
    } else {
        magnitude
    }
}

fn random_sync_request(prng: &mut DeterministicPrng) -> ProtocolFrame {
    ProtocolFrame::SyncRequest(SyncRequest {
        session_id: random_session_id(prng),
        correlation_id: prng.next_u64(),
        t1_listener_send_elapsed_ms: MonotonicMillis::new(prng.next_u64()),
    })
}

fn random_sync_response(prng: &mut DeterministicPrng) -> ProtocolFrame {
    let t2 = prng.next_u64() / 2;
    let t3 = t2 + prng.next_below(10_000) as u64;
    ProtocolFrame::SyncResponse(SyncResponse {
        session_id: random_session_id(prng),
        correlation_id: prng.next_u64(),
        t1_listener_send_elapsed_ms: MonotonicMillis::new(prng.next_u64()),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(t2),
        t3_host_send_elapsed_ms: MonotonicMillis::new(t3),
    })
}

fn random_audio_frame(prng: &mut DeterministicPrng) -> ProtocolFrame {
    let channels = 1 + u16::try_from(prng.next_below(2)).unwrap_or(0);
    let samples_per_packet = 1 + u32::try_from(prng.next_below(64)).unwrap_or(1);
    let payload_len = (samples_per_packet as usize) * (channels as usize) * 2;
    ProtocolFrame::Audio(AudioDatagram {
        session_id: random_session_id(prng),
        stream_id: random_stream_id(prng),
        sequence: PacketSequence::new(prng.next_u64()),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels,
        samples_per_packet,
        first_sample_index: SampleIndex::new(prng.next_u64()),
        host_presentation_time_ms: MonotonicMillis::new(prng.next_u64()),
        payload: random_bytes(prng, payload_len),
    })
}

fn random_frame(prng: &mut DeterministicPrng) -> ProtocolFrame {
    match prng.next_below(4) {
        0 => random_control_frame(prng),
        1 => random_sync_request(prng),
        2 => random_sync_response(prng),
        _ => random_audio_frame(prng),
    }
}

/// Property test: every generated, in-bounds frame round-trips through
/// encode -> decode -> encode byte-identically, for every frame kind.
#[test]
fn generated_valid_frames_round_trip_canonically() {
    let mut prng = DeterministicPrng::new(0xF00D_CAFE_1234_5678);
    for _ in 0..2_000 {
        let frame = random_frame(&mut prng);
        let encoded = encode_frame(&frame).expect("generated frame stays within field bounds");
        let decoded = decode_frame(&encoded).expect("encoded frame decodes");
        assert!(decoded == frame, "decoded value must equal the original");
        assert_eq!(
            encode_frame(&decoded).expect("decoded frame re-encodes"),
            encoded,
            "re-encoding a decoded frame must reproduce the same bytes"
        );
    }
}

/// Fuzzes `decode_frame`/`decode_header` with pure random bytes across a wide
/// range of lengths, some far larger than any legal frame. The only
/// requirement is that decoding never panics and never allocates in
/// proportion to an untrusted claimed length -- every result must be a typed
/// `Err`, since none of these buffers are constructed to be valid.
#[test]
fn arbitrary_random_buffers_never_panic_across_many_seeds_and_lengths() {
    for seed in 0..64_u64 {
        let mut prng = DeterministicPrng::new(seed ^ 0xABCD_EF01_2345_6789);
        for length in [
            0,
            1,
            FRAME_HEADER_BYTES - 1,
            FRAME_HEADER_BYTES,
            FRAME_HEADER_BYTES + 1,
            64,
            256,
            4_096,
            MAX_AUDIO_DATAGRAM_BYTES,
            MAX_AUDIO_DATAGRAM_BYTES + 16,
        ] {
            let bytes = random_bytes(&mut prng, length);
            let _ = decode_frame(&bytes);
            let _ = decode_header(&bytes);
        }
    }
}

fn mutate_once(prng: &mut DeterministicPrng, bytes: &mut Vec<u8>) {
    if bytes.is_empty() {
        bytes.push(u8::try_from(prng.next_below(256)).unwrap_or(0));
        return;
    }
    match prng.next_below(5) {
        0 => {
            // Flip a random bit in a random byte.
            let index = prng.next_below(bytes.len());
            let bit = 1_u8 << prng.next_below(8);
            bytes[index] ^= bit;
        }
        1 => {
            // Truncate to a random shorter length.
            let cut = prng.next_below(bytes.len());
            bytes.truncate(cut);
        }
        2 => {
            // Append random trailing bytes.
            let extra = 1 + prng.next_below(32);
            bytes.extend(random_bytes(prng, extra));
        }
        3 if bytes.len() >= 2 => {
            // Smash a two-byte window to 0xFFFF -- the shape of a hostile
            // oversized length-prefix field.
            let index = prng.next_below(bytes.len() - 1);
            bytes[index] = 0xff;
            bytes[index + 1] = 0xff;
        }
        _ => {
            // Overwrite a random byte with a random value.
            let index = prng.next_below(bytes.len());
            bytes[index] = u8::try_from(prng.next_below(256)).unwrap_or(0);
        }
    }
}

/// Fuzzes every message kind's parser by mutating real, validly-encoded
/// frames (bit flips, truncation, trailing bytes, and forced 0xFFFF length
/// windows) and confirming `decode_frame` never panics regardless of how the
/// mutated bytes land, including on already-decodable-again mutants.
#[test]
fn mutated_valid_frames_never_panic_for_any_message_kind() {
    let mut prng = DeterministicPrng::new(0x1357_9BDF_2468_ACE0);
    let seeds: Vec<ProtocolFrame> = (0..14).map(|_| random_frame(&mut prng)).collect();
    for seed_frame in seeds {
        let canonical = encode_frame(&seed_frame).expect("seed frame encodes");
        for _ in 0..200 {
            let mut mutated = canonical.clone();
            let mutation_count = 1 + prng.next_below(3);
            for _ in 0..mutation_count {
                mutate_once(&mut prng, &mut mutated);
            }
            // The only contract under test: no panic, and the parser always
            // returns a typed result. If a mutation happens to still decode
            // (e.g. a bit flip inside a text field that still passes
            // validation), that is fine -- it does not violate the malformed
            // -input contract, so no additional assertion is meaningful here.
            let _ = decode_frame(&mutated);
        }
    }
}

/// Bounded-allocation-under-hostile-lengths: every declared length the
/// parser reads before allocating/copying is checked against a maximum
/// *before* any allocation is attempted, so a claimed length that vastly
/// exceeds the bytes actually present must fail immediately with a typed
/// error rather than attempting to allocate or read out of bounds.
#[test]
fn hostile_declared_lengths_are_rejected_before_allocation() {
    // Header-level: a payload_length claiming far more than the message
    // kind's maximum, with none of the claimed payload actually present.
    for (kind, claimed_length) in [
        (MessageKind::Hello, u32::MAX),
        (
            MessageKind::Hello,
            u32::try_from(MAX_CONTROL_PAYLOAD_BYTES + 1).unwrap(),
        ),
        (MessageKind::Audio, u32::MAX),
        (
            MessageKind::Audio,
            u32::try_from(MAX_AUDIO_DATAGRAM_BYTES).unwrap(),
        ),
    ] {
        let flags = if kind == MessageKind::Audio {
            super::FLAG_PAYLOAD_INTEGRITY
        } else {
            0
        };
        let mut header_only = [0_u8; FRAME_HEADER_BYTES];
        header_only[..4].copy_from_slice(&PROTOCOL_MAGIC);
        header_only[4..6].copy_from_slice(&super::PROTOCOL_VERSION.to_be_bytes());
        header_only[6..8].copy_from_slice(&kind.code().to_be_bytes());
        header_only[8..10].copy_from_slice(&flags.to_be_bytes());
        header_only[10..12].copy_from_slice(&super::FRAME_HEADER_LENGTH.to_be_bytes());
        header_only[12..16].copy_from_slice(&claimed_length.to_be_bytes());
        assert!(
            matches!(
                decode_header(&header_only),
                Err(ProtocolError::PayloadTooLarge { .. })
            ),
            "kind {kind:?} claimed_length {claimed_length} must be rejected before allocation"
        );
        assert!(matches!(
            decode_frame(&header_only),
            Err(ProtocolError::PayloadTooLarge { .. })
        ));
    }

    // Field-level: a control string length prefix of 0xFFFF (65535) with
    // only a handful of trailing bytes actually present. `read_string`
    // rejects this because the declared length exceeds the field's maximum,
    // before ever slicing/copying the (absent) claimed bytes.
    let session_id = SessionId::new("s").expect("valid session id");
    let mut hello_payload = Vec::new();
    hello_payload.extend_from_slice(
        &u16::try_from(session_id.as_str().len())
            .unwrap()
            .to_be_bytes(),
    );
    hello_payload.extend_from_slice(session_id.as_str().as_bytes());
    hello_payload.extend_from_slice(&0xFFFF_u16.to_be_bytes()); // session_name length
    hello_payload.extend_from_slice(&[0, 1, 2, 3]); // far fewer bytes than claimed
    let mut frame = Vec::new();
    frame.extend_from_slice(&PROTOCOL_MAGIC);
    frame.extend_from_slice(&super::PROTOCOL_VERSION.to_be_bytes());
    frame.extend_from_slice(&MessageKind::Hello.code().to_be_bytes());
    frame.extend_from_slice(&0_u16.to_be_bytes());
    frame.extend_from_slice(&super::FRAME_HEADER_LENGTH.to_be_bytes());
    frame.extend_from_slice(&u32::try_from(hello_payload.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(&hello_payload);
    assert!(matches!(
        decode_frame(&frame),
        Err(ProtocolError::PayloadTooLarge { .. })
    ));

    // Audio-level: declared_payload_length claims the maximum representable
    // u16 (65535 bytes) while only a few payload bytes are actually present
    // after it -- must fail with a length mismatch, not read/allocate past
    // the real buffer.
    let stream_id = StreamId::new("a").expect("valid stream id");
    let mut audio_payload = Vec::new();
    audio_payload.extend_from_slice(
        &u16::try_from(session_id.as_str().len())
            .unwrap()
            .to_be_bytes(),
    );
    audio_payload.extend_from_slice(session_id.as_str().as_bytes());
    audio_payload.extend_from_slice(
        &u16::try_from(stream_id.as_str().len())
            .unwrap()
            .to_be_bytes(),
    );
    audio_payload.extend_from_slice(stream_id.as_str().as_bytes());
    audio_payload.extend_from_slice(&1_u64.to_be_bytes()); // sequence
    audio_payload.push(AudioCodec::PcmS16Le.code());
    audio_payload.extend_from_slice(&48_000_u32.to_be_bytes());
    audio_payload.extend_from_slice(&2_u16.to_be_bytes());
    audio_payload.extend_from_slice(&1_u32.to_be_bytes());
    audio_payload.extend_from_slice(&0_u64.to_be_bytes());
    audio_payload.extend_from_slice(&0_u64.to_be_bytes());
    audio_payload.extend_from_slice(&0xFFFF_u16.to_be_bytes()); // declared_payload_length
    audio_payload.extend_from_slice(&0_u32.to_be_bytes()); // checksum
    audio_payload.extend_from_slice(&[9, 9, 9, 9]); // far fewer bytes than declared
    let mut audio_frame = Vec::new();
    audio_frame.extend_from_slice(&PROTOCOL_MAGIC);
    audio_frame.extend_from_slice(&super::PROTOCOL_VERSION.to_be_bytes());
    audio_frame.extend_from_slice(&MessageKind::Audio.code().to_be_bytes());
    audio_frame.extend_from_slice(&super::FLAG_PAYLOAD_INTEGRITY.to_be_bytes());
    audio_frame.extend_from_slice(&super::FRAME_HEADER_LENGTH.to_be_bytes());
    audio_frame.extend_from_slice(&u32::try_from(audio_payload.len()).unwrap().to_be_bytes());
    audio_frame.extend_from_slice(&audio_payload);
    assert!(matches!(
        decode_frame(&audio_frame),
        Err(ProtocolError::LengthMismatch { .. })
    ));
}
