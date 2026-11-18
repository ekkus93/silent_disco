//! The host's remembered ("Always allow") listeners, for the app's settings
//! screen: list them and revoke one. Records are read and written in the app's
//! Rust database (the Android JNI database handle) for one host TLS key; the app
//! never handles a trust record itself.

use silent_disco_core::storage::DatabaseClient;
use silent_disco_core::transport::secure_trust_store;

use crate::FfiBridgeError;

/// One listener this host currently trusts.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiVerifiedListener {
    /// The key-bound device ID (`v4-<fingerprint>`) the listener joins with.
    pub device_id: String,
    pub display_name: String,
    /// Wall-clock time of the "Always allow" approval, for display only.
    pub enrolled_at_ms: u64,
}

/// The listeners `host_public_key_der` currently trusts, oldest approval first.
/// Revoked records are left out.
///
/// # Errors
/// Returns an error for an invalid database handle or host key, or a storage failure.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned parameters at the foreign boundary"
)]
pub fn list_verified_listeners_in_database(
    database_handle: i64,
    host_public_key_der: Vec<u8>,
) -> Result<Vec<FfiVerifiedListener>, FfiBridgeError> {
    list_verified_listeners(&client(database_handle)?, &host_public_key_der)
}

/// Revokes a remembered listener for `host_public_key_der`. Returns `false` if
/// no trusted record existed (already revoked or never remembered). A running
/// host session re-reads its trust at its next join, and a running host with
/// this key disconnects the listener at once if it is connected.
///
/// # Errors
/// Returns an error for a device ID that is not key-bound, an invalid database
/// handle or host key, or a storage failure.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned parameters at the foreign boundary"
)]
pub fn revoke_verified_listener_in_database(
    database_handle: i64,
    host_public_key_der: Vec<u8>,
    device_id: String,
    revoked_at_ms: u64,
) -> Result<bool, FfiBridgeError> {
    revoke_verified_listener(
        &client(database_handle)?,
        &host_public_key_der,
        &device_id,
        revoked_at_ms,
    )
}

fn client(database_handle: i64) -> Result<DatabaseClient, FfiBridgeError> {
    crate::android_database_abi::database_client(database_handle).map_err(|status| {
        FfiBridgeError::Core(format!("host trust database unavailable: {status}"))
    })
}

pub(crate) fn list_verified_listeners(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<Vec<FfiVerifiedListener>, FfiBridgeError> {
    Ok(
        secure_trust_store::list_remembered_listeners(client, host_public_key_der)
            .map_err(core_error)?
            .into_iter()
            .map(|listener| FfiVerifiedListener {
                device_id: listener.device_id.as_str().to_owned(),
                display_name: listener.display_name,
                enrolled_at_ms: listener.enrolled_at_ms,
            })
            .collect(),
    )
}

pub(crate) fn revoke_verified_listener(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
    device_id: &str,
    revoked_at_ms: u64,
) -> Result<bool, FfiBridgeError> {
    let device_id = secure_trust_store::parse_device_id(device_id).map_err(core_error)?;
    let revoked =
        secure_trust_store::revoke_listener(client, host_public_key_der, &device_id, revoked_at_ms)
            .map_err(core_error)?;
    crate::host_control::trust_records_changed();
    if revoked {
        // Disconnect it now from any running host with this key, not only at
        // its next join (owner decision 2026-11-18).
        crate::host_control::listener_revoked(host_public_key_der, &device_id)?;
    }
    Ok(revoked)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err passes the error by value"
)]
fn core_error(error: secure_trust_store::TrustStoreError) -> FfiBridgeError {
    FfiBridgeError::Core(error.to_string())
}
