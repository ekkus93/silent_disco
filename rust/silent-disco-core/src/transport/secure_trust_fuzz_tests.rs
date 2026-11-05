//! Seeded fuzz and property tests for the durable trust snapshot parser.
//!
//! The snapshot is read back from disk after a restart, a restore or a
//! tampering attempt, so it is untrusted input. A fixed seed keeps runs
//! reproducible (no nightly fuzzer in the pinned toolchain). Properties: no
//! panic on any input, every accepted snapshot satisfies the registry
//! invariants, valid snapshots survive a save/load cycle, and every hostile
//! shape is refused instead of being quietly accepted.

use std::collections::BTreeSet;

use p256::{ecdsa::SigningKey, pkcs8::EncodePublicKey};
use sha2::{Digest, Sha256};

use super::DeterministicPrng;
use super::secure_trust::{HostTrustError, HostVerifiedTrust};

const MAX_LISTENERS: usize = 256;
const MAX_SNAPSHOT_BYTES: usize = 131_072;

fn host_key_der(seed: u8) -> Vec<u8> {
    SigningKey::from_slice(&[seed; 32])
        .expect("fixture key")
        .verifying_key()
        .to_public_key_der()
        .expect("public key DER")
        .as_bytes()
        .to_vec()
}

fn fingerprint_of(der: &[u8]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out.copy_from_slice(&Sha256::digest(der));
    out
}

fn array_json(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(u8::to_string).collect();
    format!("[{}]", items.join(","))
}

fn listener_json(fingerprint: &[u8], name: &str, revoked: bool) -> String {
    format!(
        r#"{{"fingerprint":{},"display_name":"{name}","revoked":{revoked}}}"#,
        array_json(fingerprint)
    )
}

fn snapshot_json(version: u32, host: &[u8; 32], listeners: &[String]) -> String {
    format!(
        r#"{{"version":{version},"host_fingerprint":{},"listeners":[{}]}}"#,
        array_json(host),
        listeners.join(",")
    )
}

fn random_fingerprint(prng: &mut DeterministicPrng) -> [u8; 32] {
    loop {
        let mut out = [0_u8; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&prng.next_u64().to_le_bytes()[..chunk.len()]);
        }
        if out != [0; 32] {
            return out;
        }
    }
}

fn random_name(prng: &mut DeterministicPrng) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 -_.";
    let length = 1 + prng.next_below(40);
    let name: String = (0..length)
        .map(|_| CHARS[prng.next_below(CHARS.len())] as char)
        .collect();
    // A name of only spaces is invalid by design; keep generated ones valid.
    format!("L{name}")
}

fn assert_registry_invariants(trust: &HostVerifiedTrust) {
    let listeners = trust.stored_listeners();
    assert!(listeners.len() <= MAX_LISTENERS);
    let mut seen = BTreeSet::new();
    for listener in &listeners {
        assert!(!listener.display_name.trim().is_empty());
        assert!(listener.display_name.len() <= 128);
        assert!(!listener.display_name.chars().any(char::is_control));
        assert!(
            seen.insert(format!("{:?}", listener.id)),
            "duplicate listener in an accepted snapshot"
        );
    }
}

#[test]
fn generated_valid_snapshots_load_and_survive_a_save_load_cycle() {
    let host = host_key_der(3);
    let host_fp = fingerprint_of(&host);
    let mut prng = DeterministicPrng::new(0x5452_5553_5400_0001);
    for _ in 0..300 {
        let count = prng.next_below(40);
        let mut used = BTreeSet::new();
        let mut records = Vec::new();
        while records.len() < count {
            let fingerprint = random_fingerprint(&mut prng);
            if used.insert(fingerprint) {
                records.push(listener_json(
                    &fingerprint,
                    &random_name(&mut prng),
                    prng.next_below(2) == 0,
                ));
            }
        }
        let json = snapshot_json(1, &host_fp, &records);
        let loaded = HostVerifiedTrust::from_json(&host, &json)
            .unwrap_or_else(|error| panic!("valid snapshot rejected: {error:?}"));
        assert_eq!(loaded.stored_listeners().len(), count);
        assert_registry_invariants(&loaded);
        let saved = loaded.to_json().expect("save");
        let reloaded = HostVerifiedTrust::from_json(&host, &saved).expect("reload");
        assert_eq!(reloaded.stored_listeners(), loaded.stored_listeners());
    }
}

#[test]
fn mutated_snapshots_never_panic_and_accepted_ones_keep_the_registry_invariants() {
    let host = host_key_der(3);
    let host_fp = fingerprint_of(&host);
    let mut prng = DeterministicPrng::new(0x5452_5553_5400_0002);
    let mut accepted = 0_u32;
    for _ in 0..20_000 {
        let records: Vec<String> = (0..=prng.next_below(3))
            .map(|_| {
                listener_json(
                    &random_fingerprint(&mut prng),
                    &random_name(&mut prng),
                    false,
                )
            })
            .collect();
        let mut bytes = snapshot_json(1, &host_fp, &records).into_bytes();
        for _ in 0..=prng.next_below(3) {
            if bytes.is_empty() {
                break;
            }
            let index = prng.next_below(bytes.len());
            match prng.next_below(4) {
                0 => bytes[index] ^= 1_u8 << prng.next_below(8),
                1 => bytes.truncate(index),
                2 => bytes[index] = b"{}[]\":,0 \\-9"[prng.next_below(12)],
                _ => {
                    bytes.remove(index);
                }
            }
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if let Ok(trust) = HostVerifiedTrust::from_json(&host, &text) {
            accepted += 1;
            assert_registry_invariants(&trust);
        }
    }
    assert!(accepted > 0, "no mutated snapshot was ever accepted");
}

// A table of hostile shapes reads better as one list than split across helpers.
#[allow(clippy::too_many_lines)]
#[test]
fn hostile_snapshots_are_refused_with_the_right_reason() {
    let host = host_key_der(3);
    let host_fp = fingerprint_of(&host);
    let other_fp = fingerprint_of(&host_key_der(4));
    let one = random_fingerprint(&mut DeterministicPrng::new(1));
    let two = random_fingerprint(&mut DeterministicPrng::new(2));
    let ok = |name: &str| listener_json(&one, name, false);

    let too_many: Vec<String> = (0..=MAX_LISTENERS)
        .map(|index| {
            let mut fingerprint = [0_u8; 32];
            fingerprint[..8].copy_from_slice(&(index as u64 + 1).to_le_bytes());
            listener_json(&fingerprint, "L", false)
        })
        .collect();
    let at_limit: Vec<String> = too_many[..MAX_LISTENERS].to_vec();
    HostVerifiedTrust::from_json(&host, &snapshot_json(1, &host_fp, &at_limit))
        .expect("exactly the maximum is accepted");

    let cases: Vec<(&str, String, HostTrustError)> = vec![
        (
            "too many listeners",
            snapshot_json(1, &host_fp, &too_many),
            HostTrustError::TooManyListeners,
        ),
        (
            "other host's snapshot",
            snapshot_json(1, &other_fp, &[ok("A")]),
            HostTrustError::WrongHostIdentity,
        ),
        (
            "version 0",
            snapshot_json(0, &host_fp, &[ok("A")]),
            HostTrustError::UnsupportedVersion,
        ),
        (
            "future version",
            snapshot_json(2, &host_fp, &[ok("A")]),
            HostTrustError::UnsupportedVersion,
        ),
        (
            "duplicate fingerprint",
            snapshot_json(1, &host_fp, &[ok("A"), ok("B")]),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "zero fingerprint",
            snapshot_json(1, &host_fp, &[listener_json(&[0; 32], "A", false)]),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "blank name",
            snapshot_json(1, &host_fp, &[ok("   ")]),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "control character in name",
            snapshot_json(1, &host_fp, &[ok("a\\u0007b")]),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "oversize name",
            snapshot_json(1, &host_fp, &[ok(&"n".repeat(129))]),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "short fingerprint",
            snapshot_json(1, &host_fp, &[listener_json(&two[..31], "A", false)]),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "long fingerprint",
            snapshot_json(
                1,
                &host_fp,
                &[listener_json(&[two.as_slice(), &[1]].concat(), "A", false)],
            ),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "byte above 255",
            snapshot_json(1, &host_fp, &[ok("A")]).replacen('[', "[256,", 1),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "unknown field",
            snapshot_json(1, &host_fp, &[ok("A")]).replacen('}', r#","extra":1}"#, 1),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "revoked as string",
            snapshot_json(1, &host_fp, &[ok("A")])
                .replace("\"revoked\":false", "\"revoked\":\"no\""),
            HostTrustError::InvalidSnapshot,
        ),
        (
            "not an object",
            "[]".to_owned(),
            HostTrustError::InvalidSnapshot,
        ),
        ("empty", String::new(), HostTrustError::InvalidSnapshot),
        (
            "oversize document",
            format!(
                "{}{}",
                snapshot_json(1, &host_fp, &[]),
                " ".repeat(MAX_SNAPSHOT_BYTES)
            ),
            HostTrustError::InvalidSnapshot,
        ),
    ];
    for (name, json, expected) in cases {
        let error = HostVerifiedTrust::from_json(&host, &json)
            .err()
            .unwrap_or_else(|| panic!("hostile snapshot accepted: {name}"));
        assert_eq!(error, expected, "wrong refusal for: {name}");
    }
}

#[test]
fn an_invalid_host_key_is_refused_before_any_snapshot_is_read() {
    let json = snapshot_json(1, &[1; 32], &[]);
    assert_eq!(
        HostVerifiedTrust::from_json(&[0, 1, 2, 3], &json).err(),
        Some(HostTrustError::InvalidHostKey)
    );
    assert_eq!(
        HostVerifiedTrust::from_json(&[], "").err(),
        Some(HostTrustError::InvalidHostKey)
    );
}
