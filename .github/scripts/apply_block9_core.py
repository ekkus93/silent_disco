from __future__ import annotations

from pathlib import Path


def replace_once(path: str, old: str, new: str, label: str) -> None:
    file = Path(path)
    text = file.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    file.write_text(text.replace(old, new))


MIGRATION_V2 = r'''
const MIGRATION_V2_SQL: &str = r"
CREATE TABLE legacy_imports (
    source          TEXT PRIMARY KEY CHECK (source = 'android_shared_preferences'),
    import_version  INTEGER NOT NULL CHECK (import_version > 0),
    imported_at_ms  INTEGER NOT NULL CHECK (imported_at_ms >= 0)
) STRICT;
";

'''

MODELS = r'''
/// Current version of the typed Android `SharedPreferences` import contract.
pub const LEGACY_ANDROID_IMPORT_VERSION: u32 = 1;

/// Stable source identifier recorded after the one-time Android import commits.
pub const LEGACY_ANDROID_IMPORT_SOURCE: &str = "android_shared_preferences";

/// Typed legacy Android values accepted by the Rust database exactly once.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyAndroidImport {
    pub version: u32,
    pub settings: StoredSettings,
    pub trusted_devices: Vec<TrustedDevice>,
    pub imported_at_ms: u64,
}

impl LegacyAndroidImport {
    /// Validates the import before any transaction begins.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation error for an unsupported version,
    /// invalid settings/device data, duplicate device IDs, or invalid timestamp.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        if self.version != LEGACY_ANDROID_IMPORT_VERSION {
            return Err(StorageModelValidationError::LegacyImportVersion);
        }
        self.settings.validate()?;
        validate_sql_millis(self.imported_at_ms, StorageModelValidationError::Timestamp)?;
        let mut ids = std::collections::BTreeSet::new();
        for device in &self.trusted_devices {
            device.validate()?;
            if !ids.insert(device.device_id.as_str()) {
                return Err(StorageModelValidationError::DuplicateLegacyDevice);
            }
        }
        Ok(())
    }
}

/// Result of the idempotent one-time Android import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyImportOutcome {
    Imported,
    AlreadyImported,
}

'''

LEGACY_REPOSITORY = r'''use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{
    error::{StorageError, StorageErrorKind, StorageOperation, map_sqlite_error},
    models::{
        LEGACY_ANDROID_IMPORT_SOURCE, LegacyAndroidImport, LegacyImportOutcome,
    },
    repository_support::{invalid_model, to_sql_i64},
};

pub(crate) fn import(
    connection: &mut Connection,
    value: &LegacyAndroidImport,
    schema_version: u32,
) -> Result<LegacyImportOutcome, StorageError> {
    value
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    let imported_at_ms = to_sql_i64(
        value.imported_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "legacy_import.imported_at_ms",
    )?;
    let settings_updated_at_ms = to_sql_i64(
        value.settings.updated_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "legacy_import.settings.updated_at_ms",
    )?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    let existing = transaction
        .query_row(
            "SELECT import_version FROM legacy_imports WHERE source = ?1",
            [LEGACY_ANDROID_IMPORT_SOURCE],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Query, Some(schema_version), &error)
        })?;
    if let Some(existing) = existing {
        let expected = i64::from(value.version);
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
        return Ok(LegacyImportOutcome::AlreadyImported);
    }

    let tuning = &value.settings.tuning;
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
                i64::try_from(tuning.late_packet_threshold_ms).map_err(|_| invalid_range(schema_version))?,
                i64::try_from(tuning.hard_resync_threshold_ms).map_err(|_| invalid_range(schema_version))?,
                tuning.sync_drift_threshold_ms,
                i64::try_from(tuning.scan_window_ms).map_err(|_| invalid_range(schema_version))?,
                settings_updated_at_ms,
            ],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;

    for device in &value.trusted_devices {
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
            .map_err(|error| {
                map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
            })?;
    }

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
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(LegacyImportOutcome::Imported)
}

fn invalid_range(schema_version: u32) -> StorageError {
    StorageError::new(
        StorageErrorKind::InvalidConfiguration,
        StorageOperation::Transaction,
        "legacy Android import contains a timestamp or tuning value outside SQLite range",
        Some(schema_version),
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{DeviceId, TrustState, TuningSettings},
        storage::{
            DatabaseConfig, DatabaseWorker, LegacyAndroidImport, LegacyImportOutcome,
            StoredSettings, TrustedDevice, LEGACY_ANDROID_IMPORT_VERSION,
            test_support::TestDatabasePath,
        },
    };

    fn import_value(now: u64) -> LegacyAndroidImport {
        LegacyAndroidImport {
            version: LEGACY_ANDROID_IMPORT_VERSION,
            settings: StoredSettings {
                tuning: TuningSettings::default(),
                updated_at_ms: now,
            },
            trusted_devices: vec![TrustedDevice {
                device_id: DeviceId::new("legacy-listener").expect("valid id"),
                display_name: "Legacy Listener".into(),
                public_key: None,
                private_key_ref: None,
                trust_state: TrustState::Trusted,
                first_seen_ms: now,
                last_seen_ms: now,
                updated_at_ms: now,
            }],
            imported_at_ms: now,
        }
    }

    #[test]
    fn import_is_transactional_and_idempotent() {
        let path = TestDatabasePath::new("legacy-import");
        let worker = DatabaseWorker::start(DatabaseConfig::new(&path.path).expect("config"))
            .expect("worker");
        let client = worker.client();
        let value = import_value(10);
        assert_eq!(
            client.import_legacy_android(&value),
            Ok(LegacyImportOutcome::Imported)
        );
        assert_eq!(
            client.import_legacy_android(&value),
            Ok(LegacyImportOutcome::AlreadyImported)
        );
        assert_eq!(client.load_settings(), Ok(Some(value.settings)));
        assert!(client
            .get_trusted_device(&DeviceId::new("legacy-listener").expect("id"))
            .expect("query")
            .is_some());
        worker.stop_and_join().expect("close");
    }

    #[test]
    fn invalid_import_leaves_no_marker_or_partial_rows() {
        let path = TestDatabasePath::new("legacy-import-invalid");
        let worker = DatabaseWorker::start(DatabaseConfig::new(&path.path).expect("config"))
            .expect("worker");
        let client = worker.client();
        let mut value = import_value(20);
        value.settings.tuning.sync_sample_window = 0;
        assert!(client.import_legacy_android(&value).is_err());
        assert_eq!(client.load_settings(), Ok(None));
        assert!(client.list_trusted_devices().expect("query").is_empty());
        worker.stop_and_join().expect("close");
    }
}
'''


def main() -> None:
    replace_once(
        "rust/silent-disco-core/src/storage/migrations.rs",
        "pub const LATEST_SCHEMA_VERSION: u32 = 1;",
        "pub const LATEST_SCHEMA_VERSION: u32 = 2;",
        "latest schema version",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/migrations.rs",
        "#[derive(Debug, Clone, Copy)]\nstruct Migration {",
        MIGRATION_V2 + "#[derive(Debug, Clone, Copy)]\nstruct Migration {",
        "migration v2 insertion",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/migrations.rs",
        "const MIGRATIONS: &[Migration] = &[Migration {\n    version: 1,\n    sql: MIGRATION_V1_SQL,\n}];",
        "const MIGRATIONS: &[Migration] = &[\n    Migration {\n        version: 1,\n        sql: MIGRATION_V1_SQL,\n    },\n    Migration {\n        version: 2,\n        sql: MIGRATION_V2_SQL,\n    },\n];",
        "migration catalog",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/models.rs",
        "/// Persisted trusted-device metadata. Private key bytes are never stored here.\n",
        MODELS + "/// Persisted trusted-device metadata. Private key bytes are never stored here.\n",
        "legacy import models",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/models.rs",
        "    DiagnosticExportLimit,\n}",
        "    DiagnosticExportLimit,\n    LegacyImportVersion,\n    DuplicateLegacyDevice,\n}",
        "model error variants",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/models.rs",
        "            Self::DiagnosticExportLimit => {\n                \"diagnostic export page limit is outside the supported range\"\n            }\n",
        "            Self::DiagnosticExportLimit => {\n                \"diagnostic export page limit is outside the supported range\"\n            }\n            Self::LegacyImportVersion => \"legacy Android import version is unsupported\",\n            Self::DuplicateLegacyDevice => \"legacy Android import contains a duplicate device ID\",\n",
        "model error display",
    )
    Path("rust/silent-disco-core/src/storage/legacy_import_repository.rs").write_text(LEGACY_REPOSITORY)
    replace_once(
        "rust/silent-disco-core/src/storage/mod.rs",
        "mod migrations;\n",
        "mod migrations;\nmod legacy_import_repository;\n",
        "legacy repository module",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/mod.rs",
        "    SessionUpdate, StorageModelValidationError, StoredSettings, TrustedDevice,\n",
        "    LegacyAndroidImport, LegacyImportOutcome, LEGACY_ANDROID_IMPORT_SOURCE,\n    LEGACY_ANDROID_IMPORT_VERSION, SessionUpdate, StorageModelValidationError, StoredSettings,\n    TrustedDevice,\n",
        "legacy model exports",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/database.rs",
        "    diagnostics_repository,\n",
        "    diagnostics_repository, legacy_import_repository,\n",
        "database legacy import module",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/database.rs",
        "        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,\n        SessionEnd, SessionHistory, SessionStart, SessionUpdate, StoredSettings, TrustedDevice,\n",
        "        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,\n        LegacyAndroidImport, LegacyImportOutcome, SessionEnd, SessionHistory, SessionStart,\n        SessionUpdate, StoredSettings, TrustedDevice,\n",
        "database legacy types",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/database.rs",
        "    pub(crate) fn load_settings(&self) -> Result<Option<StoredSettings>, StorageError> {",
        "    pub(crate) fn import_legacy_android(\n        &mut self,\n        value: &LegacyAndroidImport,\n    ) -> Result<LegacyImportOutcome, StorageError> {\n        legacy_import_repository::import(\n            &mut self.connection,\n            value,\n            self.metadata.schema_version,\n        )\n    }\n\n    pub(crate) fn load_settings(&self) -> Result<Option<StoredSettings>, StorageError> {",
        "database import method",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/worker.rs",
        "        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,\n        SessionEnd, SessionHistory, SessionStart, SessionUpdate, StoredSettings, TrustedDevice,\n",
        "        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,\n        LegacyAndroidImport, LegacyImportOutcome, SessionEnd, SessionHistory, SessionStart,\n        SessionUpdate, StoredSettings, TrustedDevice,\n",
        "worker legacy types",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/worker.rs",
        "    LoadSettings {\n",
        "    ImportLegacyAndroid {\n        value: LegacyAndroidImport,\n        reply: DatabaseReply<LegacyImportOutcome>,\n    },\n    LoadSettings {\n",
        "worker import command",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/worker.rs",
        "    /// Loads persisted settings when the singleton settings row exists.\n",
        "    /// Transactionally imports known Android legacy values exactly once.\n    ///\n    /// # Errors\n    ///\n    /// Returns a visible validation, queue, transaction, corruption, or worker error.\n    pub fn import_legacy_android(\n        &self,\n        value: &LegacyAndroidImport,\n    ) -> Result<LegacyImportOutcome, StorageError> {\n        let value = value.clone();\n        self.request(StorageOperation::Transaction, |reply| {\n            DatabaseCommand::ImportLegacyAndroid { value, reply }\n        })\n    }\n\n    /// Loads persisted settings when the singleton settings row exists.\n",
        "worker client import method",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/worker.rs",
        "        DatabaseCommand::LoadSettings { reply } => {\n",
        "        DatabaseCommand::ImportLegacyAndroid { value, reply } => {\n            process_import_legacy_android(&reply, &value, connection, version)?;\n        }\n        DatabaseCommand::LoadSettings { reply } => {\n",
        "worker command dispatch",
    )
    replace_once(
        "rust/silent-disco-core/src/storage/worker.rs",
        "fn process_load_settings(\n",
        "fn process_import_legacy_android(\n    reply: &DatabaseReply<LegacyImportOutcome>,\n    value: &LegacyAndroidImport,\n    connection: &mut Option<DatabaseConnection>,\n    schema_version: u32,\n) -> Result<(), StorageError> {\n    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)\n        .and_then(|database| database.import_legacy_android(value));\n    send_reply(\n        reply,\n        result,\n        connection,\n        StorageOperation::Transaction,\n        schema_version,\n    )\n}\n\nfn process_load_settings(\n",
        "worker import processor",
    )


if __name__ == "__main__":
    main()
