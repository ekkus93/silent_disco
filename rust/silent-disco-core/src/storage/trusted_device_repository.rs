use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{DeviceId, TrustState};

use super::{
    error::{StorageError, StorageOperation, map_sqlite_error},
    models::TrustedDevice,
    repository_support::{corrupt_row, from_sql_u64, invalid_model, to_sql_i64},
};

struct RawTrustedDevice {
    device_id: String,
    display_name: String,
    public_key: Option<Vec<u8>>,
    private_key_ref: Option<String>,
    trust_state: String,
    first_seen_ms: i64,
    last_seen_ms: i64,
    updated_at_ms: i64,
}

pub(crate) fn list(
    connection: &Connection,
    schema_version: u32,
) -> Result<Vec<TrustedDevice>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT
                 device_id,
                 display_name,
                 public_key,
                 private_key_ref,
                 trust_state,
                 first_seen_ms,
                 last_seen_ms,
                 updated_at_ms
             FROM trusted_devices
             ORDER BY display_name COLLATE NOCASE ASC, device_id ASC",
        )
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let rows = statement
        .query_map([], read_raw)
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let mut devices = Vec::new();
    for row in rows {
        let raw = row.map_err(|error| {
            map_sqlite_error(StorageOperation::Query, Some(schema_version), &error)
        })?;
        devices.push(decode(raw, schema_version)?);
    }
    Ok(devices)
}

pub(crate) fn get(
    connection: &Connection,
    device_id: &DeviceId,
    schema_version: u32,
) -> Result<Option<TrustedDevice>, StorageError> {
    let raw = connection
        .query_row(
            "SELECT
                 device_id,
                 display_name,
                 public_key,
                 private_key_ref,
                 trust_state,
                 first_seen_ms,
                 last_seen_ms,
                 updated_at_ms
             FROM trusted_devices
             WHERE device_id = ?1",
            [device_id.as_str()],
            read_raw,
        )
        .optional()
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    raw.map(|value| decode(value, schema_version)).transpose()
}

pub(crate) fn upsert(
    connection: &mut Connection,
    device: &TrustedDevice,
    schema_version: u32,
) -> Result<(), StorageError> {
    device
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    let first_seen_ms = to_sql_i64(
        device.first_seen_ms,
        StorageOperation::Transaction,
        schema_version,
        "first_seen_ms",
    )?;
    let last_seen_ms = to_sql_i64(
        device.last_seen_ms,
        StorageOperation::Transaction,
        schema_version,
        "last_seen_ms",
    )?;
    let updated_at_ms = to_sql_i64(
        device.updated_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "updated_at_ms",
    )?;
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    transaction
        .execute(
            "INSERT INTO trusted_devices (
                 device_id,
                 display_name,
                 public_key,
                 private_key_ref,
                 trust_state,
                 first_seen_ms,
                 last_seen_ms,
                 updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(device_id) DO UPDATE SET
                 display_name = excluded.display_name,
                 public_key = excluded.public_key,
                 private_key_ref = excluded.private_key_ref,
                 trust_state = excluded.trust_state,
                 first_seen_ms = MIN(trusted_devices.first_seen_ms, excluded.first_seen_ms),
                 last_seen_ms = excluded.last_seen_ms,
                 updated_at_ms = excluded.updated_at_ms",
            params![
                device.device_id.as_str(),
                device.display_name.as_str(),
                device.public_key.as_deref(),
                device.private_key_ref.as_deref(),
                device.trust_state.wire_name(),
                first_seen_ms,
                last_seen_ms,
                updated_at_ms,
            ],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })
}

pub(crate) fn delete(
    connection: &mut Connection,
    device_id: &DeviceId,
    schema_version: u32,
) -> Result<bool, StorageError> {
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    let affected = transaction
        .execute(
            "DELETE FROM trusted_devices WHERE device_id = ?1",
            [device_id.as_str()],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(affected == 1)
}

fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTrustedDevice> {
    Ok(RawTrustedDevice {
        device_id: row.get(0)?,
        display_name: row.get(1)?,
        public_key: row.get(2)?,
        private_key_ref: row.get(3)?,
        trust_state: row.get(4)?,
        first_seen_ms: row.get(5)?,
        last_seen_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn decode(raw: RawTrustedDevice, schema_version: u32) -> Result<TrustedDevice, StorageError> {
    let device_id = DeviceId::new(raw.device_id).map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored trusted-device identifier is invalid: {error}"),
        )
    })?;
    let trust_state = TrustState::from_wire_name(&raw.trust_state).map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored trusted-device trust state is invalid: {error}"),
        )
    })?;
    let device = TrustedDevice {
        device_id,
        display_name: raw.display_name,
        public_key: raw.public_key,
        private_key_ref: raw.private_key_ref,
        trust_state,
        first_seen_ms: from_sql_u64(
            raw.first_seen_ms,
            StorageOperation::Query,
            schema_version,
            "first_seen_ms",
        )?,
        last_seen_ms: from_sql_u64(
            raw.last_seen_ms,
            StorageOperation::Query,
            schema_version,
            "last_seen_ms",
        )?,
        updated_at_ms: from_sql_u64(
            raw.updated_at_ms,
            StorageOperation::Query,
            schema_version,
            "updated_at_ms",
        )?,
    };
    device.validate().map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored trusted-device row is invalid: {error}"),
        )
    })?;
    Ok(device)
}
