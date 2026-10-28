//! Handshake-bound listener identity proof for the planned secure-v4 transport.
//!
//! This verifier is deliberately independent of claimed device IDs. It is
//! scaffolding: callers must supply a fresh unpredictable nonce and a TLS 1.3
//! exporter binding from an authenticated channel. Legacy v3 sockets do not
//! provide either and MUST NOT use this to claim verified admission.
use p256::{
    ecdsa::{Signature, VerifyingKey, signature::Verifier},
    pkcs8::DecodePublicKey,
};
use sha2::{Digest, Sha256};

use crate::domain::{DeviceId, SessionId};

const PROOF_DOMAIN: &[u8] = b"silent-disco-v4-listener-proof\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerProofError {
    InvalidChallenge,
    InvalidPublicKey,
    InvalidSignatureEncoding,
    SignatureMismatch,
    AlreadyConsumed,
}

/// A single-use challenge tied to one authenticated TLS channel and generation.
#[derive(Debug)]
pub struct PendingListenerProof {
    session_id: SessionId,
    generation: u64,
    host_fingerprint: [u8; 32],
    nonce: [u8; 32],
    exporter_binding: [u8; 32],
    consumed: bool,
}

/// Identity derived from verified possession of a listener's P-256 key.
/// The fingerprint cannot be supplied by a peer as an unchecked trust enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerifiedListenerKey {
    fingerprint: [u8; 32],
    host_fingerprint: [u8; 32],
}

impl VerifiedListenerKey {
    #[must_use]
    pub const fn fingerprint(&self) -> &[u8; 32] {
        &self.fingerprint
    }

    /// Host identity whose authenticated TLS-bound challenge was answered.
    #[must_use]
    pub const fn host_fingerprint(&self) -> &[u8; 32] {
        &self.host_fingerprint
    }

    /// Stable v4 routing principal, derived only from the verified public key.
    ///
    /// Never use a peer-claimed device ID for admission, UDP authorization,
    /// reconnect ownership or playback control after successful v4 proof.
    ///
    /// # Panics
    /// Only if the domain's `DeviceId` validator rejects the fixed `v4-`
    /// prefix followed by 64 lowercase hexadecimal bytes, despite satisfying
    /// its documented validation rules.
    #[must_use]
    pub fn canonical_device_id(&self) -> DeviceId {
        use std::fmt::Write;
        let mut value = String::with_capacity(3 + self.fingerprint.len() * 2);
        value.push_str("v4-");
        for byte in self.fingerprint {
            write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
        }
        DeviceId::new(value).expect("fixed-length hexadecimal device ID is valid")
    }
}

fn fingerprint(public_key_der: &[u8]) -> [u8; 32] {
    let mut output = [0_u8; 32];
    output.copy_from_slice(&Sha256::digest(public_key_der));
    output
}

impl PendingListenerProof {
    /// Rejects empty bindings and malformed host keys before issuing a proof.
    ///
    /// # Errors
    /// Returns `InvalidChallenge` when no unique generation, nonce or
    /// exporter binding is supplied, or `InvalidPublicKey` for an invalid host key.
    pub fn new(
        session_id: SessionId,
        generation: u64,
        host_public_key_der: &[u8],
        nonce: [u8; 32],
        exporter_binding: [u8; 32],
    ) -> Result<Self, ListenerProofError> {
        if generation == 0 || nonce == [0; 32] || exporter_binding == [0; 32] {
            return Err(ListenerProofError::InvalidChallenge);
        }
        VerifyingKey::from_public_key_der(host_public_key_der)
            .map_err(|_| ListenerProofError::InvalidPublicKey)?;
        Ok(Self {
            session_id,
            generation,
            host_fingerprint: fingerprint(host_public_key_der),
            nonce,
            exporter_binding,
            consumed: false,
        })
    }

    /// Canonical domain-separated bytes signed by the listener's OS-backed key.
    /// This includes both public-key principals, session, generation, nonce and
    /// authenticated TLS exporter. No untrusted display/device ID is included.
    ///
    /// # Errors
    /// Returns `InvalidPublicKey` for malformed listener keys.
    pub fn signing_transcript(
        &self,
        listener_public_key_der: &[u8],
    ) -> Result<Vec<u8>, ListenerProofError> {
        VerifyingKey::from_public_key_der(listener_public_key_der)
            .map_err(|_| ListenerProofError::InvalidPublicKey)?;
        let session = self.session_id.as_str().as_bytes();
        let session_len =
            u16::try_from(session.len()).map_err(|_| ListenerProofError::InvalidChallenge)?;
        let mut transcript = Vec::with_capacity(PROOF_DOMAIN.len() + 32 * 4 + 10 + session.len());
        transcript.extend_from_slice(PROOF_DOMAIN);
        transcript.extend_from_slice(&self.host_fingerprint);
        transcript.extend_from_slice(&fingerprint(listener_public_key_der));
        transcript.extend_from_slice(&session_len.to_be_bytes());
        transcript.extend_from_slice(session);
        transcript.extend_from_slice(&self.generation.to_be_bytes());
        transcript.extend_from_slice(&self.nonce);
        transcript.extend_from_slice(&self.exporter_binding);
        Ok(transcript)
    }

    /// Consumes the challenge on the FIRST verification attempt, successful or
    /// not. A failed signature cannot be retried against the same challenge.
    ///
    /// # Errors
    /// Returns a typed error for replay, malformed keys/signatures or mismatch.
    pub fn verify_and_consume(
        &mut self,
        listener_public_key_der: &[u8],
        signature_der: &[u8],
    ) -> Result<VerifiedListenerKey, ListenerProofError> {
        if self.consumed {
            return Err(ListenerProofError::AlreadyConsumed);
        }
        self.consumed = true;
        let transcript = self.signing_transcript(listener_public_key_der)?;
        let key = VerifyingKey::from_public_key_der(listener_public_key_der)
            .map_err(|_| ListenerProofError::InvalidPublicKey)?;
        let signature = Signature::from_der(signature_der)
            .map_err(|_| ListenerProofError::InvalidSignatureEncoding)?;
        key.verify(&transcript, &signature)
            .map_err(|_| ListenerProofError::SignatureMismatch)?;
        Ok(VerifiedListenerKey {
            fingerprint: fingerprint(listener_public_key_der),
            host_fingerprint: self.host_fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ListenerProofError, PendingListenerProof, fingerprint};
    use crate::domain::SessionId;
    use p256::{
        ecdsa::{Signature, SigningKey, signature::Signer},
        pkcs8::EncodePublicKey,
    };

    fn key(seed: u8) -> (SigningKey, Vec<u8>) {
        let signing = SigningKey::from_slice(&[seed; 32]).expect("valid fixture private key");
        let der = signing
            .verifying_key()
            .to_public_key_der()
            .expect("public key DER")
            .as_bytes()
            .to_vec();
        (signing, der)
    }

    fn pending(host: &[u8]) -> PendingListenerProof {
        PendingListenerProof::new(
            SessionId::new("session-1").expect("session"),
            7,
            host,
            [11; 32],
            [19; 32],
        )
        .expect("valid challenge")
    }

    fn sign(proof: &PendingListenerProof, key: &SigningKey, der: &[u8]) -> Vec<u8> {
        let message = proof.signing_transcript(der).expect("transcript");
        let signature: Signature = key.sign(&message);
        signature.to_der().as_bytes().to_vec()
    }

    #[test]
    fn verified_identity_is_key_bound_and_challenge_is_single_use() {
        let (_, host) = key(3);
        let (listener_key, listener) = key(5);
        let mut proof = pending(&host);
        let signature = sign(&proof, &listener_key, &listener);
        let verified = proof
            .verify_and_consume(&listener, &signature)
            .expect("valid listener proof");
        assert_eq!(*verified.fingerprint(), fingerprint(&listener));
        assert_ne!(*verified.fingerprint(), fingerprint(&host));
        assert_eq!(
            proof.verify_and_consume(&listener, &signature),
            Err(ListenerProofError::AlreadyConsumed)
        );
    }

    #[test]
    fn tampered_channel_session_generation_host_or_nonce_fails_closed() {
        let (_, host) = key(3);
        let (_, forged_host) = key(9);
        let (listener_key, listener) = key(5);
        let original = pending(&host);
        let signature = sign(&original, &listener_key, &listener);
        let variants = [
            PendingListenerProof::new(
                SessionId::new("other-session").expect("session"),
                7,
                &host,
                [11; 32],
                [19; 32],
            )
            .expect("session variant"),
            PendingListenerProof::new(
                SessionId::new("session-1").expect("session"),
                8,
                &host,
                [11; 32],
                [19; 32],
            )
            .expect("generation variant"),
            PendingListenerProof::new(
                SessionId::new("session-1").expect("session"),
                7,
                &forged_host,
                [11; 32],
                [19; 32],
            )
            .expect("host variant"),
            PendingListenerProof::new(
                SessionId::new("session-1").expect("session"),
                7,
                &host,
                [12; 32],
                [19; 32],
            )
            .expect("nonce variant"),
            PendingListenerProof::new(
                SessionId::new("session-1").expect("session"),
                7,
                &host,
                [11; 32],
                [20; 32],
            )
            .expect("exporter variant"),
        ];
        for mut variant in variants {
            assert_eq!(
                variant.verify_and_consume(&listener, &signature),
                Err(ListenerProofError::SignatureMismatch)
            );
            assert_eq!(
                variant.verify_and_consume(&listener, &signature),
                Err(ListenerProofError::AlreadyConsumed)
            );
        }
    }

    #[test]
    fn wrong_key_malformed_signature_and_missing_channel_binding_are_rejected() {
        let (_, host) = key(3);
        let (listener_key, listener) = key(5);
        let (_, impostor) = key(6);
        let original = pending(&host);
        let signature = sign(&original, &listener_key, &listener);
        assert_eq!(
            pending(&host).verify_and_consume(&impostor, &signature),
            Err(ListenerProofError::SignatureMismatch)
        );
        assert_eq!(
            pending(&host).verify_and_consume(&listener, &[0xff, 0x01]),
            Err(ListenerProofError::InvalidSignatureEncoding)
        );
        assert_eq!(
            pending(&host).verify_and_consume(&[1, 2, 3], &signature),
            Err(ListenerProofError::InvalidPublicKey)
        );
        assert_eq!(
            PendingListenerProof::new(
                SessionId::new("session-1").expect("session"),
                0,
                &host,
                [11; 32],
                [19; 32],
            )
            .unwrap_err(),
            ListenerProofError::InvalidChallenge
        );
        assert_eq!(
            PendingListenerProof::new(
                SessionId::new("session-1").expect("session"),
                7,
                &host,
                [11; 32],
                [0; 32],
            )
            .unwrap_err(),
            ListenerProofError::InvalidChallenge
        );
    }
}
