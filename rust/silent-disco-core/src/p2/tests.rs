#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use p256::{
        ecdsa::{Signature, SigningKey, signature::Signer},
        pkcs8::EncodePublicKey,
    };

    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "silent-disco-p2-{name}-{}-{}.sqlite3",
            std::process::id(),
            current_unix_millis()
        ))
    }

    fn remove_database(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// Matches `ManualHostEndpoint::parse`'s own `valid_payload` fixture
    /// exactly (`transport::manual_endpoint::tests`), since a desktop host's
    /// real embedded connection payload is produced the same way.
    fn connection_payload_json() -> String {
        format!(
            r#"{{"hostAddress":"192.168.1.50","controlPort":41000,"syncPort":41001,"audioPort":41002,"sessionId":"550e8400-e29b-41d4-a716-446655440000","protocolVersion":{},"inviteCodeRequired":true,"expiresAtMs":null}}"#,
            crate::runtime::current_protocol_version()
        )
    }

    fn input(now: u64, connection_payload_json: Option<String>) -> (QrInvitationInput, SigningKey) {
        let signing_key = SigningKey::from_bytes((&[7_u8; 32]).into()).expect("fixed key");
        let public_key = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("public DER")
            .as_bytes()
            .to_vec();
        (
            QrInvitationInput {
                session_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
                session_name: "Rooftop Disco".to_owned(),
                host_name: "Phillip's host".to_owned(),
                host_public_key_der: public_key,
                approval_mode: "invite_code".to_owned(),
                invite_code: Some("4826".to_owned()),
                issued_at_ms: now,
                expires_at_ms: now + 300_000,
                nonce: "nonce-1234567890abcdef".to_owned(),
                connection_payload_json,
            },
            signing_key,
        )
    }

    fn sign(unsigned_input: &QrInvitationInput, signing_key: &SigningKey) -> String {
        let unsigned = prepare_unsigned_qr(unsigned_input).expect("unsigned payload");
        let signature: Signature = signing_key.sign(unsigned.as_bytes());
        finalize_qr(
            &unsigned,
            &URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes()),
        )
        .expect("signed payload")
    }

    fn signed_payload(now: u64) -> (String, Vec<u8>) {
        let (qr_input, signing_key) = input(now, None);
        let public_key = qr_input.host_public_key_der.clone();
        (sign(&qr_input, &signing_key), public_key)
    }

    #[test]
    fn recent_listener_query_is_bounded_and_expires_old_rows() {
        let path = test_path("recent");
        let mut store = P2Store::open(&path).expect("open");
        let now = DEFAULT_RECENT_MAX_AGE_MS + 10_000;
        for (index, age) in [1_000_u64, DEFAULT_RECENT_MAX_AGE_MS + 1]
            .into_iter()
            .enumerate()
        {
            store
                .record_session(&RecentSessionRecord {
                    session_id: format!("550e8400-e29b-41d4-a716-44665544000{index}"),
                    role: RecentSessionRole::Listener,
                    session_name: format!("Session {index}"),
                    host_name: "Host".to_owned(),
                    host_fingerprint: None,
                    started_at_ms: now - age - 100,
                    ended_at_ms: now - age,
                    outcome: RecentSessionOutcome::Completed,
                })
                .expect("record");
        }
        let recent = store
            .list_recent_listener_sessions(now, DEFAULT_RECENT_MAX_AGE_MS, 5)
            .expect("recent");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].session_name, "Session 0");
        drop(store);
        remove_database(&path);
    }

    #[test]
    fn trusted_hosts_are_keyed_by_public_key_not_display_name() {
        let path = test_path("trust");
        let mut store = P2Store::open(&path).expect("open");
        for (byte, name) in [(3_u8, "Same name"), (4_u8, "Same name")] {
            let signing_key = SigningKey::from_bytes((&[byte; 32]).into()).expect("key");
            let public_key = signing_key
                .verifying_key()
                .to_public_key_der()
                .expect("DER")
                .as_bytes()
                .to_vec();
            store
                .trust_host(&TrustedHostRecord {
                    fingerprint: public_key_fingerprint(&public_key),
                    display_name: name.to_owned(),
                    public_key_der: public_key,
                    first_verified_ms: 1,
                    last_verified_ms: 2,
                })
                .expect("trust");
        }
        assert_eq!(store.list_trusted_hosts().expect("list").len(), 2);
        drop(store);
        remove_database(&path);
    }

    #[test]
    fn qr_signature_expiry_tampering_and_replay_are_enforced() {
        let path = test_path("qr");
        let now = 2_000_000_000_u64;
        let (payload, public_key) = signed_payload(now);
        let mut store = P2Store::open(&path).expect("open");
        let validated = store
            .validate_and_consume_qr(&payload, now + 1_000)
            .expect("validate");
        assert_eq!(
            validated.host_fingerprint,
            public_key_fingerprint(&public_key)
        );
        assert_eq!(
            store.validate_and_consume_qr(&payload, now + 2_000),
            Err(P2Error::ReplayedQr)
        );

        let (another, _) = signed_payload(now + 10_000);
        let tampered = another.replace("Rooftop Disco", "Basement Disco");
        assert_eq!(
            store.validate_and_consume_qr(&tampered, now + 11_000),
            Err(P2Error::InvalidSignature)
        );
        assert_eq!(
            store.validate_and_consume_qr(
                &another,
                now + MAX_QR_LIFETIME_MS + QR_CLOCK_SKEW_MS + 20_000
            ),
            Err(P2Error::ExpiredQr)
        );
        drop(store);
        remove_database(&path);
    }

    /// Block 31.1/31.3: a desktop host's invitation carries a real
    /// connection endpoint, embedded verbatim and validated/round-tripped
    /// through the signed envelope exactly like every other field.
    #[test]
    fn qr_with_a_valid_embedded_connection_payload_round_trips() {
        let path = test_path("qr-conn-valid");
        let now = 2_000_000_000_u64;
        let (qr_input, signing_key) = input(now, Some(connection_payload_json()));
        let payload = sign(&qr_input, &signing_key);
        let mut store = P2Store::open(&path).expect("open");
        let validated = store
            .validate_and_consume_qr(&payload, now + 1_000)
            .expect("validate");
        assert_eq!(
            validated.connection_payload_json,
            Some(connection_payload_json())
        );
        drop(store);
        remove_database(&path);
    }

    /// A QR with no embedded connection payload (Android's own
    /// peer-to-peer QR flow) must keep validating exactly as before --
    /// this field is additive, not required.
    #[test]
    fn qr_without_a_connection_payload_still_validates() {
        let path = test_path("qr-conn-absent");
        let now = 2_000_000_000_u64;
        let (payload, _) = signed_payload(now);
        let mut store = P2Store::open(&path).expect("open");
        let validated = store
            .validate_and_consume_qr(&payload, now + 1_000)
            .expect("validate");
        assert_eq!(validated.connection_payload_json, None);
        drop(store);
        remove_database(&path);
    }

    /// Tampering with the embedded connection payload is caught the same
    /// way as tampering with any other signed field: the canonical form no
    /// longer matches what was signed.
    #[test]
    fn qr_with_a_tampered_connection_payload_is_rejected() {
        let path = test_path("qr-conn-tampered");
        let now = 2_000_000_000_u64;
        let (qr_input, signing_key) = input(now, Some(connection_payload_json()));
        let payload = sign(&qr_input, &signing_key);
        let tampered = payload.replace("192.168.1.50", "10.0.0.99");
        let mut store = P2Store::open(&path).expect("open");
        assert_eq!(
            store.validate_and_consume_qr(&tampered, now + 1_000),
            Err(P2Error::InvalidSignature)
        );
        drop(store);
        remove_database(&path);
    }

    /// An oversized connection payload is rejected up front, before the
    /// host ever signs it -- never surfaced only once a listener tries to
    /// act on an unusable invitation.
    #[test]
    fn qr_with_an_oversized_connection_payload_is_rejected_before_signing() {
        let (mut qr_input, _signing_key) = input(2_000_000_000_u64, None);
        let filler = "a".repeat(MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES + 1);
        qr_input.connection_payload_json = Some(format!(r#"{{"hostAddress":"{filler}"}}"#));
        assert_eq!(
            prepare_unsigned_qr(&qr_input),
            Err(P2Error::InvalidQr(
                "connection payload size is unsupported".to_owned()
            ))
        );
    }

    /// A structurally invalid connection payload (not the manual-endpoint
    /// JSON shape at all) is rejected up front too, via the same
    /// `ManualHostEndpoint::parse` the manual paste flow already uses.
    #[test]
    fn qr_with_a_structurally_invalid_connection_payload_is_rejected_before_signing() {
        let (mut qr_input, _signing_key) = input(2_000_000_000_u64, None);
        qr_input.connection_payload_json = Some("not json".to_owned());
        let error = prepare_unsigned_qr(&qr_input).expect_err("must reject malformed JSON");
        match error {
            P2Error::InvalidQr(message) => {
                assert!(message.starts_with("connection payload is invalid:"));
            }
            other => panic!("expected InvalidQr, got {other:?}"),
        }
    }
}
