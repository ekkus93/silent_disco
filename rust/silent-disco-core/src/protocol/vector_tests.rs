use std::collections::{BTreeMap, BTreeSet};

use super::{
    DecodePolicy, MAX_AUDIO_DATAGRAM_BYTES, PacketSequence, ProtocolDecoder, ProtocolError,
    ProtocolFrame, crc32, decode_frame, encode_frame,
};
use crate::domain::SessionId;

const CONTROL_VECTORS: &str =
    include_str!("../../testdata/protocol/v2/control_vectors.txt");
const SYNC_VECTORS: &str = include_str!("../../testdata/protocol/v2/sync_vectors.txt");
const AUDIO_VECTORS: &str = include_str!("../../testdata/protocol/v2/audio_vectors.txt");
const BOUNDARY_VECTORS: &str =
    include_str!("../../testdata/protocol/v2/boundary_vectors.txt");
const MALFORMED_VECTORS: &str =
    include_str!("../../testdata/protocol/v2/malformed_vectors.txt");

struct ValidVector {
    name: String,
    kind: String,
    frame_crc32: u32,
    payload_crc32: u32,
    bytes: Vec<u8>,
}

struct MalformedVector {
    name: String,
    expected_error: String,
    bytes: Vec<u8>,
}

#[test]
fn every_protocol_message_kind_has_a_canonical_golden_vector() {
    let vectors = all_primary_vectors();
    let names: BTreeSet<&str> = vectors.iter().map(|vector| vector.name.as_str()).collect();
    let expected: BTreeSet<&str> = [
        "hello",
        "join_request",
        "join_approval",
        "join_rejection",
        "heartbeat",
        "stream_start",
        "pause",
        "stop",
        "disconnect",
        "resync_notice",
        "sync_request",
        "sync_response",
        "audio",
    ]
    .into_iter()
    .collect();
    assert_eq!(names, expected);

    for vector in vectors {
        verify_valid_vector(&vector);
    }
}

#[test]
fn boundary_vectors_are_canonical_and_reach_declared_limits() {
    let vectors = parse_valid_vectors(BOUNDARY_VECTORS);
    assert_eq!(vectors.len(), 4);
    for vector in &vectors {
        verify_valid_vector(vector);
    }

    let maximum_audio = vectors
        .iter()
        .find(|vector| vector.name == "max_audio_datagram")
        .unwrap_or_else(|| panic!("missing max_audio_datagram vector"));
    assert_eq!(maximum_audio.bytes.len(), MAX_AUDIO_DATAGRAM_BYTES);
}

#[test]
fn malformed_vectors_fail_with_the_declared_stable_category() {
    for vector in parse_malformed_vectors(MALFORMED_VECTORS) {
        let error = match decode_frame(&vector.bytes) {
            Ok(_) => panic!("malformed vector {} decoded successfully", vector.name),
            Err(error) => error,
        };
        assert_eq!(error_tag(&error), vector.expected_error);
    }
}

#[test]
fn diagnostic_counters_distinguish_every_required_failure_class() {
    let malformed: BTreeMap<String, MalformedVector> = parse_malformed_vectors(MALFORMED_VECTORS)
        .into_iter()
        .map(|vector| (vector.name.clone(), vector))
        .collect();
    let control: BTreeMap<String, ValidVector> = parse_valid_vectors(CONTROL_VECTORS)
        .into_iter()
        .map(|vector| (vector.name.clone(), vector))
        .collect();
    let audio: BTreeMap<String, ValidVector> = parse_valid_vectors(AUDIO_VECTORS)
        .into_iter()
        .map(|vector| (vector.name.clone(), vector))
        .collect();

    let mut decoder = ProtocolDecoder::default();
    decode_expect_error(
        &mut decoder,
        &required_malformed(&malformed, "bad_magic").bytes,
        DecodePolicy::default(),
    );
    decode_expect_error(
        &mut decoder,
        &required_malformed(&malformed, "unsupported_version").bytes,
        DecodePolicy::default(),
    );
    decode_expect_error(
        &mut decoder,
        &required_malformed(&malformed, "oversized_control").bytes,
        DecodePolicy::default(),
    );

    let expected_session = SessionId::new("different-session")
        .unwrap_or_else(|error| panic!("test session ID must be valid: {error}"));
    decode_expect_error(
        &mut decoder,
        &required_valid(&control, "hello").bytes,
        DecodePolicy {
            expected_session_id: Some(&expected_session),
            minimum_audio_sequence: None,
        },
    );

    let audio_bytes = &required_valid(&audio, "audio").bytes;
    let decoded_audio = decode_valid(audio_bytes);
    let minimum = match decoded_audio {
        ProtocolFrame::Audio(value) => PacketSequence::new(value.sequence.get() + 1),
        _ => panic!("audio fixture decoded to the wrong message kind"),
    };
    decode_expect_error(
        &mut decoder,
        audio_bytes,
        DecodePolicy {
            expected_session_id: None,
            minimum_audio_sequence: Some(minimum),
        },
    );

    let counters = decoder.counters();
    assert_eq!(counters.malformed, 1);
    assert_eq!(counters.unsupported, 1);
    assert_eq!(counters.unauthorized, 1);
    assert_eq!(counters.stale, 1);
    assert_eq!(counters.oversized, 1);
}

#[test]
fn oversized_fixture_is_rejected_from_header_without_payload_bytes() {
    let malformed: BTreeMap<String, MalformedVector> = parse_malformed_vectors(MALFORMED_VECTORS)
        .into_iter()
        .map(|vector| (vector.name.clone(), vector))
        .collect();
    let vector = required_malformed(&malformed, "oversized_control");
    assert_eq!(vector.bytes.len(), super::FRAME_HEADER_BYTES);
    assert!(matches!(
        decode_frame(&vector.bytes),
        Err(ProtocolError::PayloadTooLarge { .. })
    ));
}

fn all_primary_vectors() -> Vec<ValidVector> {
    [CONTROL_VECTORS, SYNC_VECTORS, AUDIO_VECTORS]
        .into_iter()
        .flat_map(parse_valid_vectors)
        .collect()
}

fn verify_valid_vector(vector: &ValidVector) {
    assert_eq!(crc32(&vector.bytes), vector.frame_crc32, "{}", vector.name);
    assert_eq!(
        crc32(&vector.bytes[super::FRAME_HEADER_BYTES..]),
        vector.payload_crc32,
        "{}",
        vector.name
    );
    let decoded = decode_valid(&vector.bytes);
    assert_eq!(decoded.kind().stable_name(), vector.kind);
    let reencoded = encode_frame(&decoded)
        .unwrap_or_else(|error| panic!("failed to re-encode {}: {error}", vector.name));
    assert_eq!(reencoded, vector.bytes, "{}", vector.name);
}

fn decode_valid(bytes: &[u8]) -> ProtocolFrame {
    match decode_frame(bytes) {
        Ok(frame) => frame,
        Err(error) => panic!("golden vector failed to decode: {error}"),
    }
}

fn decode_expect_error(
    decoder: &mut ProtocolDecoder,
    bytes: &[u8],
    policy: DecodePolicy<'_>,
) {
    if decoder.decode(bytes, policy).is_ok() {
        panic!("expected protocol fixture to fail");
    }
}

fn required_valid<'a>(
    vectors: &'a BTreeMap<String, ValidVector>,
    name: &str,
) -> &'a ValidVector {
    vectors
        .get(name)
        .unwrap_or_else(|| panic!("missing valid vector {name}"))
}

fn required_malformed<'a>(
    vectors: &'a BTreeMap<String, MalformedVector>,
    name: &str,
) -> &'a MalformedVector {
    vectors
        .get(name)
        .unwrap_or_else(|| panic!("missing malformed vector {name}"))
}

fn parse_valid_vectors(source: &str) -> Vec<ValidVector> {
    data_lines(source)
        .map(|line| {
            let mut fields = line.split('|');
            let name = required_field(&mut fields, "name").to_owned();
            let kind = required_field(&mut fields, "kind").to_owned();
            let frame_crc32 = parse_hex_u32(required_field(&mut fields, "frame_crc32"));
            let payload_crc32 = parse_hex_u32(required_field(&mut fields, "payload_crc32"));
            let bytes = parse_hex_bytes(required_field(&mut fields, "hex"));
            assert!(fields.next().is_none(), "unexpected valid-vector field");
            ValidVector {
                name,
                kind,
                frame_crc32,
                payload_crc32,
                bytes,
            }
        })
        .collect()
}

fn parse_malformed_vectors(source: &str) -> Vec<MalformedVector> {
    data_lines(source)
        .map(|line| {
            let mut fields = line.split('|');
            let name = required_field(&mut fields, "name").to_owned();
            let expected_error = required_field(&mut fields, "expected_error").to_owned();
            let bytes = parse_hex_bytes(required_field(&mut fields, "hex"));
            assert!(fields.next().is_none(), "unexpected malformed-vector field");
            MalformedVector {
                name,
                expected_error,
                bytes,
            }
        })
        .collect()
}

fn data_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

fn required_field<'a>(fields: &mut impl Iterator<Item = &'a str>, name: &str) -> &'a str {
    fields
        .next()
        .unwrap_or_else(|| panic!("missing fixture field {name}"))
}

fn parse_hex_u32(value: &str) -> u32 {
    u32::from_str_radix(value, 16)
        .unwrap_or_else(|error| panic!("invalid fixture u32 hex {value}: {error}"))
}

fn parse_hex_bytes(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2), "fixture hex length must be even");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]))
        .collect()
}

fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid fixture hex digit"),
    }
}

fn error_tag(error: &ProtocolError) -> &'static str {
    match error {
        ProtocolError::Truncated => "truncated",
        ProtocolError::TrailingBytes => "trailing_bytes",
        ProtocolError::InvalidMagic => "invalid_magic",
        ProtocolError::UnsupportedVersion { .. } => "unsupported_version",
        ProtocolError::UnsupportedMessageKind { .. } => "unsupported_kind",
        ProtocolError::UnsupportedFlags { .. } => "unsupported_flags",
        ProtocolError::PayloadTooLarge { .. } => "oversized",
        ProtocolError::LengthMismatch { .. } => "length_mismatch",
        ProtocolError::InvalidBoolean { .. } => "invalid_boolean",
        ProtocolError::UnsupportedAudioCodec { .. } => "unsupported_codec",
        ProtocolError::IntegrityMismatch => "integrity_mismatch",
        other => panic!("unexpected malformed-vector error: {other}"),
    }
}
