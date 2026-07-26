use std::{path::PathBuf, thread, time::Duration};

use rusqlite::{Connection, OpenFlags};

use super::{
    diagnostics_repository,
    error::{StorageError, StorageErrorKind, StorageOperation, map_sqlite_error},
    legacy_import::{self, LegacyAndroidImport, LegacyImportOutcome},
    migrations,
    models::{
        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
        SessionEnd, SessionHistory, SessionStart, SessionUpdate, StoredSettings, TrustedDevice,
    },
    session_read_repository, session_write_repository, settings_repository,
    trusted_device_repository,
};
use crate::domain::{DeviceId, SessionId};

pub const DEFAULT_DATABASE_QUEUE_CAPACITY: usize = 32;
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 2_000;
const MAX_BUSY_TIMEOUT_MS: u128 = i32::MAX as u128;

/// Explicit `SQLite` synchronous policy for low-write-volume durable mobile data.
///
/// `Full` is intentionally selected for the initial implementation. With WAL,
/// `SQLite` synchronizes the WAL on every transaction commit, prioritizing
/// durable settings, trust, session, and diagnostic records over write latency.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SynchronousPolicy {
    Full = 2,
}

impl SynchronousPolicy {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Full => "full",
        }
    }

    const fn pragma_value(self) -> &'static str {
        match self {
            Self::Full => "FULL",
        }
    }

    const fn sqlite_numeric_value(self) -> i64 {
        self as i64
    }
}

/// Validated configuration used to start one `SQLite` worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub path: PathBuf,
    pub queue_capacity: usize,
    pub busy_timeout: Duration,
    pub synchronous_policy: SynchronousPolicy,
}

impl DatabaseConfig {
    /// Creates the default durable worker configuration for one file path.
    ///
    /// # Errors
    ///
    /// Returns an invalid-configuration error for an empty path.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let config = Self {
            path: path.into(),
            queue_capacity: DEFAULT_DATABASE_QUEUE_CAPACITY,
            busy_timeout: Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            synchronous_policy: SynchronousPolicy::Full,
        };
        config.validate()?;
        Ok(config)
    }

    /// Selects the bounded command queue capacity.
    ///
    /// # Errors
    ///
    /// Returns an invalid-configuration error for zero capacity.
    pub fn with_queue_capacity(mut self, queue_capacity: usize) -> Result<Self, StorageError> {
        self.queue_capacity = queue_capacity;
        self.validate()?;
        Ok(self)
    }

    /// Selects a nonzero `SQLite` busy timeout that fits `SQLite`'s millisecond API.
    ///
    /// # Errors
    ///
    /// Returns an invalid-configuration error for zero or excessive duration.
    pub fn with_busy_timeout(mut self, busy_timeout: Duration) -> Result<Self, StorageError> {
        self.busy_timeout = busy_timeout;
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn validate(&self) -> Result<(), StorageError> {
        if self.path.as_os_str().is_empty() {
            return Err(StorageError::new(
                StorageErrorKind::InvalidConfiguration,
                StorageOperation::ValidateConfiguration,
                "database path must not be empty",
                None,
            ));
        }
        if self.queue_capacity == 0 {
            return Err(StorageError::new(
                StorageErrorKind::InvalidConfiguration,
                StorageOperation::ValidateConfiguration,
                "database queue capacity must be greater than zero",
                None,
            ));
        }
        let busy_timeout_ms = self.busy_timeout.as_millis();
        if busy_timeout_ms == 0 || busy_timeout_ms > MAX_BUSY_TIMEOUT_MS {
            return Err(StorageError::new(
                StorageErrorKind::InvalidConfiguration,
                StorageOperation::ValidateConfiguration,
                "database busy timeout must be between 1 and i32::MAX milliseconds",
                None,
            ));
        }
        Ok(())
    }
}

/// Verified `SQLite` runtime and migration metadata exposed for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseMetadata {
    pub sqlite_version: String,
    pub foreign_keys_enabled: bool,
    pub journal_mode: String,
    pub busy_timeout_ms: u32,
    pub synchronous_policy: SynchronousPolicy,
    pub schema_version: u32,
    pub applied_migrations: Vec<migrations::MigrationRecord>,
    pub integrity_check: String,
    pub owner_thread_id: String,
    pub owner_thread_name: String,
}

/// Result returned by an explicit WAL checkpoint request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseCheckpoint {
    pub busy_readers: u32,
    pub wal_frames: u32,
    pub checkpointed_frames: u32,
}

pub(crate) struct DatabaseConnection {
    connection: Connection,
    metadata: DatabaseMetadata,
}

impl DatabaseConnection {
    pub(crate) fn open(config: &DatabaseConfig) -> Result<Self, StorageError> {
        config.validate()?;
        let mut connection = Connection::open_with_flags(
            &config.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|error| map_sqlite_error(StorageOperation::OpenDatabase, None, &error))?;

        configure_foreign_keys(&connection)?;
        let journal_mode = configure_wal(&connection)?;
        let busy_timeout_ms = configure_busy_timeout(&connection, config.busy_timeout)?;
        configure_synchronous_policy(&connection, config.synchronous_policy)?;
        let migration_report = migrations::run_migrations(&mut connection)?;
        let integrity_check = verify_quick_check(&connection, migration_report.schema_version)?;
        let owner_thread = thread::current();
        let owner_thread_name = owner_thread.name().unwrap_or("unnamed").to_owned();
        let metadata = DatabaseMetadata {
            sqlite_version: rusqlite::version().to_owned(),
            foreign_keys_enabled: true,
            journal_mode,
            busy_timeout_ms,
            synchronous_policy: config.synchronous_policy,
            schema_version: migration_report.schema_version,
            applied_migrations: migration_report.records,
            integrity_check,
            owner_thread_id: format!("{:?}", owner_thread.id()),
            owner_thread_name,
        };
        Ok(Self {
            connection,
            metadata,
        })
    }

    pub(crate) fn metadata(&self) -> DatabaseMetadata {
        self.metadata.clone()
    }

    pub(crate) fn import_legacy_android_data(
        &mut self,
        import: &LegacyAndroidImport,
    ) -> Result<LegacyImportOutcome, StorageError> {
        legacy_import::import_android(&mut self.connection, import, self.metadata.schema_version)
    }

    pub(crate) fn load_settings(&self) -> Result<Option<StoredSettings>, StorageError> {
        settings_repository::load(&self.connection, self.metadata.schema_version)
    }

    pub(crate) fn save_settings(&mut self, settings: &StoredSettings) -> Result<(), StorageError> {
        settings_repository::save(&mut self.connection, settings, self.metadata.schema_version)
    }

    pub(crate) fn list_trusted_devices(&self) -> Result<Vec<TrustedDevice>, StorageError> {
        trusted_device_repository::list(&self.connection, self.metadata.schema_version)
    }

    pub(crate) fn get_trusted_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Option<TrustedDevice>, StorageError> {
        trusted_device_repository::get(&self.connection, device_id, self.metadata.schema_version)
    }

    pub(crate) fn upsert_trusted_device(
        &mut self,
        device: &TrustedDevice,
    ) -> Result<(), StorageError> {
        trusted_device_repository::upsert(
            &mut self.connection,
            device,
            self.metadata.schema_version,
        )
    }

    pub(crate) fn delete_trusted_device(
        &mut self,
        device_id: &DeviceId,
    ) -> Result<bool, StorageError> {
        trusted_device_repository::delete(
            &mut self.connection,
            device_id,
            self.metadata.schema_version,
        )
    }

    pub(crate) fn begin_session(&mut self, session: &SessionStart) -> Result<(), StorageError> {
        session_write_repository::begin(&mut self.connection, session, self.metadata.schema_version)
    }

    pub(crate) fn update_session(&mut self, update: &SessionUpdate) -> Result<bool, StorageError> {
        session_write_repository::update(&mut self.connection, update, self.metadata.schema_version)
    }

    pub(crate) fn end_session(&mut self, end: &SessionEnd) -> Result<bool, StorageError> {
        session_write_repository::end(&mut self.connection, end, self.metadata.schema_version)
    }

    pub(crate) fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionHistory>, StorageError> {
        session_read_repository::get(&self.connection, session_id, self.metadata.schema_version)
    }

    pub(crate) fn insert_diagnostic_run(
        &mut self,
        run: &DiagnosticRunSummary,
    ) -> Result<(), StorageError> {
        diagnostics_repository::insert(&mut self.connection, run, self.metadata.schema_version)
    }

    pub(crate) fn query_diagnostic_runs(
        &self,
        query: &DiagnosticQuery,
    ) -> Result<Vec<DiagnosticRunSummary>, StorageError> {
        diagnostics_repository::query(&self.connection, query, self.metadata.schema_version)
    }

    pub(crate) fn export_diagnostic_runs(
        &self,
        request: &DiagnosticExportRequest,
    ) -> Result<DiagnosticExport, StorageError> {
        diagnostics_repository::export(&self.connection, request, self.metadata.schema_version)
    }

    pub(crate) fn checkpoint(&self) -> Result<DatabaseCheckpoint, StorageError> {
        checkpoint_with_mode(&self.connection, "PASSIVE", self.metadata.schema_version)
    }

    pub(crate) fn checkpoint_and_close(self) -> Result<(), StorageError> {
        let schema_version = self.metadata.schema_version;
        let checkpoint_result = checkpoint_with_mode(&self.connection, "TRUNCATE", schema_version)
            .and_then(|checkpoint| {
                if checkpoint.busy_readers == 0 {
                    Ok(())
                } else {
                    Err(StorageError::new(
                        StorageErrorKind::Busy,
                        StorageOperation::Checkpoint,
                        format!(
                            "WAL checkpoint could not complete because {} reader(s) were busy",
                            checkpoint.busy_readers
                        ),
                        Some(schema_version),
                    ))
                }
            });

        let close_result = match self.connection.close() {
            Ok(()) => Ok(()),
            Err((_connection, error)) => Err(map_sqlite_error(
                StorageOperation::CloseDatabase,
                Some(schema_version),
                &error,
            )),
        };

        match (checkpoint_result, close_result) {
            (_, Err(close_error)) => Err(close_error),
            (Err(checkpoint_error), Ok(())) => Err(checkpoint_error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

fn configure_foreign_keys(connection: &Connection) -> Result<(), StorageError> {
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| map_sqlite_error(StorageOperation::ConfigureForeignKeys, None, &error))?;
    let enabled = connection
        .query_row("PRAGMA foreign_keys;", [], |row| row.get::<_, i64>(0))
        .map_err(|error| map_sqlite_error(StorageOperation::ConfigureForeignKeys, None, &error))?;
    if enabled != 1 {
        return Err(StorageError::new(
            StorageErrorKind::Pragma,
            StorageOperation::ConfigureForeignKeys,
            format!("foreign_keys verification returned {enabled}, expected 1"),
            None,
        ));
    }
    Ok(())
}

fn configure_wal(connection: &Connection) -> Result<String, StorageError> {
    let journal_mode = connection
        .query_row("PRAGMA journal_mode = WAL;", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| map_sqlite_error(StorageOperation::ConfigureJournalMode, None, &error))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::new(
            StorageErrorKind::Pragma,
            StorageOperation::ConfigureJournalMode,
            format!("journal_mode verification returned {journal_mode}, expected wal"),
            None,
        ));
    }
    Ok(journal_mode.to_ascii_lowercase())
}

fn configure_busy_timeout(connection: &Connection, timeout: Duration) -> Result<u32, StorageError> {
    connection
        .busy_timeout(timeout)
        .map_err(|error| map_sqlite_error(StorageOperation::ConfigureBusyTimeout, None, &error))?;
    let configured = connection
        .query_row("PRAGMA busy_timeout;", [], |row| row.get::<_, i64>(0))
        .map_err(|error| map_sqlite_error(StorageOperation::ConfigureBusyTimeout, None, &error))?;
    let expected = i64::try_from(timeout.as_millis()).map_err(|_| {
        StorageError::new(
            StorageErrorKind::InvalidConfiguration,
            StorageOperation::ConfigureBusyTimeout,
            "database busy timeout does not fit SQLite's millisecond field",
            None,
        )
    })?;
    if configured != expected {
        return Err(StorageError::new(
            StorageErrorKind::Pragma,
            StorageOperation::ConfigureBusyTimeout,
            format!("busy_timeout verification returned {configured}, expected {expected}"),
            None,
        ));
    }
    u32::try_from(configured).map_err(|_| {
        StorageError::new(
            StorageErrorKind::Pragma,
            StorageOperation::ConfigureBusyTimeout,
            "verified busy_timeout does not fit the diagnostics field",
            None,
        )
    })
}

fn configure_synchronous_policy(
    connection: &Connection,
    policy: SynchronousPolicy,
) -> Result<(), StorageError> {
    connection
        .execute_batch(&format!("PRAGMA synchronous = {};", policy.pragma_value()))
        .map_err(|error| {
            map_sqlite_error(StorageOperation::ConfigureSynchronousPolicy, None, &error)
        })?;
    let configured = connection
        .query_row("PRAGMA synchronous;", [], |row| row.get::<_, i64>(0))
        .map_err(|error| {
            map_sqlite_error(StorageOperation::ConfigureSynchronousPolicy, None, &error)
        })?;
    if configured != policy.sqlite_numeric_value() {
        return Err(StorageError::new(
            StorageErrorKind::Pragma,
            StorageOperation::ConfigureSynchronousPolicy,
            format!(
                "synchronous verification returned {configured}, expected {} ({})",
                policy.sqlite_numeric_value(),
                policy.stable_name()
            ),
            None,
        ));
    }
    Ok(())
}

fn verify_quick_check(
    connection: &Connection,
    schema_version: u32,
) -> Result<String, StorageError> {
    let result = connection
        .query_row("PRAGMA quick_check(1);", [], |row| row.get::<_, String>(0))
        .map_err(|error| {
            map_sqlite_error(StorageOperation::ReadMetadata, Some(schema_version), &error)
        })?;
    if !result.eq_ignore_ascii_case("ok") {
        return Err(StorageError::new(
            StorageErrorKind::Corruption,
            StorageOperation::ReadMetadata,
            format!("SQLite quick_check failed: {result}"),
            Some(schema_version),
        ));
    }
    Ok(result.to_ascii_lowercase())
}

fn checkpoint_with_mode(
    connection: &Connection,
    mode: &str,
    schema_version: u32,
) -> Result<DatabaseCheckpoint, StorageError> {
    let sql = format!("PRAGMA wal_checkpoint({mode});");
    connection
        .query_row(&sql, [], |row| {
            Ok(DatabaseCheckpoint {
                busy_readers: row.get(0)?,
                wal_frames: row.get(1)?,
                checkpointed_frames: row.get(2)?,
            })
        })
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Checkpoint, Some(schema_version), &error)
        })
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use super::{DEFAULT_BUSY_TIMEOUT_MS, DatabaseConfig, DatabaseConnection, SynchronousPolicy};
    use crate::storage::{
        LATEST_SCHEMA_VERSION, StorageErrorKind, StorageOperation, test_support::TestDatabasePath,
    };

    #[test]
    fn verifies_durable_connection_policy_and_latest_schema() {
        let test_path = TestDatabasePath::new("database-policy");
        let config = DatabaseConfig::new(test_path.path()).expect("valid config");
        let connection = DatabaseConnection::open(&config).expect("database opens");
        let metadata = connection.metadata();

        assert!(metadata.foreign_keys_enabled);
        assert_eq!(metadata.journal_mode, "wal");
        assert_eq!(u64::from(metadata.busy_timeout_ms), DEFAULT_BUSY_TIMEOUT_MS);
        assert_eq!(metadata.synchronous_policy, SynchronousPolicy::Full);
        assert_eq!(metadata.schema_version, LATEST_SCHEMA_VERSION);
        assert_eq!(
            metadata.applied_migrations.len(),
            usize::try_from(LATEST_SCHEMA_VERSION).expect("schema version fits usize"),
        );
        assert_eq!(metadata.integrity_check, "ok");
        assert!(!metadata.owner_thread_name.is_empty());
        connection
            .checkpoint_and_close()
            .expect("checkpoint and close succeeds");
    }

    #[test]
    fn rejects_invalid_configuration_and_non_wal_database() {
        let zero_queue = DatabaseConfig::new("database.sqlite3")
            .and_then(|config| config.with_queue_capacity(0))
            .expect_err("zero queue must fail");
        assert_eq!(zero_queue.kind, StorageErrorKind::InvalidConfiguration);
        assert_eq!(
            zero_queue.operation,
            StorageOperation::ValidateConfiguration
        );

        let zero_timeout = DatabaseConfig::new("database.sqlite3")
            .and_then(|config| config.with_busy_timeout(Duration::ZERO))
            .expect_err("zero timeout must fail");
        assert_eq!(zero_timeout.kind, StorageErrorKind::InvalidConfiguration);

        let memory_config = DatabaseConfig::new(":memory:").expect("valid path shape");
        let memory_error = match DatabaseConnection::open(&memory_config) {
            Ok(connection) => {
                let _ = connection.checkpoint_and_close();
                panic!("memory database unexpectedly verified WAL")
            }
            Err(error) => error,
        };
        assert_eq!(memory_error.kind, StorageErrorKind::Pragma);
    }

    #[test]
    fn corrupt_file_fails_without_recreation() {
        let test_path = TestDatabasePath::new("database-corrupt");
        fs::write(test_path.path(), b"not a sqlite database").expect("write corrupt file");
        let config = DatabaseConfig::new(test_path.path()).expect("valid config");
        let error = match DatabaseConnection::open(&config) {
            Ok(connection) => {
                let _ = connection.checkpoint_and_close();
                panic!("corrupt database unexpectedly opened")
            }
            Err(error) => error,
        };
        assert_eq!(error.kind, StorageErrorKind::Corruption);
        assert_eq!(
            fs::read(test_path.path()).expect("corrupt file remains"),
            b"not a sqlite database"
        );
    }
}
