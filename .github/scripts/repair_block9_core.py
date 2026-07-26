from pathlib import Path

path = Path("rust/silent-disco-core/src/storage/legacy_import_repository.rs")
text = path.read_text()
text = text.replace(
    "use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};",
    "use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};",
)
text = text.replace(
    "        LEGACY_ANDROID_IMPORT_SOURCE, LegacyAndroidImport, LegacyImportOutcome,\n",
    "        LEGACY_ANDROID_IMPORT_SOURCE, LegacyAndroidImport, LegacyImportOutcome, StoredSettings,\n        TrustedDevice,\n",
)
start = text.index("pub(crate) fn import(")
end = text.index("fn invalid_range", start)
replacement = r'''pub(crate) fn import(
    connection: &mut Connection,
    value: &LegacyAndroidImport,
    schema_version: u32,
) -> Result<LegacyImportOutcome, StorageError> {
    value
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    if let Some(existing) = existing_import_version(&transaction, schema_version)? {
        return finish_existing_import(transaction, existing, value.version, schema_version);
    }
    write_settings(&transaction, &value.settings, schema_version)?;
    for device in &value.trusted_devices {
        write_trusted_device(&transaction, device, schema_version)?;
    }
    write_import_marker(&transaction, value, schema_version)?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(LegacyImportOutcome::Imported)
}

fn existing_import_version(
    transaction: &Transaction<'_>,
    schema_version: u32,
) -> Result<Option<i64>, StorageError> {
    transaction
        .query_row(
            "SELECT import_version FROM legacy_imports WHERE source = ?1",
            [LEGACY_ANDROID_IMPORT_SOURCE],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Query, Some(schema_version), &error)
        })
}

fn finish_existing_import(
    transaction: Transaction<'_>,
    existing: i64,
    requested: u32,
    schema_version: u32,
) -> Result<LegacyImportOutcome, StorageError> {
    let expected = i64::from(requested);
    if existing != expected {
        return Err(StorageError::new(
            StorageErrorKind::Corruption,
            StorageOperation::Transaction,
            format!(
                "legacy Android import version {existing} does not match supported version {expected}"
            ),
            Some(schema_version),
        ));
    }
    transaction.rollback().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(LegacyImportOutcome::AlreadyImported)
}

fn write_settings(
    transaction: &Transaction<'_>,
    settings: &StoredSettings,
    schema_version: u32,
) -> Result<(), StorageError> {
    let tuning = &settings.tuning;
    let updated_at_ms = i64::try_from(settings.updated_at_ms)
        .map_err(|_| invalid_range(schema_version))?;
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
                i64::from(tuning.sync_sample_window),
                i64::try_from(tuning.sync_cadence_ms).map_err(|_| invalid_range(schema_version))?,
                i64::try_from(tuning.startup_buffer_ms).map_err(|_| invalid_range(schema_version))?,
                i64::try_from(tuning.late_packet_threshold_ms)
                    .map_err(|_| invalid_range(schema_version))?,
                i64::try_from(tuning.hard_resync_threshold_ms)
                    .map_err(|_| invalid_range(schema_version))?,
                tuning.sync_drift_threshold_ms,
                i64::try_from(tuning.scan_window_ms).map_err(|_| invalid_range(schema_version))?,
                updated_at_ms,
            ],
        )
        .map(|_| ())
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })
}

fn write_trusted_device(
    transaction: &Transaction<'_>,
    device: &TrustedDevice,
    schema_version: u32,
) -> Result<(), StorageError> {
    let first_seen_ms = i64::try_from(device.first_seen_ms)
        .map_err(|_| invalid_range(schema_version))?;
    let last_seen_ms = i64::try_from(device.last_seen_ms)
        .map_err(|_| invalid_range(schema_version))?;
    let updated_at_ms = i64::try_from(device.updated_at_ms)
        .map_err(|_| invalid_range(schema_version))?;
    transaction
        .execute(
            "INSERT INTO trusted_devices (
                 device_id, display_name, public_key, private_key_ref, trust_state,
                 first_seen_ms, last_seen_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(device_id) DO UPDATE SET
                 display_name = excluded.display_name,
                 public_key = excluded.public_key,
                 private_key_ref = excluded.private_key_ref,
                 trust_state = excluded.trust_state,
                 last_seen_ms = MAX(trusted_devices.last_seen_ms, excluded.last_seen_ms),
                 updated_at_ms = MAX(trusted_devices.updated_at_ms, excluded.updated_at_ms)",
            params![
                device.device_id.as_str(),
                device.display_name,
                device.public_key,
                device.private_key_ref,
                device.trust_state.wire_name(),
                first_seen_ms,
                last_seen_ms,
                updated_at_ms,
            ],
        )
        .map(|_| ())
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })
}

fn write_import_marker(
    transaction: &Transaction<'_>,
    value: &LegacyAndroidImport,
    schema_version: u32,
) -> Result<(), StorageError> {
    let imported_at_ms = i64::try_from(value.imported_at_ms)
        .map_err(|_| invalid_range(schema_version))?;
    transaction
        .execute(
            "INSERT INTO legacy_imports(source, import_version, imported_at_ms)
             VALUES (?1, ?2, ?3)",
            params![
                LEGACY_ANDROID_IMPORT_SOURCE,
                i64::from(value.version),
                imported_at_ms,
            ],
        )
        .map(|_| ())
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })
}

'''
text = text[:start] + replacement + text[end:]
text = text.replace("DatabaseConfig::new(&path.path)", "DatabaseConfig::new(path.path())")
if "&path.path" in text:
    raise SystemExit("private TestDatabasePath field reference remains")
path.write_text(text)
