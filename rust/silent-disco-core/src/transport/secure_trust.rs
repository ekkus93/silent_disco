//! Host-owned public-key trust records for secure-v4 admission.
//!
//! A record can be enrolled only from a `VerifiedListenerKey`, which is
//! constructible solely by a successful, single-use, channel-bound signature
//! proof. This module does not authenticate legacy-v3 sockets or enable their
//! auto-admission. Platform adapters must protect the serialized records with
//! the host's OS credential store and retain the private key outside this JSON.
use std::collections::BTreeMap;

use p256::{ecdsa::VerifyingKey, pkcs8::DecodePublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::secure_identity::VerifiedListenerKey;

const TRUST_RECORD_VERSION: u32 = 1;
const MAX_TRUSTED_LISTENERS: usize = 256;
const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_SNAPSHOT_BYTES: usize = 131_072;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostTrustError {
    InvalidHostKey,
    InvalidSnapshot,
    WrongHostIdentity,
    UnsupportedVersion,
    TooManyListeners,
    InvalidDisplayName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedTrustDisposition {
    Trusted,
    Unknown,
    Revoked,
}

/// Opaque identity for a listener already present in this host's trust registry.
/// Returned by `stored_listeners`; not derived from a peer-claimed device ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredListenerId {
    fingerprint: [u8; 32],
    host_fingerprint: [u8; 32],
}

impl StoredListenerId {
    #[must_use]
    pub const fn fingerprint(&self) -> &[u8; 32] {
        &self.fingerprint
    }
}

/// Host-owned, non-secret metadata for enrollment and offline revocation UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredListenerSummary {
    pub id: StoredListenerId,
    pub display_name: String,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredListener {
    fingerprint: [u8; 32],
    display_name: String,
    revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustSnapshot {
    version: u32,
    host_fingerprint: [u8; 32],
    listeners: Vec<StoredListener>,
}

/// Durable host-owned trust policy, scoped to the host's public-key identity.
///
/// A peer-supplied device ID, display name or trust enum cannot be passed to
/// `disposition` or `enroll`; callers must first complete the v4 TLS-bound
/// possession proof. Enrollment additionally requires an explicit host action.
#[derive(Debug, Clone)]
pub struct HostVerifiedTrust {
    host_fingerprint: [u8; 32],
    listeners: BTreeMap<[u8; 32], StoredListener>,
}

fn host_fingerprint(host_public_key_der: &[u8]) -> Result<[u8; 32], HostTrustError> {
    VerifyingKey::from_public_key_der(host_public_key_der)
        .map_err(|_| HostTrustError::InvalidHostKey)?;
    let mut fingerprint = [0_u8; 32];
    fingerprint.copy_from_slice(&Sha256::digest(host_public_key_der));
    Ok(fingerprint)
}

fn valid_display_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= MAX_DISPLAY_NAME_BYTES
        && !name.chars().any(char::is_control)
}

impl HostVerifiedTrust {
    /// Creates an empty trust registry for one validated host public key.
    ///
    /// # Errors
    /// Rejects an invalid P-256 host public key.
    pub fn new(host_public_key_der: &[u8]) -> Result<Self, HostTrustError> {
        Ok(Self {
            host_fingerprint: host_fingerprint(host_public_key_der)?,
            listeners: BTreeMap::new(),
        })
    }

    /// Loads only a versioned snapshot bound to this exact host public key.
    /// Legacy device-ID records are deliberately not migrated to verified trust.
    ///
    /// # Errors
    /// Rejects corrupt, oversized, duplicated, mismatched or unsupported data.
    pub fn from_json(host_public_key_der: &[u8], json: &str) -> Result<Self, HostTrustError> {
        let expected_host = host_fingerprint(host_public_key_der)?;
        if json.len() > MAX_SNAPSHOT_BYTES {
            return Err(HostTrustError::InvalidSnapshot);
        }
        let snapshot: TrustSnapshot =
            serde_json::from_str(json).map_err(|_| HostTrustError::InvalidSnapshot)?;
        if snapshot.version != TRUST_RECORD_VERSION {
            return Err(HostTrustError::UnsupportedVersion);
        }
        if snapshot.host_fingerprint != expected_host {
            return Err(HostTrustError::WrongHostIdentity);
        }
        if snapshot.listeners.len() > MAX_TRUSTED_LISTENERS {
            return Err(HostTrustError::TooManyListeners);
        }
        let mut listeners = BTreeMap::new();
        for record in snapshot.listeners {
            if record.fingerprint == [0; 32]
                || !valid_display_name(&record.display_name)
                || listeners.insert(record.fingerprint, record).is_some()
            {
                return Err(HostTrustError::InvalidSnapshot);
            }
        }
        Ok(Self {
            host_fingerprint: expected_host,
            listeners,
        })
    }

    /// Exports only public fingerprints, display names and revocation flags.
    /// The caller must use OS-protected, atomic storage for this snapshot.
    ///
    /// # Errors
    /// Returns `InvalidSnapshot` if serialization unexpectedly fails.
    pub fn to_json(&self) -> Result<String, HostTrustError> {
        serde_json::to_string(&TrustSnapshot {
            version: TRUST_RECORD_VERSION,
            host_fingerprint: self.host_fingerprint,
            listeners: self.listeners.values().cloned().collect(),
        })
        .map_err(|_| HostTrustError::InvalidSnapshot)
    }

    /// Classifies a *verified* listener key, never a claimed device identifier.
    #[must_use]
    pub fn disposition(&self, key: &VerifiedListenerKey) -> VerifiedTrustDisposition {
        if key.host_fingerprint() != &self.host_fingerprint {
            return VerifiedTrustDisposition::Unknown;
        }
        match self.listeners.get(key.fingerprint()) {
            Some(record) if record.revoked => VerifiedTrustDisposition::Revoked,
            Some(_) => VerifiedTrustDisposition::Trusted,
            None => VerifiedTrustDisposition::Unknown,
        }
    }

    /// Persists an explicit "Always allow" enrollment for a verified key.
    /// Re-enrolling a revoked key is an explicit host action, not a reconnect.
    ///
    /// # Errors
    /// Rejects invalid names and a full registry without changing its contents.
    pub fn enroll(
        &mut self,
        key: VerifiedListenerKey,
        display_name: &str,
    ) -> Result<(), HostTrustError> {
        if key.host_fingerprint() != &self.host_fingerprint {
            return Err(HostTrustError::WrongHostIdentity);
        }
        if !valid_display_name(display_name) {
            return Err(HostTrustError::InvalidDisplayName);
        }
        if !self.listeners.contains_key(key.fingerprint())
            && self.listeners.len() >= MAX_TRUSTED_LISTENERS
        {
            return Err(HostTrustError::TooManyListeners);
        }
        self.listeners.insert(
            *key.fingerprint(),
            StoredListener {
                fingerprint: *key.fingerprint(),
                display_name: display_name.to_owned(),
                revoked: false,
            },
        );
        Ok(())
    }

    /// Lists stored enrollments, including revoked ones, for explicit host UI.
    /// The opaque ID comes from this registry, never from an incoming join.
    #[must_use]
    pub fn stored_listeners(&self) -> Vec<StoredListenerSummary> {
        self.listeners
            .values()
            .map(|record| StoredListenerSummary {
                id: StoredListenerId {
                    fingerprint: record.fingerprint,
                    host_fingerprint: self.host_fingerprint,
                },
                display_name: record.display_name.clone(),
                revoked: record.revoked,
            })
            .collect()
    }

    /// Revokes an enrolled listener while it is offline, without requiring a
    /// fresh possession proof from the listener being removed. Unknown IDs
    /// cannot create records or affect any other enrollment.
    pub fn revoke_stored(&mut self, id: StoredListenerId) -> bool {
        if id.host_fingerprint != self.host_fingerprint {
            return false;
        }
        if let Some(record) = self.listeners.get_mut(&id.fingerprint) {
            record.revoked = true;
            true
        } else {
            false
        }
    }

    /// Revocation survives serialization and blocks subsequent verified joins.
    /// Unknown identities are not implicitly inserted as revoked records.
    pub fn revoke(&mut self, key: &VerifiedListenerKey) -> bool {
        if key.host_fingerprint() != &self.host_fingerprint {
            return false;
        }
        if let Some(record) = self.listeners.get_mut(key.fingerprint()) {
            record.revoked = true;
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.listeners.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.listeners.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{HostTrustError, HostVerifiedTrust, TrustSnapshot, VerifiedTrustDisposition};
    use crate::domain::SessionId;
    use crate::transport::secure_identity::{PendingListenerProof, VerifiedListenerKey};
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

    fn verified(host: &[u8], signing: &SigningKey, listener: &[u8]) -> VerifiedListenerKey {
        let mut proof = PendingListenerProof::new(
            SessionId::new("secure-session").expect("session"),
            1,
            host,
            [7; 32],
            [9; 32],
        )
        .expect("challenge");
        let transcript = proof.signing_transcript(listener).expect("transcript");
        let signature: Signature = signing.sign(&transcript);
        proof
            .verify_and_consume(listener, signature.to_der().as_bytes())
            .expect("verified possession")
    }

    #[test]
    fn only_verified_enrollment_grants_durable_trust_across_restart() {
        let (_, host) = key(3);
        let (signing, listener) = key(5);
        let (other_signing, other) = key(6);
        let listener = verified(&host, &signing, &listener);
        let other = verified(&host, &other_signing, &other);
        let mut store = HostVerifiedTrust::new(&host).expect("registry");
        assert_eq!(
            store.disposition(&listener),
            VerifiedTrustDisposition::Unknown
        );
        assert_eq!(store.disposition(&other), VerifiedTrustDisposition::Unknown);
        store
            .enroll(listener, "Living room")
            .expect("host enrollment");
        assert_eq!(
            store.disposition(&listener),
            VerifiedTrustDisposition::Trusted
        );
        assert_eq!(store.disposition(&other), VerifiedTrustDisposition::Unknown);
        let persisted = store.to_json().expect("snapshot");
        assert!(!persisted.contains("private"));
        let restored = HostVerifiedTrust::from_json(&host, &persisted).expect("restart");
        assert_eq!(
            restored.disposition(&listener),
            VerifiedTrustDisposition::Trusted
        );
        assert_eq!(
            restored.disposition(&other),
            VerifiedTrustDisposition::Unknown
        );
    }

    #[test]
    fn revocation_and_explicit_reenrollment_are_persistent() {
        let (_, host) = key(3);
        let (signing, listener) = key(5);
        let verified = verified(&host, &signing, &listener);
        let mut store = HostVerifiedTrust::new(&host).expect("registry");
        assert!(!store.revoke(&verified));
        store.enroll(verified, "Speaker").expect("enroll");
        assert!(store.revoke(&verified));
        assert_eq!(
            store.disposition(&verified),
            VerifiedTrustDisposition::Revoked
        );
        let json = store.to_json().expect("snapshot");
        let mut restored = HostVerifiedTrust::from_json(&host, &json).expect("reload");
        assert_eq!(
            restored.disposition(&verified),
            VerifiedTrustDisposition::Revoked
        );
        restored
            .enroll(verified, "Speaker")
            .expect("explicit re-enroll");
        assert_eq!(
            restored.disposition(&verified),
            VerifiedTrustDisposition::Trusted
        );
    }

    #[test]
    fn snapshots_reject_other_host_legacy_records_and_corruption() {
        let (_, host) = key(3);
        let (_, other_host) = key(9);
        let (signing, listener) = key(5);
        let verified = verified(&host, &signing, &listener);
        let mut store = HostVerifiedTrust::new(&host).expect("registry");
        store.enroll(verified, "Speaker").expect("enroll");
        let json = store.to_json().expect("snapshot");
        assert_eq!(
            HostVerifiedTrust::from_json(&other_host, &json).unwrap_err(),
            HostTrustError::WrongHostIdentity
        );
        assert_eq!(
            HostVerifiedTrust::from_json(&host, r#"{"listener_id":"speaker","trusted":true}"#)
                .unwrap_err(),
            HostTrustError::InvalidSnapshot
        );
        assert_eq!(
            HostVerifiedTrust::from_json(&host, "not-json").unwrap_err(),
            HostTrustError::InvalidSnapshot
        );
        let mut snapshot: TrustSnapshot = serde_json::from_str(&json).expect("valid snapshot");
        snapshot.version += 1;
        assert_eq!(
            HostVerifiedTrust::from_json(
                &host,
                &serde_json::to_string(&snapshot).expect("serialize")
            )
            .unwrap_err(),
            HostTrustError::UnsupportedVersion
        );
        snapshot.version -= 1;
        snapshot.listeners.push(snapshot.listeners[0].clone());
        assert_eq!(
            HostVerifiedTrust::from_json(
                &host,
                &serde_json::to_string(&snapshot).expect("serialize")
            )
            .unwrap_err(),
            HostTrustError::InvalidSnapshot
        );
    }

    #[test]
    fn offline_revocation_is_host_owned_persistent_and_does_not_trust_claimed_ids() {
        let (_, host) = key(3);
        let (first_key, first_der) = key(5);
        let (other_key, other_der) = key(6);
        let first = verified(&host, &first_key, &first_der);
        let other = verified(&host, &other_key, &other_der);
        let mut store = HostVerifiedTrust::new(&host).expect("registry");
        store.enroll(first, "Kitchen").expect("enroll first");
        let first_id = store.stored_listeners()[0].id;
        assert_eq!(first_id.fingerprint(), first.fingerprint());
        assert_eq!(store.stored_listeners()[0].display_name, "Kitchen");
        assert!(!store.stored_listeners()[0].revoked);
        assert!(store.revoke_stored(first_id));
        assert!(store.stored_listeners()[0].revoked);
        assert_eq!(store.disposition(&first), VerifiedTrustDisposition::Revoked);
        assert_eq!(store.disposition(&other), VerifiedTrustDisposition::Unknown);

        let snapshot = store.to_json().expect("snapshot");
        let mut restored = HostVerifiedTrust::from_json(&host, &snapshot).expect("restart");
        assert!(restored.stored_listeners()[0].revoked);
        assert_eq!(
            restored.disposition(&first),
            VerifiedTrustDisposition::Revoked
        );
        assert!(restored.revoke_stored(first_id));
        restored.enroll(other, "Office").expect("enroll other");
        let other_id = restored
            .stored_listeners()
            .into_iter()
            .find(|record| record.id.fingerprint() == other.fingerprint())
            .expect("second enrollment")
            .id;
        let mut unrelated = HostVerifiedTrust::new(&host).expect("unrelated registry");
        unrelated
            .enroll(first, "Kitchen")
            .expect("unrelated enrollment");
        assert!(!unrelated.revoke_stored(other_id));
        assert_eq!(
            unrelated.disposition(&first),
            VerifiedTrustDisposition::Trusted
        );
        assert_eq!(unrelated.len(), 1);
    }

    #[test]
    fn proofs_and_offline_revocation_handles_cannot_cross_host_boundaries() {
        let (_, host) = key(3);
        let (_, other_host) = key(9);
        let (signing, listener) = key(5);
        let own = verified(&host, &signing, &listener);
        let foreign = verified(&other_host, &signing, &listener);
        let mut store = HostVerifiedTrust::new(&host).expect("host registry");
        let mut other_store = HostVerifiedTrust::new(&other_host).expect("other registry");
        store.enroll(own, "Kitchen").expect("enroll");
        other_store
            .enroll(foreign, "Kitchen")
            .expect("enroll other");
        assert_eq!(
            store.disposition(&foreign),
            VerifiedTrustDisposition::Unknown
        );
        assert_eq!(
            store.enroll(foreign, "Spoofed"),
            Err(HostTrustError::WrongHostIdentity)
        );
        assert!(!store.revoke(&foreign));
        assert!(!store.revoke_stored(other_store.stored_listeners()[0].id));
        assert_eq!(store.disposition(&own), VerifiedTrustDisposition::Trusted);
        assert_eq!(store.stored_listeners()[0].display_name, "Kitchen");
    }

    #[test]
    fn invalid_enrollment_never_mutates_existing_trust() {
        let (_, host) = key(3);
        let (signing, listener) = key(5);
        let verified = verified(&host, &signing, &listener);
        let mut store = HostVerifiedTrust::new(&host).expect("registry");
        for name in ["", "  ", "bad\nname"] {
            assert_eq!(
                store.enroll(verified, name),
                Err(HostTrustError::InvalidDisplayName)
            );
        }
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        assert_eq!(
            store.disposition(&verified),
            VerifiedTrustDisposition::Unknown
        );
    }
}
