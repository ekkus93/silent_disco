//! Secure-v4 admission boundary: never promote peer-claimed trust into Rust policy.
//!
//! The caller must obtain the proof's nonce and exporter binding from an
//! authenticated TLS 1.3 channel. This module does not enable secure-v4 sockets;
//! legacy-v3 callers must not synthesize a proof or use this entry point.
use crate::domain::TrustState;
use crate::transport::secure_identity::{
    ListenerProofError, PendingListenerProof, VerifiedListenerKey,
};
use crate::transport::secure_trust::{HostVerifiedTrust, VerifiedTrustDisposition};

use super::{HostDraft, JoinRequestDisposition, JoinRequestSummary, classify_join_request};

/// A request whose trust classification was derived from a verified key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedJoinAdmission {
    verified_key: VerifiedListenerKey,
    request: JoinRequestSummary,
    disposition: JoinRequestDisposition,
}

impl VerifiedJoinAdmission {
    #[must_use]
    pub const fn verified_key(&self) -> VerifiedListenerKey {
        self.verified_key
    }

    #[must_use]
    pub const fn request(&self) -> &JoinRequestSummary {
        &self.request
    }

    #[must_use]
    pub const fn disposition(&self) -> JoinRequestDisposition {
        self.disposition
    }
}

/// Failure is terminal for this challenge and must be reported to the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedJoinError {
    Proof(ListenerProofError),
    RevokedListener,
}

/// Verify possession and classify a join using only host-owned trust records.
///
/// The incoming `request.trust_state` is deliberately ignored: it is not
/// cryptographic evidence. A revoked key is refused even in Manual/InviteCode
/// mode, and a failed proof cannot be retried against the consumed challenge.
///
/// # Errors
///
/// Rejects invalid, replayed or mismatched proof and revoked enrollment.
pub fn verify_and_classify_join(
    draft: &HostDraft,
    request: &JoinRequestSummary,
    proof: &mut PendingListenerProof,
    listener_public_key_der: &[u8],
    signature_der: &[u8],
    trust: &HostVerifiedTrust,
) -> Result<VerifiedJoinAdmission, VerifiedJoinError> {
    let verified_key = proof
        .verify_and_consume(listener_public_key_der, signature_der)
        .map_err(VerifiedJoinError::Proof)?;
    let state = match trust.disposition(&verified_key) {
        VerifiedTrustDisposition::Trusted => TrustState::Trusted,
        VerifiedTrustDisposition::Unknown => TrustState::SessionOnly,
        VerifiedTrustDisposition::Revoked => return Err(VerifiedJoinError::RevokedListener),
    };
    let mut verified_request = request.clone();
    // A signed listener must not seize an existing listener's routing slot by
    // claiming its legacy device ID. All v4 actor/transport ownership uses the
    // proven key's stable principal, including reconnect and authorization.
    verified_request.device_id = verified_key.canonical_device_id();
    verified_request.trust_state = state;
    let disposition = classify_join_request(draft, &verified_request);
    Ok(VerifiedJoinAdmission {
        verified_key,
        request: verified_request,
        disposition,
    })
}

#[cfg(test)]
mod tests {
    use super::{VerifiedJoinError, verify_and_classify_join};
    use crate::domain::{
        ApprovalMode, DeviceId, MonotonicMillis, RequestId, SessionId, TrustState,
    };
    use crate::runtime::{
        HostDraft, JoinRejectionReason, JoinRequestDisposition, JoinRequestSummary,
    };
    use crate::transport::secure_identity::{ListenerProofError, PendingListenerProof};
    use crate::transport::secure_trust::HostVerifiedTrust;
    use p256::{
        ecdsa::{Signature, SigningKey, signature::Signer},
        pkcs8::EncodePublicKey,
    };

    fn key(seed: u8) -> (SigningKey, Vec<u8>) {
        let signing = SigningKey::from_slice(&[seed; 32]).expect("fixture key");
        let der = signing
            .verifying_key()
            .to_public_key_der()
            .expect("public key DER")
            .as_bytes()
            .to_vec();
        (signing, der)
    }

    fn challenge(host: &[u8], generation: u64) -> PendingListenerProof {
        PendingListenerProof::new(
            SessionId::new("session-1").expect("session"),
            generation,
            host,
            [11; 32],
            [19; 32],
        )
        .expect("challenge")
    }

    fn signature(proof: &PendingListenerProof, signing: &SigningKey, listener: &[u8]) -> Vec<u8> {
        let transcript = proof.signing_transcript(listener).expect("transcript");
        let signed: Signature = signing.sign(&transcript);
        signed.to_der().as_bytes().to_vec()
    }

    fn request(claimed_trust: TrustState) -> JoinRequestSummary {
        JoinRequestSummary::new(
            RequestId::new("join-1").expect("request"),
            DeviceId::new("claimed-device").expect("device"),
            "Listener",
            claimed_trust,
            true,
            MonotonicMillis::new(10),
        )
        .expect("join")
    }

    #[test]
    fn approved_only_uses_verified_host_records_not_claimed_trust() {
        let (_, host) = key(3);
        let (signing, listener) = key(5);
        let mut registry = HostVerifiedTrust::new(&host).expect("registry");
        let draft = HostDraft {
            approval_mode: ApprovalMode::TrustedDevices,
            ..HostDraft::default()
        };
        let mut first = challenge(&host, 1);
        let signed = signature(&first, &signing, &listener);
        let unknown = verify_and_classify_join(
            &draft,
            &request(TrustState::Trusted),
            &mut first,
            &listener,
            &signed,
            &registry,
        )
        .expect("valid proof");
        assert_eq!(unknown.request().trust_state, TrustState::SessionOnly);
        assert_eq!(unknown.request().device_id, unknown.verified_key().canonical_device_id());
        assert_ne!(unknown.request().device_id, request(TrustState::Trusted).device_id);
        assert_eq!(
            unknown.disposition(),
            JoinRequestDisposition::AutoReject {
                reason: JoinRejectionReason::DeviceNotApproved,
            }
        );
        registry
            .enroll(unknown.verified_key(), "Kitchen")
            .expect("enroll");
        let persisted = registry.to_json().expect("persist");
        let registry = HostVerifiedTrust::from_json(&host, &persisted).expect("restart");
        let mut second = challenge(&host, 2);
        let signed = signature(&second, &signing, &listener);
        let trusted = verify_and_classify_join(
            &draft,
            &request(TrustState::SessionOnly),
            &mut second,
            &listener,
            &signed,
            &registry,
        )
        .expect("verified trusted");
        assert_eq!(trusted.request().trust_state, TrustState::Trusted);
        assert_eq!(trusted.request().device_id, unknown.request().device_id);
        assert_eq!(
            trusted.disposition(),
            JoinRequestDisposition::AutoApprove {
                trusted_for_future: true,
            }
        );
    }

    #[test]
    fn revocation_blocks_all_modes_and_wrong_key_cannot_replay() {
        let (_, host) = key(3);
        let (signing, listener) = key(5);
        let (_, impostor) = key(6);
        let mut registry = HostVerifiedTrust::new(&host).expect("registry");
        let mut initial = challenge(&host, 1);
        let signed = signature(&initial, &signing, &listener);
        let known = initial
            .verify_and_consume(&listener, &signed)
            .expect("verified");
        registry.enroll(known, "Kitchen").expect("enroll");
        assert!(registry.revoke(&known));
        for mode in [
            ApprovalMode::Manual,
            ApprovalMode::InviteCode,
            ApprovalMode::TrustedDevices,
        ] {
            let draft = HostDraft {
                approval_mode: mode,
                ..HostDraft::default()
            };
            let mut proof = challenge(&host, 2);
            let signed = signature(&proof, &signing, &listener);
            assert_eq!(
                verify_and_classify_join(
                    &draft,
                    &request(TrustState::Trusted),
                    &mut proof,
                    &listener,
                    &signed,
                    &registry,
                ),
                Err(VerifiedJoinError::RevokedListener)
            );
        }
        let mut proof = challenge(&host, 3);
        let signed = signature(&proof, &signing, &listener);
        let draft = HostDraft::default();
        assert_eq!(
            verify_and_classify_join(
                &draft,
                &request(TrustState::Trusted),
                &mut proof,
                &impostor,
                &signed,
                &registry,
            ),
            Err(VerifiedJoinError::Proof(
                ListenerProofError::SignatureMismatch
            ))
        );
        assert_eq!(
            verify_and_classify_join(
                &draft,
                &request(TrustState::Trusted),
                &mut proof,
                &listener,
                &signed,
                &registry,
            ),
            Err(VerifiedJoinError::Proof(
                ListenerProofError::AlreadyConsumed
            ))
        );
    }


    #[test]
    fn forged_claimed_device_id_cannot_collide_with_another_verified_listener() {
        let (_, host) = key(3);
        let (first_signing, first_der) = key(5);
        let (second_signing, second_der) = key(6);
        let registry = HostVerifiedTrust::new(&host).expect("registry");
        let draft = HostDraft::default();
        let claimed = request(TrustState::Trusted);

        let mut first_proof = challenge(&host, 1);
        let first_signature = signature(&first_proof, &first_signing, &first_der);
        let first = verify_and_classify_join(
            &draft, &claimed, &mut first_proof, &first_der, &first_signature, &registry,
        )
        .expect("first verified join");

        // Same claimed ID, different signing key: distinct actor/UDP principals.
        let mut second_proof = challenge(&host, 2);
        let second_signature = signature(&second_proof, &second_signing, &second_der);
        let second = verify_and_classify_join(
            &draft, &claimed, &mut second_proof, &second_der, &second_signature, &registry,
        )
        .expect("second verified join");
        assert_ne!(first.request().device_id, second.request().device_id);
        assert_ne!(first.verified_key(), second.verified_key());
        assert_ne!(first.request().device_id, claimed.device_id);

        // Same signing key, different self-reported ID: stable reconnect owner.
        let mut renamed = request(TrustState::SessionOnly);
        renamed.device_id = DeviceId::new("forged-other-id").expect("claimed ID");
        let mut reconnect_proof = challenge(&host, 3);
        let reconnect_signature = signature(&reconnect_proof, &first_signing, &first_der);
        let reconnect = verify_and_classify_join(
            &draft, &renamed, &mut reconnect_proof, &first_der, &reconnect_signature, &registry,
        )
        .expect("reconnect verified");
        assert_eq!(first.request().device_id, reconnect.request().device_id);
        assert_eq!(first.verified_key(), reconnect.verified_key());
    }
    #[test]
    fn manual_and_invite_policy_still_require_explicit_approval() {
        let (_, host) = key(3);
        let (signing, listener) = key(5);
        let registry = HostVerifiedTrust::new(&host).expect("registry");
        for mode in [ApprovalMode::Manual, ApprovalMode::InviteCode] {
            let draft = HostDraft {
                approval_mode: mode,
                ..HostDraft::default()
            };
            let mut proof = challenge(&host, 4);
            let signed = signature(&proof, &signing, &listener);
            let admitted = verify_and_classify_join(
                &draft,
                &request(TrustState::Trusted),
                &mut proof,
                &listener,
                &signed,
                &registry,
            )
            .expect("verified");
            assert_eq!(
                admitted.disposition(),
                JoinRequestDisposition::PendingManualDecision
            );
            assert_eq!(admitted.request().trust_state, TrustState::SessionOnly);
        }
    }
}
