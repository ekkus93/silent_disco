//! Host-owned listener trust for the FFI host. A join's trust state is derived
//! here from the host's own records; the app never supplies it.

use silent_disco_core::domain::{DeviceId, TrustState};
use silent_disco_core::transport::secure_trust::HostVerifiedTrust;
use silent_disco_core::transport::secure_trust_store;

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

#[allow(
    clippy::missing_errors_doc,
    reason = "exported method uses the uniform validated FfiBridgeError contract"
)]
#[uniffi::export]
impl FfiCoreHandle {
    /// Stores an "Always allow" approval for a verified listener in the app's
    /// Rust database and reloads this host's trust from it. Only a key-bound
    /// device ID (a key proven over the secure channel) can be enrolled; a
    /// claimed legacy ID is refused. The caller then reports the storage
    /// effect's completion to the actor.
    ///
    /// # Errors
    /// Returns an error for a device ID that is not key-bound, an invalid
    /// database handle or host key, an invalid name, or a storage failure.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned parameters at the foreign boundary"
    )]
    pub fn enroll_verified_listener_in_database(
        &self,
        database_handle: i64,
        host_public_key_der: Vec<u8>,
        device_id: String,
        display_name: String,
        enrolled_at_ms: u64,
    ) -> Result<(), FfiBridgeError> {
        let client =
            crate::android_database_abi::database_client(database_handle).map_err(|status| {
                FfiBridgeError::Core(format!("host trust database unavailable: {status}"))
            })?;
        enroll_verified_listener(
            self,
            &client,
            &host_public_key_der,
            &device_id,
            &display_name,
            enrolled_at_ms,
        )
    }
}

/// Writes the enrolment for a key-bound `device_id`, then reloads the trust
/// registry so the next join from that key is trusted.
pub(super) fn enroll_verified_listener(
    handle: &FfiCoreHandle,
    client: &silent_disco_core::storage::DatabaseClient,
    host_public_key_der: &[u8],
    device_id: &str,
    display_name: &str,
    enrolled_at_ms: u64,
) -> Result<(), FfiBridgeError> {
    let device_id = secure_trust_store::parse_device_id(device_id).map_err(core_error)?;
    secure_trust_store::enroll_listener(
        client,
        host_public_key_der,
        &device_id,
        display_name,
        enrolled_at_ms,
    )
    .map_err(core_error)?;
    load_host_trust(handle, client, host_public_key_der).map(|_| ())
}

/// Reads the records for `host_public_key_der` and installs them as this
/// handle's trust registry.
pub(super) fn load_host_trust(
    handle: &FfiCoreHandle,
    client: &silent_disco_core::storage::DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<u32, FfiBridgeError> {
    let trust =
        secure_trust_store::load_host_trust(client, host_public_key_der).map_err(core_error)?;
    let count = u32::try_from(trust.len()).unwrap_or(u32::MAX);
    *handle
        .host_trust
        .lock()
        .map_err(|_| FfiBridgeError::Closed("host trust is poisoned".to_owned()))? = Some(trust);
    Ok(count)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err passes the error by value"
)]
fn core_error(error: secure_trust_store::TrustStoreError) -> FfiBridgeError {
    FfiBridgeError::Core(error.to_string())
}

/// Trust for a join. Only a key-bound device ID can be trusted, and only when the
/// host's own records hold that key as trusted. Every other join is session-only.
pub(super) fn join_trust_state(
    handle: &FfiCoreHandle,
    device_id: &DeviceId,
) -> Result<TrustState, FfiBridgeError> {
    let guard = handle
        .host_trust
        .lock()
        .map_err(|_| FfiBridgeError::Closed("host trust is poisoned".to_owned()))?;
    Ok(secure_trust_store::join_trust_state(
        guard.as_ref(),
        device_id,
    ))
}
