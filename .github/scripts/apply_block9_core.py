from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected exactly one match, found {count}: {old[:120]!r}")
    file_path.write_text(text.replace(old, new), encoding="utf-8")


legacy_import = r'''use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::domain::TrustState;

use super::{
    error::{StorageError, StorageErrorKind, StorageOperation, map_sqlite_error},
    models::{StoredSettings, TrustedDevice},
    repository_support::{invalid_model, to_sql_i64},
};

/// Stable Android SharedPreferences import contract version.
pub const ANDROID_LEGACY_IMPORT_VERSION: u32 = 1;
/// Stable source identifier recorded in SQLite after a committed import.
pub const ANDROID_LEGACY_IMPORT_SOURCE: &str = "android_shared_preferences";
/// Defensive upper bound for one legacy trust import.
pub const MAX_LEGACY_TRUSTED_DEVICE_COUNT: usize = 1_024;

/// Typed values read from the pre-Rust Android SharedPreferences store.
///
/// Kotlin may read only the known legacy keys. Rust validates this complete
/// record and commits settings, trusted devices, and the completion marker in
/// one transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyAndroidImport {
    pub version: u32,
    pub imported_at_ms: u64,
    pub settings: Option<StoredSettings>,
    pub trusted_devices: Vec<TrustedDevice>,
}

impl LegacyAndroidImport {
    /// Validates the complete import before any transaction begins.
    ///
    /// # Errors
    ///
    /// Returns a stable validation error for an unsupported version, invalid
    /// timestamp, duplicate device, excessive device count, invalid model, or
    /// non-trusted legacy record.
    pub fn validate(&self) -> Result<(), LegacyImportValidationError> {
        if self.version != ANDROID_LEGACY_IMPORT_VERSION {
            return Err(LegacyImportValidationError::UnsupportedVersion);
        }
        i64::try_from(self.imported_at_ms)
            .map_err(|_| LegacyImportValidationError::Timestamp)?;
        if self.trusted_devices.len() > MAX_LEGACY_TRUSTED_DEVICE_COUNT {
            return Err(LegacyImportValidationError::DeviceCount);
        }
        if let Some(settings) = &self.settings {
            settings
                .validate()
                .map_err(|_| LegacyImportValidationError::Settings)?;
        }
        let mut identifiers = BTreeSet::new();
        for device in &self.trusted_devices {
            device
                .validate()
                .map_err(|_| LegacyImportValidationError::TrustedDevice)?;
            if device.trust_state != TrustState::Trusted {
                return Err(LegacyImportValidationError::TrustState);
            }
            if !identifiers.insert(device.device_id.as_str()) {
                return Err(LegacyImportValidationError::DuplicateDevice);
            }
        }
        Ok(())
    }
}

/// Whether a valid import was newly committed or had already completed.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegacyImportDisposition {
    Imported = 1,
    AlreadyCompleted = 2,
}

/// Durable result of the one-time Android legacy import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyImportOutcome {
    pub disposition: LegacyImportDisposition,
    pub import_version: u32,
    pub completed_at_ms: u64,
    pub settings_imported: bool,
    pub trusted_device_count: u32,
}

/// Stable validation failures for [`LegacyAndroidImport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyImportValidationError {
    UnsupportedVersion,
    Timestamp,
    DeviceCount,
    Settings,
    TrustedDevice,
    TrustState,
    DuplicateDevice,
}

impl core::fmt::Display for LegacyImportValidationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedVersion => "legacy Android import version is unsupported",
            Self::Timestamp => "legacy Android import timestamp exceeds the SQLite integer range",
            Self::DeviceCount => "legacy Android import contains too many trusted devices",
            Self::Settings => "legacy Android import settings are invalid",
            Self::TrustedDevice => "legacy Android import contains invalid trusted-device metadata",
            Self::TrustState => "legacy Android import contains a device that is not trusted",
            Self::DuplicateDevice => "legacy Android import contains duplicate device identifiers",
        })
    }
}

impl std::error::Error for LegacyImportValidationError {}

pub(crate) fn import_android(
    connection: &mut Connection,
    import: &LegacyAndroidImport,
    schema_version: u32,
) -> Result<LegacyImportOutcome, StorageError> {
    import.validate().map_err(|error| {
        invalid_model(StorageOperation::ImportLegacyData, schema_version, error)
    })?;
    let completed_at_ms = to_sql_i64(
        import.imported_at_ms,
        StorageOperation::ImportLegacyData,
        schema_version,
        "completed_at_ms",
    )?;
    let trusted_device_count = u32::try_from(import.trusted_devices.len()).map_err(|_| {
        invalid_model(
            StorageOperation::ImportLegacyData,
            schema_version,
            "trusted device count exceeds the supported range",
        )
    })?;
    let trusted_device_count_sql = i64::from(trusted_device_count);
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            map_sqlite_error(
                StorageOperation::ImportLegacyData,
                Some(schema_version),
                &error,
            )
        })?;

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
        .map_err(|error| {
            map_sqlite_error(
                StorageOperation::ImportLegacyData,
                Some(schema_version),
                &error,
            )
        })?;
    if let Some((version, timestamp, settings_imported, device_count)) = existing {
        let version = u32::try_from(version).map_err(|_| {
            corrupt_import_marker(schema_version, "stored legacy import version is invalid")
        })?;
        if version != import.version {
            return Err(StorageError::new(
                StorageErrorKind::Migration,
                StorageOperation::ImportLegacyData,
                format!(
                    "legacy Android import marker version {version} does not match requested version {}",
                    import.version
                ),
                Some(schema_version),
            ));
        }
        return Ok(LegacyImportOutcome {
            disposition: LegacyImportDisposition::AlreadyCompleted,
            import_version: version,
            completed_at_ms: u64::try_from(timestamp).map_err(|_| {
                corrupt_import_marker(schema_version, "stored legacy import timestamp is invalid")
            })?,
            settings_imported: match settings_imported {
                0 => false,
                1 => true,
                _ => {
                    return Err(corrupt_import_marker(
                        schema_version,
                        "stored legacy settings-imported flag is invalid",
                    ));
                }
            },
            trusted_device_count: u32::try_from(device_count).map_err(|_| {
                corrupt_import_marker(schema_version, "stored legacy device count is invalid")
            })?,
        });
    }

    if let Some(settings) = &import.settings {
        let sync_cadence_ms = to_sql_i64(
            settings.tuning.sync_cadence_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "sync_cadence_ms",
        )?;
        let startup_buffer_ms = to_sql_i64(
            settings.tuning.startup_buffer_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "startup_buffer_ms",
        )?;
        let late_packet_threshold_ms = to_sql_i64(
            settings.tuning.late_packet_threshold_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "late_packet_threshold_ms",
        )?;
        let hard_resync_threshold_ms = to_sql_i64(
            settings.tuning.hard_resync_threshold_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "hard_resync_threshold_ms",
        )?;
        let scan_window_ms = to_sql_i64(
            settings.tuning.scan_window_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "scan_window_ms",
        )?;
        let updated_at_ms = to_sql_i64(
            settings.updated_at_ms,
            StorageOperation::ImportLegacyData,
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
            .map_err(|error| {
                map_sqlite_error(
                    StorageOperation::ImportLegacyData,
                    Some(schema_version),
                    &error,
                )
            })?;
    }

    for device in &import.trusted_devices {
        let first_seen_ms = to_sql_i64(
            device.first_seen_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "first_seen_ms",
        )?;
        let last_seen_ms = to_sql_i64(
            device.last_seen_ms,
            StorageOperation::ImportLegacyData,
            schema_version,
            "last_seen_ms",
        )?;
        let updated_at_ms = to_sql_i64(
            device.updated_at_ms,
            StorageOperation::ImportLegacyData,
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
            .map_err(|error| {
                map_sqlite_error(
                    StorageOperation::ImportLegacyData,
                    Some(schema_version),
                    &error,
                )
            })?;
    }

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
                trusted_device_count_sql,
            ],
        )
        .map_err(|error| {
            map_sqlite_error(
                StorageOperation::ImportLegacyData,
                Some(schema_version),
                &error,
            )
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(
            StorageOperation::ImportLegacyData,
            Some(schema_version),
            &error,
        )
    })?;

    Ok(LegacyImportOutcome {
        disposition: LegacyImportDisposition::Imported,
        import_version: import.version,
        completed_at_ms: import.imported_at_ms,
        settings_imported: import.settings.is_some(),
        trusted_device_count,
    })
}

fn corrupt_import_marker(schema_version: u32, message: &str) -> StorageError {
    StorageError::new(
        StorageErrorKind::Corruption,
        StorageOperation::ImportLegacyData,
        message,
        Some(schema_version),
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        domain::{DeviceId, TrustState, TuningSettings},
        storage::{
            DatabaseConfig, DatabaseWorker, LegacyAndroidImport, LegacyImportDisposition,
            StoredSettings, TrustedDevice, test_support::TestDatabasePath,
        },
    };

    fn trusted_device(identifier: &str, timestamp: u64) -> TrustedDevice {
        TrustedDevice {
            device_id: DeviceId::new(identifier).expect("valid device identifier"),
            display_name: identifier.to_owned(),
            public_key: None,
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: timestamp,
            last_seen_ms: timestamp,
            updated_at_ms: timestamp,
        }
    }

    #[test]
    fn imports_settings_and_devices_once_through_the_worker() {
        let path = TestDatabasePath::new("legacy-import-success");
        let mut worker = DatabaseWorker::start(
            DatabaseConfig::new(path.path()).expect("valid database configuration"),
        )
        .expect("start database worker");
        let client = worker.client();
        let import = LegacyAndroidImport {
            version: super::ANDROID_LEGACY_IMPORT_VERSION,
            imported_at_ms: 10_000,
            settings: Some(StoredSettings {
                tuning: TuningSettings {
                    sync_cadence_ms: 2_250,
                    ..TuningSettings::default()
                },
                updated_at_ms: 10_000,
            }),
            trusted_devices: vec![trusted_device("legacy-device", 10_000)],
        };

        let first = client
            .import_legacy_android_data(&import)
            .expect("first import succeeds");
        assert_eq!(first.disposition, LegacyImportDisposition::Imported);
        assert!(first.settings_imported);
        assert_eq!(first.trusted_device_count, 1);
        assert_eq!(
            client
                .load_settings()
                .expect("load settings")
                .expect("settings exist")
                .tuning
                .sync_cadence_ms,
            2_250
        );
        assert_eq!(
            client.list_trusted_devices().expect("list trusted devices"),
            import.trusted_devices
        );

        let changed = LegacyAndroidImport {
            settings: Some(StoredSettings {
                tuning: TuningSettings {
                    sync_cadence_ms: 4_000,
                    ..TuningSettings::default()
                },
                updated_at_ms: 20_000,
            }),
            imported_at_ms: 20_000,
            ..import
        };
        let second = client
            .import_legacy_android_data(&changed)
            .expect("repeat import is idempotent");
        assert_eq!(
            second.disposition,
            LegacyImportDisposition::AlreadyCompleted
        );
        assert_eq!(second.completed_at_ms, 10_000);
        assert_eq!(
            client
                .load_settings()
                .expect("load settings")
                .expect("settings exist")
                .tuning
                .sync_cadence_ms,
            2_250
        );
        worker.stop_and_join().expect("stop worker");
    }

    #[test]
    fn invalid_import_leaves_no_partial_settings_or_devices() {
        let path = TestDatabasePath::new("legacy-import-rollback");
        let mut worker = DatabaseWorker::start(
            DatabaseConfig::new(path.path()).expect("valid database configuration"),
        )
        .expect("start database worker");
        let client = worker.client();
        let duplicate = trusted_device("duplicate", 5_000);
        let import = LegacyAndroidImport {
            version: super::ANDROID_LEGACY_IMPORT_VERSION,
            imported_at_ms: 5_000,
            settings: Some(StoredSettings {
                tuning: TuningSettings::default(),
                updated_at_ms: 5_000,
            }),
            trusted_devices: vec![duplicate.clone(), duplicate],
        };

        let error = client
            .import_legacy_android_data(&import)
            .expect_err("duplicate import must fail before commit");
        assert_eq!(error.operation, crate::storage::StorageOperation::ImportLegacyData);
        assert_eq!(client.load_settings().expect("load settings"), None);
        assert!(
            client
                .list_trusted_devices()
                .expect("list trusted devices")
                .is_empty()
        );
        worker.stop_and_join().expect("stop worker");
    }
}
'''
Path("rust/silent-disco-core/src/storage/legacy_import.rs").write_text(legacy_import, encoding="utf-8")

replace_exact(
    "rust/silent-disco-core/src/storage/migrations.rs",
    "pub const LATEST_SCHEMA_VERSION: u32 = 1;",
    "pub const LATEST_SCHEMA_VERSION: u32 = 2;",
)
replace_exact(
    "rust/silent-disco-core/src/storage/migrations.rs",
    '''const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: MIGRATION_V1_SQL,
}];''',
    '''const MIGRATION_V2_SQL: &str = r"
CREATE TABLE legacy_imports (
    source               TEXT PRIMARY KEY CHECK (length(source) BETWEEN 1 AND 64),
    import_version       INTEGER NOT NULL CHECK (import_version > 0),
    completed_at_ms      INTEGER NOT NULL CHECK (completed_at_ms >= 0),
    settings_imported    INTEGER NOT NULL CHECK (settings_imported IN (0, 1)),
    trusted_device_count INTEGER NOT NULL CHECK (trusted_device_count >= 0)
) STRICT;
";

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: MIGRATION_V1_SQL,
    },
    Migration {
        version: 2,
        sql: MIGRATION_V2_SQL,
    },
];''',
)
replace_exact(
    "rust/silent-disco-core/src/storage/migrations.rs",
    "assert_eq!(first.records.len(), 1);",
    "assert_eq!(first.records.len(), usize::try_from(LATEST_SCHEMA_VERSION).expect(\"schema version fits usize\"));",
)

replace_exact(
    "rust/silent-disco-core/src/storage/error.rs",
    "    JoinWorker = 15,\n",
    "    JoinWorker = 15,\n    ImportLegacyData = 16,\n",
)
replace_exact(
    "rust/silent-disco-core/src/storage/error.rs",
    "            Self::JoinWorker => \"join_worker\",\n",
    "            Self::JoinWorker => \"join_worker\",\n            Self::ImportLegacyData => \"import_legacy_data\",\n",
)
replace_exact(
    "rust/silent-disco-core/src/storage/error.rs",
    "            Self::Transaction => StorageErrorKind::Transaction,\n",
    "            Self::Transaction | Self::ImportLegacyData => StorageErrorKind::Transaction,\n",
)

replace_exact(
    "rust/silent-disco-core/src/storage/mod.rs",
    "mod error;\nmod migrations;",
    "mod error;\nmod legacy_import;\nmod migrations;",
)
replace_exact(
    "rust/silent-disco-core/src/storage/mod.rs",
    "pub use error::{StorageError, StorageErrorKind, StorageOperation};\n",
    "pub use error::{StorageError, StorageErrorKind, StorageOperation};\npub use legacy_import::{\n    ANDROID_LEGACY_IMPORT_SOURCE, ANDROID_LEGACY_IMPORT_VERSION, LegacyAndroidImport,\n    LegacyImportDisposition, LegacyImportOutcome, LegacyImportValidationError,\n    MAX_LEGACY_TRUSTED_DEVICE_COUNT,\n};\n",
)

replace_exact(
    "rust/silent-disco-core/src/storage/database.rs",
    "    migrations,\n",
    "    legacy_import::{self, LegacyAndroidImport, LegacyImportOutcome},\n    migrations,\n",
)
replace_exact(
    "rust/silent-disco-core/src/storage/database.rs",
    '''    pub(crate) fn load_settings(&self) -> Result<Option<StoredSettings>, StorageError> {
        settings_repository::load(&self.connection, self.metadata.schema_version)
    }
''',
    '''    pub(crate) fn import_legacy_android_data(
        &mut self,
        import: &LegacyAndroidImport,
    ) -> Result<LegacyImportOutcome, StorageError> {
        legacy_import::import_android(&mut self.connection, import, self.metadata.schema_version)
    }

    pub(crate) fn load_settings(&self) -> Result<Option<StoredSettings>, StorageError> {
        settings_repository::load(&self.connection, self.metadata.schema_version)
    }
''',
)

replace_exact(
    "rust/silent-disco-core/src/storage/worker.rs",
    "    error::{StorageError, StorageErrorKind, StorageOperation},\n",
    "    error::{StorageError, StorageErrorKind, StorageOperation},\n    legacy_import::{LegacyAndroidImport, LegacyImportOutcome},\n",
)
replace_exact(
    "rust/silent-disco-core/src/storage/worker.rs",
    '''    ReadMetadata {
        reply: DatabaseReply<DatabaseMetadata>,
    },
''',
    '''    ReadMetadata {
        reply: DatabaseReply<DatabaseMetadata>,
    },
    ImportLegacyAndroidData {
        import: LegacyAndroidImport,
        reply: DatabaseReply<LegacyImportOutcome>,
    },
''',
)
replace_exact(
    "rust/silent-disco-core/src/storage/worker.rs",
    '''    /// Loads persisted settings when the singleton settings row exists.
''',
    '''    /// Atomically imports the typed legacy Android persistence record once.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, migration, transaction, constraint,
    /// corruption, or worker lifecycle error. A failure commits no partial data.
    pub fn import_legacy_android_data(
        &self,
        import: &LegacyAndroidImport,
    ) -> Result<LegacyImportOutcome, StorageError> {
        let import = import.clone();
        self.request(StorageOperation::ImportLegacyData, |reply| {
            DatabaseCommand::ImportLegacyAndroidData { import, reply }
        })
    }

    /// Loads persisted settings when the singleton settings row exists.
''',
)
replace_exact(
    "rust/silent-disco-core/src/storage/worker.rs",
    '''        DatabaseCommand::ReadMetadata { reply } => {
            process_read_metadata(&reply, connection, version)?;
        }
''',
    '''        DatabaseCommand::ReadMetadata { reply } => {
            process_read_metadata(&reply, connection, version)?;
        }
        DatabaseCommand::ImportLegacyAndroidData { import, reply } => {
            process_import_legacy_android_data(&reply, &import, connection, version)?;
        }
''',
)
replace_exact(
    "rust/silent-disco-core/src/storage/worker.rs",
    '''fn process_load_settings(
''',
    '''fn process_import_legacy_android_data(
    reply: &DatabaseReply<LegacyImportOutcome>,
    import: &LegacyAndroidImport,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(
        connection,
        StorageOperation::ImportLegacyData,
        schema_version,
    )
    .and_then(|database| database.import_legacy_android_data(import));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::ImportLegacyData,
        schema_version,
    )
}

fn process_load_settings(
''',
)
