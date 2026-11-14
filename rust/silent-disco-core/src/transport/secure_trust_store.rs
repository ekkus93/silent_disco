//! The host's remembered ("Always allow") listeners in the Rust database: load
//! a host's trust registry, enrol a verified listener, list and revoke them,
//! and derive a join's trust state. Shared by every host platform so they all
//! apply one rule: only a key-bound device ID (a key proven over TLS) is ever
//! remembered or trusted, and only for the host key that remembered it.

use std::fmt;

use crate::domain::{DeviceId, TrustState};
use crate::storage::{DatabaseClient, StorageError, VerifiedListenerRecord};
use crate::transport::secure_identity::{key_bound_device_id, key_bound_fingerprint};
use crate::transport::secure_trust::{HostTrustError, HostVerifiedTrust, VerifiedTrustDisposition};

/// Why a trust-record operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStoreError {
    /// The device ID is not key-bound, so the listener never proved a key.
    NotKeyBound,
    /// The device ID itself is malformed.
    InvalidDeviceId(String),
    /// The host key is invalid, or stored records do not fit it.
    Trust(HostTrustError),
    /// The database refused the operation.
    Storage {
        action: &'static str,
        error: StorageError,
    },
}

impl fmt::Display for TrustStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotKeyBound => formatter.write_str(
                "only a listener that proved its key over the secure connection can be remembered or revoked",
            ),
            Self::InvalidDeviceId(reason) => write!(formatter, "invalid listener ID: {reason}"),
            Self::Trust(error) => write!(formatter, "host trust is invalid: {error:?}"),
            Self::Storage { action, error } => write!(formatter, "could not {action}: {error}"),
        }
    }
}

impl std::error::Error for TrustStoreError {}

/// A listener the host currently trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberedListener {
    pub device_id: DeviceId,
    pub display_name: String,
    /// Wall-clock approval time, for display only.
    pub enrolled_at_ms: u64,
}

fn host_fingerprint(host_public_key_der: &[u8]) -> Result<[u8; 32], TrustStoreError> {
    HostVerifiedTrust::new(host_public_key_der)
        .map(|trust| *trust.host_fingerprint())
        .map_err(TrustStoreError::Trust)
}

fn listener_fingerprint(device_id: &DeviceId) -> Result<[u8; 32], TrustStoreError> {
    key_bound_fingerprint(device_id).ok_or(TrustStoreError::NotKeyBound)
}

fn records(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<Vec<VerifiedListenerRecord>, TrustStoreError> {
    client
        .list_verified_listeners(host_fingerprint(host_public_key_der)?)
        .map_err(|error| TrustStoreError::Storage {
            action: "read the trusted listeners",
            error,
        })
}

/// Builds the host's trust registry from its stored records (revoked ones
/// stay revoked).
///
/// # Errors
/// An invalid host key, records that do not belong to it, or a storage failure.
pub fn load_host_trust(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<HostVerifiedTrust, TrustStoreError> {
    let records = records(client, host_public_key_der)?;
    HostVerifiedTrust::from_records(host_public_key_der, &records).map_err(TrustStoreError::Trust)
}

/// Remembers a verified listener for this host. A revoked key that is enrolled
/// again becomes trusted again.
///
/// # Errors
/// A device ID that is not key-bound, an invalid host key, or a storage failure.
pub fn enroll_listener(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
    device_id: &DeviceId,
    display_name: &str,
    enrolled_at_ms: u64,
) -> Result<(), TrustStoreError> {
    let listener_key_fingerprint = listener_fingerprint(device_id)?;
    client
        .enroll_verified_listener(&VerifiedListenerRecord {
            host_key_fingerprint: host_fingerprint(host_public_key_der)?,
            listener_key_fingerprint,
            display_name: display_name.to_owned(),
            enrolled_at_ms,
            revoked_at_ms: None,
        })
        .map_err(|error| TrustStoreError::Storage {
            action: "remember the listener",
            error,
        })
}

/// The listeners this host trusts now, oldest approval first.
///
/// # Errors
/// An invalid host key or a storage failure.
pub fn list_remembered_listeners(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
) -> Result<Vec<RememberedListener>, TrustStoreError> {
    let mut records = records(client, host_public_key_der)?;
    records.retain(|record| record.revoked_at_ms.is_none());
    records.sort_by_key(|record| record.enrolled_at_ms);
    Ok(records
        .into_iter()
        .map(|record| RememberedListener {
            device_id: key_bound_device_id(&record.listener_key_fingerprint),
            display_name: record.display_name,
            enrolled_at_ms: record.enrolled_at_ms,
        })
        .collect())
}

/// Revokes a remembered listener; the record is kept, marked revoked. Returns
/// `false` if this host did not trust it.
///
/// # Errors
/// A device ID that is not key-bound, an invalid host key, or a storage failure.
pub fn revoke_listener(
    client: &DatabaseClient,
    host_public_key_der: &[u8],
    device_id: &DeviceId,
    revoked_at_ms: u64,
) -> Result<bool, TrustStoreError> {
    let listener = listener_fingerprint(device_id)?;
    client
        .revoke_verified_listener(
            host_fingerprint(host_public_key_der)?,
            listener,
            revoked_at_ms,
        )
        .map_err(|error| TrustStoreError::Storage {
            action: "revoke the listener",
            error,
        })
}

/// Parses an app-supplied device ID for [`enroll_listener`] or [`revoke_listener`].
///
/// # Errors
/// A malformed device ID.
pub fn parse_device_id(device_id: &str) -> Result<DeviceId, TrustStoreError> {
    DeviceId::new(device_id).map_err(|error| TrustStoreError::InvalidDeviceId(format!("{error:?}")))
}

/// A join's trust: `Trusted` only for a key-bound device ID that `trust` holds
/// as trusted; every other join is session-only.
#[must_use]
pub fn join_trust_state(trust: Option<&HostVerifiedTrust>, device_id: &DeviceId) -> TrustState {
    let trusted = key_bound_fingerprint(device_id).is_some_and(|fingerprint| {
        trust.is_some_and(|trust| {
            trust.disposition_for_fingerprint(&fingerprint) == VerifiedTrustDisposition::Trusted
        })
    });
    if trusted {
        TrustState::Trusted
    } else {
        TrustState::SessionOnly
    }
}
