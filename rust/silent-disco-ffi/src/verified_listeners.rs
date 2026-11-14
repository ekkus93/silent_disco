//! The host's remembered ("Always allow") listeners, for the app's settings
//! screen: list them and revoke one. Records are read and written in the app's
//! Rust database (the Android JNI database handle) for one host TLS key; the app
//! never handles a trust record itself.

use silent_disco_core::domain::DeviceId;
use silent_disco_core::storage::DatabaseClient;
use silent_disco_core::transport::secure_identity::{key_bound_device_id, key_bound_fingerprint};
use silent_disco_core::transport::secure_trust::HostVerifiedTrust;

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
/// no trusted record existed (already revoked or never remembered). A host
/// session loads its trust when it starts, so a running session keeps the
/// listener until the next start.
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

fn host_fingerprint(host_public_key_der: &[u8]) -> Result<[u8; 32], FfiBridgeError> {
    HostVerifiedTrust::new(host_public_key_der)
        .map(|trust| *trust.host_fingerprint())
        .map_err(|error| FfiBridgeError::Core(format!("{error:?}")))
}

pub(crate) fn list_verified_listeners(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<Vec<FfiVerifiedListener>, FfiBridgeError> {
    let mut records = client
        .list_verified_listeners(host_fingerprint(host_public_key_der)?)
        .map_err(|error| {
            FfiBridgeError::Core(format!("host trust records unavailable: {error}"))
        })?;
    records.retain(|record| record.revoked_at_ms.is_none());
    records.sort_by_key(|record| record.enrolled_at_ms);
    Ok(records
        .into_iter()
        .map(|record| FfiVerifiedListener {
            device_id: key_bound_device_id(&record.listener_key_fingerprint)
                .as_str()
                .to_owned(),
            display_name: record.display_name,
            enrolled_at_ms: record.enrolled_at_ms,
        })
        .collect())
}

pub(crate) fn revoke_verified_listener(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
    device_id: &str,
    revoked_at_ms: u64,
) -> Result<bool, FfiBridgeError> {
    let host = host_fingerprint(host_public_key_der)?;
    let device_id =
        DeviceId::new(device_id).map_err(|error| FfiBridgeError::Core(format!("{error:?}")))?;
    let listener = key_bound_fingerprint(&device_id).ok_or_else(|| {
        FfiBridgeError::Core("only a remembered secure listener can be revoked".to_owned())
    })?;
    client
        .revoke_verified_listener(host, listener, revoked_at_ms)
        .map_err(|error| FfiBridgeError::Core(format!("could not revoke the listener: {error}")))
}
