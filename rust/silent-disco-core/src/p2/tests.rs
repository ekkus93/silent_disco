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

    fn signed_payload(now: u64) -> (String, Vec<u8>) {
        let signing_key = SigningKey::from_bytes((&[7_u8; 32]).into()).expect("fixed key");
        let public_key = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("public DER")
            .as_bytes()
            .to_vec();
        let unsigned = prepare_unsigned_qr(&QrInvitationInput {
            session_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            session_name: "Rooftop Disco".to_owned(),
            host_name: "Phillip's host".to_owned(),
            host_public_key_der: public_key.clone(),
            approval_mode: "invite_code".to_owned(),
            invite_code: Some("4826".to_owned()),
            issued_at_ms: now,
            expires_at_ms: now + 300_000,
            nonce: "nonce-1234567890abcdef".to_owned(),
        })
        .expect("unsigned payload");
        let signature: Signature = signing_key.sign(unsigned.as_bytes());
        let payload = finalize_qr(
            &unsigned,
            &URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes()),
        )
        .expect("signed payload");
        (payload, public_key)
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
}
