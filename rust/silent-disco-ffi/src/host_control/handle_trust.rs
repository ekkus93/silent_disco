//! Host-owned listener trust for the FFI host. A join's trust state is derived
//! here from the host's own records; the app never supplies it.

use silent_disco_core::domain::{DeviceId, TrustState};
use silent_disco_core::transport::secure_identity::key_bound_fingerprint;
use silent_disco_core::transport::secure_trust::{HostVerifiedTrust, VerifiedTrustDisposition};

use super::handle::FfiCoreHandle;
use super::types::FfiBridgeError;

#[allow(
    clippy::missing_errors_doc,
    reason = "exported method uses the uniform validated FfiBridgeError contract"
)]
#[uniffi::export]
impl FfiCoreHandle {
    /// Loads the host's trusted-listener records for this host identity. Trust is
    /// read only from these records; a join never supplies its own trust state.
    ///
    /// # Errors
    /// Returns an error if the host key or the snapshot is invalid, or the handle is closed.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned parameters at the foreign boundary"
    )]
    pub fn set_host_trust_snapshot(
        &self,
        host_public_key_der: Vec<u8>,
        snapshot_json: String,
    ) -> Result<(), FfiBridgeError> {
        let trust = HostVerifiedTrust::from_json(&host_public_key_der, &snapshot_json)
            .map_err(|error| FfiBridgeError::Core(format!("{error:?}")))?;
        let mut guard = self
            .host_trust
            .lock()
            .map_err(|_| FfiBridgeError::Closed("host trust is poisoned".to_owned()))?;
        *guard = Some(trust);
        Ok(())
    }
}

#[allow(
    clippy::missing_errors_doc,
    reason = "exported method uses the uniform validated FfiBridgeError contract"
)]
#[uniffi::export]
impl FfiCoreHandle {
    /// Loads this host's trusted-listener records from the app's Rust database
    /// (the Android JNI database handle) for the host key the session presents.
    /// The records never pass through the app. Returns how many were loaded,
    /// revoked ones included.
    ///
    /// # Errors
    /// Returns an error for an invalid database handle, a storage failure, an
    /// invalid host key or stored records that do not belong to it.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned parameters at the foreign boundary"
    )]
    pub fn load_host_trust_from_database(
        &self,
        database_handle: i64,
        host_public_key_der: Vec<u8>,
    ) -> Result<u32, FfiBridgeError> {
        let client =
            crate::android_database_abi::database_client(database_handle).map_err(|status| {
                FfiBridgeError::Core(format!("host trust database unavailable: {status:?}"))
            })?;
        load_host_trust(self, &client, &host_public_key_der)
    }
}

/// Reads the records for `host_public_key_der` and installs them as this
/// handle's trust registry.
pub(super) fn load_host_trust(
    handle: &FfiCoreHandle,
    client: &silent_disco_core::storage::DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<u32, FfiBridgeError> {
    let empty = HostVerifiedTrust::new(host_public_key_der)
        .map_err(|error| FfiBridgeError::Core(format!("{error:?}")))?;
    let records = client
        .list_verified_listeners(*empty.host_fingerprint())
        .map_err(|error| {
            FfiBridgeError::Core(format!("host trust records unavailable: {error}"))
        })?;
    let trust = HostVerifiedTrust::from_records(host_public_key_der, &records)
        .map_err(|error| FfiBridgeError::Core(format!("{error:?}")))?;
    let count = u32::try_from(trust.len()).unwrap_or(u32::MAX);
    *handle
        .host_trust
        .lock()
        .map_err(|_| FfiBridgeError::Closed("host trust is poisoned".to_owned()))? = Some(trust);
    Ok(count)
}

/// Trust for a join. Only a key-bound device ID can be trusted, and only when the
/// host's own records hold that key as trusted. Every other join is session-only.
pub(super) fn join_trust_state(
    handle: &FfiCoreHandle,
    device_id: &DeviceId,
) -> Result<TrustState, FfiBridgeError> {
    let Some(fingerprint) = key_bound_fingerprint(device_id) else {
        return Ok(TrustState::SessionOnly);
    };
    let guard = handle
        .host_trust
        .lock()
        .map_err(|_| FfiBridgeError::Closed("host trust is poisoned".to_owned()))?;
    let trusted = guard.as_ref().is_some_and(|trust| {
        trust.disposition_for_fingerprint(&fingerprint) == VerifiedTrustDisposition::Trusted
    });
    Ok(if trusted {
        TrustState::Trusted
    } else {
        TrustState::SessionOnly
    })
}
