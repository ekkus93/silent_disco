//! The desktop host's remembered ("Always allow") listeners for this profile's
//! host TLS key: the registry joins are checked against, plus enrol, list and
//! revoke. Every change is written to the profile's Rust database first and the
//! registry reloaded from it, so a revoke takes effect for the next join, even
//! during a running session.

use silent_disco_core::domain::{DeviceId, TrustState};
use silent_disco_core::storage::DatabaseClient;
use silent_disco_core::transport::secure_trust::HostVerifiedTrust;
use silent_disco_core::transport::secure_trust_store::{self, RememberedListener, TrustStoreError};
use std::sync::Mutex;

pub(crate) struct DesktopVerifiedTrust {
    host_public_key_der: Vec<u8>,
    database: DatabaseClient,
    registry: Mutex<HostVerifiedTrust>,
}

impl DesktopVerifiedTrust {
    /// Loads the records for `host_public_key_der` from `database`.
    pub(crate) fn load(
        database: DatabaseClient,
        host_public_key_der: Vec<u8>,
    ) -> Result<Self, TrustStoreError> {
        let registry = secure_trust_store::load_host_trust(&database, &host_public_key_der)?;
        Ok(Self {
            host_public_key_der,
            database,
            registry: Mutex::new(registry),
        })
    }

    /// `Trusted` only for a key-bound ID this host remembered and has not revoked.
    /// A poisoned registry trusts nobody.
    pub(crate) fn join_trust_state(&self, device_id: &DeviceId) -> TrustState {
        self.registry
            .lock()
            .map_or(TrustState::SessionOnly, |registry| {
                secure_trust_store::join_trust_state(Some(&registry), device_id)
            })
    }

    /// Remembers a verified listener, then reloads the registry.
    pub(crate) fn remember(
        &self,
        device_id: &DeviceId,
        display_name: &str,
        enrolled_at_ms: u64,
    ) -> Result<(), TrustStoreError> {
        secure_trust_store::enroll_listener(
            &self.database,
            &self.host_public_key_der,
            device_id,
            display_name,
            enrolled_at_ms,
        )?;
        self.reload()
    }

    pub(crate) fn list(&self) -> Result<Vec<RememberedListener>, TrustStoreError> {
        secure_trust_store::list_remembered_listeners(&self.database, &self.host_public_key_der)
    }

    /// Revokes a remembered listener, then reloads the registry. Returns
    /// `false` if this host did not trust it.
    pub(crate) fn revoke(
        &self,
        device_id: &DeviceId,
        revoked_at_ms: u64,
    ) -> Result<bool, TrustStoreError> {
        let revoked = secure_trust_store::revoke_listener(
            &self.database,
            &self.host_public_key_der,
            device_id,
            revoked_at_ms,
        )?;
        self.reload()?;
        Ok(revoked)
    }

    fn reload(&self) -> Result<(), TrustStoreError> {
        let registry =
            secure_trust_store::load_host_trust(&self.database, &self.host_public_key_der)?;
        // A poisoned lock is replaced with the fresh registry: the database,
        // not the old in-memory copy, is authoritative.
        let mut guard = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = registry;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopVerifiedTrust;
    use silent_disco_core::domain::{DeviceId, TrustState};
    use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
    use silent_disco_core::transport::secure_identity::key_bound_device_id;

    fn spki(seed: u8) -> Vec<u8> {
        use p256::{ecdsa::SigningKey, pkcs8::EncodePublicKey};
        SigningKey::from_slice(&[seed; 32])
            .expect("fixture key")
            .verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn remember_and_revoke_change_the_next_joins_trust_and_survive_reopen() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("domain.sqlite3");
        let worker = DatabaseWorker::start(DatabaseConfig::new(&path).expect("config"))
            .expect("database worker");
        let listener = key_bound_device_id(&[0xa1; 32]);
        let trust = DesktopVerifiedTrust::load(worker.client(), spki(3)).expect("load");
        assert_eq!(trust.join_trust_state(&listener), TrustState::SessionOnly);

        trust.remember(&listener, "Alex", 10).expect("remember");
        assert_eq!(trust.join_trust_state(&listener), TrustState::Trusted);
        assert_eq!(trust.list().expect("list").len(), 1);
        let claimed = DeviceId::new("claimed-listener").expect("id");
        assert!(trust.remember(&claimed, "Claimed", 11).is_err());
        assert_eq!(trust.join_trust_state(&claimed), TrustState::SessionOnly);

        let reopened = DesktopVerifiedTrust::load(worker.client(), spki(3)).expect("reopen");
        assert_eq!(reopened.join_trust_state(&listener), TrustState::Trusted);
        let other_host = DesktopVerifiedTrust::load(worker.client(), spki(4)).expect("other");
        assert_eq!(
            other_host.join_trust_state(&listener),
            TrustState::SessionOnly
        );

        assert!(trust.revoke(&listener, 20).expect("revoke"));
        assert_eq!(
            trust.join_trust_state(&listener),
            TrustState::SessionOnly,
            "a revoke reaches the live registry at once"
        );
        assert!(trust.list().expect("list").is_empty());
        assert!(!trust.revoke(&listener, 21).expect("again"));
        drop((trust, reopened, other_host));
        worker.stop_and_join().expect("close");
    }
}
