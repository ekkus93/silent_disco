//! Seeded fuzz and property tests for the manual connection payload parser.
//!
//! The payload is untrusted text pasted or scanned from outside the process.
//! As with `protocol::fuzz_tests`, a fixed-seed generator keeps every run
//! reproducible without a nightly toolchain. The properties checked are: the
//! parser never panics, every accepted value satisfies the documented
//! invariants, every valid payload round-trips, and hostile shapes (oversize,
//! deep nesting, out-of-range numbers, unknown or duplicate fields) are
//! refused rather than accepted.

use std::net::IpAddr;

use super::manual_endpoint::{MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES, ManualHostEndpoint};
use super::{DeterministicPrng, TransportErrorKind};
use crate::runtime::current_protocol_version;

const NOW_MS: u64 = 1_700_000_000_000;

fn random_address(prng: &mut DeterministicPrng) -> String {
    if prng.next_below(4) == 0 {
        // IPv6 unicast (unique local).
        format!("fd00::{:x}", 1 + prng.next_below(0xfffe))
    } else {
        format!(
            "{}.{}.{}.{}",
            [10, 100, 172, 192][prng.next_below(4)],
            prng.next_below(256),
            prng.next_below(256),
            1 + prng.next_below(254)
        )
    }
}

fn port(prng: &mut DeterministicPrng) -> u16 {
    u16::try_from(1 + prng.next_below(65_535)).expect("1..=65535 fits u16")
}

struct Generated {
    json: String,
    address: String,
    ports: [u16; 3],
    session: String,
    invite_required: bool,
    expires: Option<u64>,
}

fn generated_payload(prng: &mut DeterministicPrng) -> Generated {
    let address = random_address(prng);
    let ports = [port(prng), port(prng), port(prng)];
    let session = format!("session-{}-{}", prng.next_u64(), prng.next_below(1_000));
    let invite_required = prng.next_below(2) == 0;
    let expires = (prng.next_below(2) == 0).then(|| NOW_MS + 1 + prng.next_u64() % 1_000_000_000);
    let expires_json = expires.map_or_else(|| "null".to_owned(), |value| format!("\"{value}\""));
    let json = format!(
        r#"{{"hostAddress":"{address}","controlPort":{},"syncPort":{},"audioPort":{},"sessionId":"{session}","protocolVersion":{},"inviteCodeRequired":{invite_required},"expiresAtMs":{expires_json}}}"#,
        ports[0],
        ports[1],
        ports[2],
        current_protocol_version()
    );
    Generated {
        json,
        address,
        ports,
        session,
        invite_required,
        expires,
    }
}

/// Everything the parser promises about an accepted value.
fn assert_accepted_invariants(parsed: &ManualHostEndpoint, now_ms: u64) {
    let address: IpAddr = parsed.endpoint.address;
    assert!(!address.is_unspecified() && !address.is_multicast());
    if let IpAddr::V4(v4) = address {
        assert!(!v4.is_broadcast());
    }
    assert_ne!(parsed.endpoint.control_port, 0);
    assert_ne!(parsed.endpoint.sync_port, 0);
    assert_ne!(parsed.endpoint.audio_port, 0);
    assert_eq!(parsed.protocol_version, current_protocol_version());
    if let Some(expires) = parsed.expires_at_ms {
        assert!(now_ms < expires, "an expired payload was accepted");
    }
}

#[test]
fn generated_valid_payloads_parse_and_round_trip_their_fields() {
    let mut prng = DeterministicPrng::new(0x4D41_4E55_414C_0001);
    for _ in 0..5_000 {
        let generated = generated_payload(&mut prng);
        let parsed = ManualHostEndpoint::parse(&generated.json, NOW_MS).unwrap_or_else(|error| {
            panic!("valid payload rejected: {error:?}\n{}", generated.json)
        });
        assert_accepted_invariants(&parsed, NOW_MS);
        assert_eq!(
            parsed.endpoint.address.to_string(),
            generated.address.parse::<IpAddr>().unwrap().to_string()
        );
        assert_eq!(
            [
                parsed.endpoint.control_port,
                parsed.endpoint.sync_port,
                parsed.endpoint.audio_port
            ],
            generated.ports
        );
        assert_eq!(parsed.session_id.as_str(), generated.session);
        assert_eq!(parsed.invite_code_required, generated.invite_required);
        assert_eq!(parsed.expires_at_ms, generated.expires);
    }
}

#[test]
fn mutated_payloads_never_panic_and_any_accepted_value_keeps_its_invariants() {
    let mut prng = DeterministicPrng::new(0x4D41_4E55_414C_0002);
    let mut accepted = 0_u32;
    for _ in 0..20_000 {
        let mut bytes = generated_payload(&mut prng).json.into_bytes();
        for _ in 0..=prng.next_below(3) {
            if bytes.is_empty() {
                break;
            }
            let index = prng.next_below(bytes.len());
            match prng.next_below(4) {
                0 => bytes[index] ^= 1_u8 << prng.next_below(8),
                1 => bytes.truncate(index),
                2 => bytes[index] = b"{}[]\":,0 \\-"[prng.next_below(11)],
                _ => {
                    bytes.remove(index);
                }
            }
        }
        // The parser takes &str; non-UTF-8 is rejected before it is reached.
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if let Ok(parsed) = ManualHostEndpoint::parse(&text, NOW_MS) {
            accepted += 1;
            assert_accepted_invariants(&parsed, NOW_MS);
        }
    }
    // Some mutations (for example flipping a digit inside the session ID) stay
    // valid, so a run that accepts none would not be exercising acceptance.
    assert!(accepted > 0, "no mutated payload was ever accepted");
}

#[test]
fn arbitrary_text_never_panics_and_is_never_accepted_by_accident() {
    let mut prng = DeterministicPrng::new(0x4D41_4E55_414C_0003);
    for _ in 0..20_000 {
        let length = prng.next_below(512);
        let text: String = (0..length)
            .map(|_| {
                char::from_u32(u32::try_from(prng.next_below(0x2FF)).unwrap_or(0)).unwrap_or('?')
            })
            .collect();
        assert!(ManualHostEndpoint::parse(&text, NOW_MS).is_err());
    }
}

#[test]
fn oversize_payload_is_refused_before_parsing_and_the_limit_is_exact() {
    let mut prng = DeterministicPrng::new(0x4D41_4E55_414C_0004);
    let valid = generated_payload(&mut prng).json;
    // Padding with trailing whitespace keeps the JSON valid at exactly the limit.
    let at_limit = format!(
        "{valid}{}",
        " ".repeat(MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES - valid.len())
    );
    assert_eq!(at_limit.len(), MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES);
    ManualHostEndpoint::parse(&at_limit, NOW_MS).expect("a payload at the limit is accepted");
    let over = format!("{at_limit} ");
    let error = ManualHostEndpoint::parse(&over, NOW_MS).expect_err("one byte over the limit");
    assert_eq!(error.kind, TransportErrorKind::InvalidConfiguration);
    let huge = "a".repeat(10 * 1024 * 1024);
    assert!(ManualHostEndpoint::parse(&huge, NOW_MS).is_err());
}

#[test]
fn hostile_shapes_are_refused_not_accepted() {
    let version = current_protocol_version();
    let base = |fields: &str| format!("{{{fields}}}");
    let ok_fields = format!(
        r#""hostAddress":"192.168.1.50","controlPort":41000,"syncPort":41001,"audioPort":41002,"sessionId":"s","protocolVersion":{version},"inviteCodeRequired":false,"expiresAtMs":null"#
    );
    ManualHostEndpoint::parse(&base(&ok_fields), NOW_MS).expect("baseline is valid");

    let cases: Vec<(&str, String)> = vec![
        ("unknown field", base(&format!(r#"{ok_fields},"extra":1"#))),
        (
            "duplicate field",
            base(&format!(r#"{ok_fields},"controlPort":41005"#)),
        ),
        ("port above u16", base(&ok_fields.replace("41000", "65536"))),
        ("negative port", base(&ok_fields.replace("41000", "-1"))),
        ("zero port", base(&ok_fields.replace("41000", "0"))),
        (
            "fractional port",
            base(&ok_fields.replace("41000", "4.1e4")),
        ),
        (
            "unspecified address",
            base(&ok_fields.replace("192.168.1.50", "0.0.0.0")),
        ),
        (
            "multicast address",
            base(&ok_fields.replace("192.168.1.50", "224.0.0.1")),
        ),
        (
            "broadcast address",
            base(&ok_fields.replace("192.168.1.50", "255.255.255.255")),
        ),
        (
            "hostname instead of address",
            base(&ok_fields.replace("192.168.1.50", "example.com")),
        ),
        (
            "expiry overflows u64",
            base(&ok_fields.replace(
                r#""expiresAtMs":null"#,
                r#""expiresAtMs":"99999999999999999999999""#,
            )),
        ),
        (
            "expiry in the past",
            base(&ok_fields.replace(
                r#""expiresAtMs":null"#,
                &format!(r#""expiresAtMs":"{}""#, NOW_MS - 1),
            )),
        ),
        (
            "expiry as a number",
            base(&ok_fields.replace(r#""expiresAtMs":null"#, r#""expiresAtMs":123"#)),
        ),
        (
            "wrong protocol version",
            base(&ok_fields.replace(
                &format!(r#""protocolVersion":{version}"#),
                r#""protocolVersion":65535"#,
            )),
        ),
        (
            "empty session",
            base(&ok_fields.replace(r#""sessionId":"s""#, r#""sessionId":"""#)),
        ),
        (
            "session of control characters",
            base(&ok_fields.replace(r#""sessionId":"s""#, r#""sessionId":"\u0000\u0001""#)),
        ),
        (
            "missing field",
            base(&ok_fields.replace(r#""audioPort":41002,"#, "")),
        ),
        ("not an object", format!("[{ok_fields}]")),
        ("trailing garbage", format!("{}x", base(&ok_fields))),
        ("deep nesting", "[".repeat(2_000)),
        (
            "deep object nesting",
            format!("{}1{}", r#"{"a":"#.repeat(300), "}".repeat(300)),
        ),
    ];
    for (name, payload) in cases {
        assert!(
            payload.len() <= MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES,
            "{name}: case must be small enough to reach the parser"
        );
        assert!(
            ManualHostEndpoint::parse(&payload, NOW_MS).is_err(),
            "hostile payload accepted: {name}"
        );
    }
}
