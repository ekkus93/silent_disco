from pathlib import Path

path = Path("rust/silent-disco-core/src/storage/legacy_import.rs")
text = path.read_text(encoding="utf-8")
text = text.replace(
    "/// Stable Android SharedPreferences import contract version.",
    "/// Stable Android `SharedPreferences` import contract version.",
)
text = text.replace(
    "/// Stable source identifier recorded in SQLite after a committed import.",
    "/// Stable source identifier recorded in `SQLite` after a committed import.",
)
text = text.replace(
    "/// Typed values read from the pre-Rust Android SharedPreferences store.",
    "/// Typed values read from the pre-Rust Android `SharedPreferences` store.",
)
text = text.replace("let mut worker = DatabaseWorker::start(", "let worker = DatabaseWorker::start(")
text = text.replace(
    "use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};",
    "use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};",
)
start_marker = "pub(crate) fn import_android("
end_marker = "fn corrupt_import_marker"
if text.count(start_marker) != 1 or text.count(end_marker) != 1:
    raise RuntimeError("legacy import function markers did not match exactly")
start = text.index(start_marker)
end = text.index(end_marker)
replacement = r'''pub(crate) fn import_android(
    connection: &mut Connection,
    import: &LegacyAndroidImport,
    schema_version: u32,
) -> Result<LegacyImportOutcome, StorageError> {
    import.validate().map_err(|error| {
        invalid_model(StorageOperation::ImportLegacyData, schema_version, error)
    })?;
    let trusted_device_count = u32::try_from(import.trusted_devices.len()).map_err(|_| {
        invalid_model(
            StorageOperation::ImportLegacyData,
            schema_version,
            "trusted device count exceeds the supported range",
        )
    })?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| map_import_sqlite(schema_version, &error))?;
    if let Some(outcome) = read_existing_outcome(&transaction, import.version, schema_version)? {
        return Ok(outcome);
    }
    if let Some(settings) = &import.settings {
        save_import_settings(&transaction, settings, schema_version)?;
    }
    for device in &import.trusted_devices {
        save_import_device(&transaction, device, schema_version)?;
    }
    record_import_marker(
        &transaction,
        import,
        trusted_device_count,
        schema_version,
    )?;
    transaction
        .commit()
        .map_err(|error| map_import_sqlite(schema_version, &error))?;
    Ok(LegacyImportOutcome {
        disposition: LegacyImportDisposition::Imported,
        import_version: import.version,
        completed_at_ms: import.imported_at_ms,
        settings_imported: import.settings.is_some(),
        trusted_device_count,
    })
}

fn read_existing_outcome(
    transaction: &Transaction<'_>,
    requested_version: u32,
    schema_version: u32,
) -> Result<Option<LegacyImportOutcome>, StorageError> {
    let existing = transaction
        .query_row(
            "SELECT import_version, completed_at_ms, settings_imported, trusted_device_count
             FROM legacy_imports
             WHERE source = ?1",
            [ANDROID_LEGACY_IMPORT_SOURCE],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| map_import_sqlite(schema_version, &error))?;
    existing
        .map(|raw| decode_existing_outcome(raw, requested_version, schema_version))
        .transpose()
}

fn decode_existing_outcome(
    raw: (i64, i64, i64, i64),
    requested_version: u32,
    schema_version: u32,
) -> Result<LegacyImportOutcome, StorageError> {
    let (version, timestamp, settings_imported, device_count) = raw;
    let version = u32::try_from(version).map_err(|_| {
        corrupt_import_marker(schema_version, "stored legacy import version is invalid")
    })?;
    if version != requested_version {
        return Err(StorageError::new(
            StorageErrorKind::Migration,
            StorageOperation::ImportLegacyData,
            format!(
                "legacy Android import marker version {version} does not match requested version {requested_version}"
            ),
            Some(schema_version),
        ));
    }
    let settings_imported = match settings_imported {
        0 => false,
        1 => true,
        _ => {
            return Err(corrupt_import_marker(
                schema_version,
                "stored legacy settings-imported flag is invalid",
            ));
        }
    };
    Ok(LegacyImportOutcome {
        disposition: LegacyImportDisposition::AlreadyCompleted,
        import_version: version,
        completed_at_ms: u64::try_from(timestamp).map_err(|_| {
            corrupt_import_marker(schema_version, "stored legacy import timestamp is invalid")
        })?,
        settings_imported,
        trusted_device_count: u32::try_from(device_count).map_err(|_| {
            corrupt_import_marker(schema_version, "stored legacy device count is invalid")
        })?,
    })
}

fn save_import_settings(
    transaction: &Transaction<'_>,
    settings: &StoredSettings,
    schema_version: u32,
) -> Result<(), StorageError> {
    let operation = StorageOperation::ImportLegacyData;
    let sync_cadence_ms = to_sql_i64(
        settings.tuning.sync_cadence_ms,
        operation,
        schema_version,
        "sync_cadence_ms",
    )?;
    let startup_buffer_ms = to_sql_i64(
        settings.tuning.startup_buffer_ms,
        operation,
        schema_version,
        "startup_buffer_ms",
    )?;
    let late_packet_threshold_ms = to_sql_i64(
        settings.tuning.late_packet_threshold_ms,
        operation,
        schema_version,
        "late_packet_threshold_ms",
    )?;
    let hard_resync_threshold_ms = to_sql_i64(
        settings.tuning.hard_resync_threshold_ms,
        operation,
        schema_version,
        "hard_resync_threshold_ms",
    )?;
    let scan_window_ms = to_sql_i64(
        settings.tuning.scan_window_ms,
        operation,
        schema_version,
        "scan_window_ms",
    )?;
    let updated_at_ms = to_sql_i64(
        settings.updated_at_ms,
        operation,
        schema_version,
        "updated_at_ms",
    )?;
    transaction
        .execute(
            "INSERT INTO app_settings (
                 id, sync_sample_window, sync_cadence_ms, startup_buffer_ms,
                 late_packet_threshold_ms, hard_resync_threshold_ms,
                 sync_drift_threshold_ms, scan_window_ms, updated_at_ms
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
        .map(|_| ())
        .map_err(|error| map_import_sqlite(schema_version, &error))
}

fn save_import_device(
    transaction: &Transaction<'_>,
    device: &TrustedDevice,
    schema_version: u32,
) -> Result<(), StorageError> {
    let operation = StorageOperation::ImportLegacyData;
    let first_seen_ms = to_sql_i64(
        device.first_seen_ms,
        operation,
        schema_version,
        "first_seen_ms",
    )?;
    let last_seen_ms = to_sql_i64(
        device.last_seen_ms,
        operation,
        schema_version,
        "last_seen_ms",
    )?;
    let updated_at_ms = to_sql_i64(
        device.updated_at_ms,
        operation,
        schema_version,
        "updated_at_ms",
    )?;
    transaction
        .execute(
            "INSERT INTO trusted_devices (
                 device_id, display_name, public_key, private_key_ref,
                 trust_state, first_seen_ms, last_seen_ms, updated_at_ms
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
        .map(|_| ())
        .map_err(|error| map_import_sqlite(schema_version, &error))
}

fn record_import_marker(
    transaction: &Transaction<'_>,
    import: &LegacyAndroidImport,
    trusted_device_count: u32,
    schema_version: u32,
) -> Result<(), StorageError> {
    let completed_at_ms = to_sql_i64(
        import.imported_at_ms,
        StorageOperation::ImportLegacyData,
        schema_version,
        "completed_at_ms",
    )?;
    transaction
        .execute(
            "INSERT INTO legacy_imports (
                 source, import_version, completed_at_ms,
                 settings_imported, trusted_device_count
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                ANDROID_LEGACY_IMPORT_SOURCE,
                i64::from(import.version),
                completed_at_ms,
                i64::from(import.settings.is_some()),
                i64::from(trusted_device_count),
            ],
        )
        .map(|_| ())
        .map_err(|error| map_import_sqlite(schema_version, &error))
}

fn map_import_sqlite(schema_version: u32, error: &rusqlite::Error) -> StorageError {
    map_sqlite_error(
        StorageOperation::ImportLegacyData,
        Some(schema_version),
        error,
    )
}

'''
path.write_text(text[:start] + replacement + text[end:], encoding="utf-8")
