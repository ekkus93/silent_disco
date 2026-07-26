use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::TuningSettings;

use super::{
    error::{StorageError, StorageOperation, map_sqlite_error},
    models::StoredSettings,
    repository_support::{corrupt_row, from_sql_u16, from_sql_u64, invalid_model, to_sql_i64},
};

#[derive(Clone, Copy)]
struct RawSettings {
    sync_sample_window: i64,
    sync_cadence_ms: i64,
    startup_buffer_ms: i64,
    late_packet_threshold_ms: i64,
    hard_resync_threshold_ms: i64,
    sync_drift_threshold_ms: f64,
    scan_window_ms: i64,
    updated_at_ms: i64,
}

pub(crate) fn load(
    connection: &Connection,
    schema_version: u32,
) -> Result<Option<StoredSettings>, StorageError> {
    let raw = connection
        .query_row(
            "SELECT
                 sync_sample_window,
                 sync_cadence_ms,
                 startup_buffer_ms,
                 late_packet_threshold_ms,
                 hard_resync_threshold_ms,
                 sync_drift_threshold_ms,
                 scan_window_ms,
                 updated_at_ms
             FROM app_settings
             WHERE id = 1",
            [],
            |row| {
                Ok(RawSettings {
                    sync_sample_window: row.get(0)?,
                    sync_cadence_ms: row.get(1)?,
                    startup_buffer_ms: row.get(2)?,
                    late_packet_threshold_ms: row.get(3)?,
                    hard_resync_threshold_ms: row.get(4)?,
                    sync_drift_threshold_ms: row.get(5)?,
                    scan_window_ms: row.get(6)?,
                    updated_at_ms: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    raw.map(|value| decode(value, schema_version)).transpose()
}

pub(crate) fn save(
    connection: &mut Connection,
    settings: &StoredSettings,
    schema_version: u32,
) -> Result<(), StorageError> {
    settings
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    let sync_cadence_ms = to_sql_i64(
        settings.tuning.sync_cadence_ms,
        StorageOperation::Transaction,
        schema_version,
        "sync_cadence_ms",
    )?;
    let startup_buffer_ms = to_sql_i64(
        settings.tuning.startup_buffer_ms,
        StorageOperation::Transaction,
        schema_version,
        "startup_buffer_ms",
    )?;
    let late_packet_threshold_ms = to_sql_i64(
        settings.tuning.late_packet_threshold_ms,
        StorageOperation::Transaction,
        schema_version,
        "late_packet_threshold_ms",
    )?;
    let hard_resync_threshold_ms = to_sql_i64(
        settings.tuning.hard_resync_threshold_ms,
        StorageOperation::Transaction,
        schema_version,
        "hard_resync_threshold_ms",
    )?;
    let scan_window_ms = to_sql_i64(
        settings.tuning.scan_window_ms,
        StorageOperation::Transaction,
        schema_version,
        "scan_window_ms",
    )?;
    let updated_at_ms = to_sql_i64(
        settings.updated_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "updated_at_ms",
    )?;
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    transaction
        .execute(
            "INSERT INTO app_settings (
                 id,
                 sync_sample_window,
                 sync_cadence_ms,
                 startup_buffer_ms,
                 late_packet_threshold_ms,
                 hard_resync_threshold_ms,
                 sync_drift_threshold_ms,
                 scan_window_ms,
                 updated_at_ms
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 sync_sample_window = excluded.sync_sample_window,
                 sync_cadence_ms = excluded.sync_cadence_ms,
                 startup_buffer_ms = excluded.startup_buffer_ms,
                 late_packet_threshold_ms = excluded.late_packet_threshold_ms,
                 hard_resync_threshold_ms = excluded.hard_resync_threshold_ms,
                 sync_drift_threshold_ms = excluded.sync_drift_threshold_ms,
                 scan_window_ms = excluded.scan_window_ms,
                 updated_at_ms = excluded.updated_at_ms",
            params![
                i64::from(settings.tuning.sync_sample_window),
                sync_cadence_ms,
                startup_buffer_ms,
                late_packet_threshold_ms,
                hard_resync_threshold_ms,
                settings.tuning.sync_drift_threshold_ms,
                scan_window_ms,
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

fn decode(raw: RawSettings, schema_version: u32) -> Result<StoredSettings, StorageError> {
    let settings = StoredSettings {
        tuning: TuningSettings {
            sync_sample_window: from_sql_u16(
                raw.sync_sample_window,
                StorageOperation::Query,
                schema_version,
                "sync_sample_window",
            )?,
            sync_cadence_ms: from_sql_u64(
                raw.sync_cadence_ms,
                StorageOperation::Query,
                schema_version,
                "sync_cadence_ms",
            )?,
            startup_buffer_ms: from_sql_u64(
                raw.startup_buffer_ms,
                StorageOperation::Query,
                schema_version,
                "startup_buffer_ms",
            )?,
            late_packet_threshold_ms: from_sql_u64(
                raw.late_packet_threshold_ms,
                StorageOperation::Query,
                schema_version,
                "late_packet_threshold_ms",
            )?,
            hard_resync_threshold_ms: from_sql_u64(
                raw.hard_resync_threshold_ms,
                StorageOperation::Query,
                schema_version,
                "hard_resync_threshold_ms",
            )?,
            sync_drift_threshold_ms: raw.sync_drift_threshold_ms,
            scan_window_ms: from_sql_u64(
                raw.scan_window_ms,
                StorageOperation::Query,
                schema_version,
                "scan_window_ms",
            )?,
        },
        updated_at_ms: from_sql_u64(
            raw.updated_at_ms,
            StorageOperation::Query,
            schema_version,
            "updated_at_ms",
        )?,
    };
    settings.validate().map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored app settings are invalid: {error}"),
        )
    })?;
    Ok(settings)
}
