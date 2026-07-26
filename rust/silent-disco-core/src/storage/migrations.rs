use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, TransactionBehavior, params};

use super::error::{StorageError, StorageErrorKind, StorageOperation, map_sqlite_error};

pub const LATEST_SCHEMA_VERSION: u32 = 2;

const MIGRATION_V1_SQL: &str = r"
CREATE TABLE schema_migrations (
    version       INTEGER PRIMARY KEY CHECK (version > 0),
    applied_at_ms INTEGER NOT NULL CHECK (applied_at_ms >= 0),
    checksum      TEXT NOT NULL CHECK (length(checksum) > 0)
) STRICT;

CREATE TABLE app_settings (
    id                         INTEGER PRIMARY KEY CHECK (id = 1),
    sync_sample_window         INTEGER NOT NULL CHECK (sync_sample_window BETWEEN 4 AND 32),
    sync_cadence_ms            INTEGER NOT NULL CHECK (sync_cadence_ms BETWEEN 500 AND 5000),
    startup_buffer_ms          INTEGER NOT NULL CHECK (startup_buffer_ms BETWEEN 100 AND 1500),
    late_packet_threshold_ms   INTEGER NOT NULL CHECK (late_packet_threshold_ms BETWEEN 10 AND 250),
    hard_resync_threshold_ms   INTEGER NOT NULL CHECK (hard_resync_threshold_ms BETWEEN 40 AND 500),
    sync_drift_threshold_ms    REAL NOT NULL CHECK (
        sync_drift_threshold_ms BETWEEN 4.0 AND 100.0
    ),
    scan_window_ms             INTEGER NOT NULL CHECK (scan_window_ms BETWEEN 1000 AND 10000),
    updated_at_ms              INTEGER NOT NULL CHECK (updated_at_ms >= 0),
    CHECK (hard_resync_threshold_ms >= late_packet_threshold_ms + 20)
) STRICT;

CREATE TABLE trusted_devices (
    device_id          TEXT PRIMARY KEY CHECK (length(device_id) BETWEEN 1 AND 128),
    display_name       TEXT NOT NULL CHECK (
        length(display_name) BETWEEN 1 AND 256 AND trim(display_name) <> ''
    ),
    public_key         BLOB CHECK (
        public_key IS NULL OR length(public_key) BETWEEN 1 AND 4096
    ),
    private_key_ref    TEXT CHECK (
        private_key_ref IS NULL OR (
            length(private_key_ref) BETWEEN 1 AND 512
            AND trim(private_key_ref) = private_key_ref
        )
    ),
    trust_state        TEXT NOT NULL CHECK (trust_state IN ('session_only', 'trusted')),
    first_seen_ms      INTEGER NOT NULL CHECK (first_seen_ms >= 0),
    last_seen_ms       INTEGER NOT NULL CHECK (last_seen_ms >= first_seen_ms),
    updated_at_ms      INTEGER NOT NULL CHECK (updated_at_ms >= last_seen_ms)
) STRICT;

CREATE TABLE session_history (
    session_id         TEXT PRIMARY KEY CHECK (length(session_id) BETWEEN 1 AND 128),
    role               TEXT NOT NULL CHECK (role IN ('host', 'listener')),
    session_name       TEXT NOT NULL CHECK (
        length(session_name) BETWEEN 1 AND 256 AND trim(session_name) <> ''
    ),
    started_at_ms      INTEGER NOT NULL CHECK (started_at_ms >= 0),
    ended_at_ms        INTEGER CHECK (
        ended_at_ms IS NULL OR ended_at_ms >= started_at_ms
    ),
    listener_count     INTEGER NOT NULL DEFAULT 0 CHECK (listener_count >= 0),
    outcome            TEXT NOT NULL CHECK (
        outcome IN ('active', 'completed', 'cancelled', 'failed')
    ),
    failure_code       TEXT CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    ),
    failure_message    TEXT CHECK (
        failure_message IS NULL OR length(failure_message) <= 512
    ),
    CHECK (
        (outcome = 'active' AND ended_at_ms IS NULL)
        OR (outcome <> 'active' AND ended_at_ms IS NOT NULL)
    ),
    CHECK (
        (outcome = 'failed' AND failure_code IS NOT NULL)
        OR (outcome <> 'failed' AND failure_code IS NULL AND failure_message IS NULL)
    )
) STRICT;

CREATE TABLE diagnostic_runs (
    run_id             TEXT PRIMARY KEY CHECK (length(run_id) BETWEEN 1 AND 128),
    session_id         TEXT,
    started_at_ms      INTEGER NOT NULL CHECK (started_at_ms >= 0),
    ended_at_ms        INTEGER CHECK (
        ended_at_ms IS NULL OR ended_at_ms >= started_at_ms
    ),
    summary_json       TEXT NOT NULL CHECK (
        length(summary_json) BETWEEN 1 AND 262144 AND json_valid(summary_json)
    ),
    FOREIGN KEY(session_id) REFERENCES session_history(session_id)
        ON UPDATE CASCADE ON DELETE SET NULL
) STRICT;

CREATE INDEX idx_trusted_devices_last_seen
    ON trusted_devices(last_seen_ms DESC, device_id);

CREATE INDEX idx_session_history_started
    ON session_history(started_at_ms DESC, session_id);

CREATE INDEX idx_session_history_outcome
    ON session_history(outcome, started_at_ms DESC);

CREATE INDEX idx_diagnostic_runs_session
    ON diagnostic_runs(session_id, started_at_ms DESC, run_id);
";

#[derive(Debug, Clone, Copy)]
struct Migration {
    version: u32,
    sql: &'static str,
}

impl Migration {
    fn checksum(self) -> String {
        format!("fnv1a64:{:016x}", fnv1a64(self.sql.as_bytes()))
    }
}

const MIGRATION_V2_SQL: &str = r"
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
];

/// One migration row persisted in `schema_migrations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRecord {
    pub version: u32,
    pub applied_at_ms: u64,
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationReport {
    pub schema_version: u32,
    pub records: Vec<MigrationRecord>,
}

pub(crate) fn run_migrations(connection: &mut Connection) -> Result<MigrationReport, StorageError> {
    run_migration_catalog(connection, MIGRATIONS)
}

fn run_migration_catalog(
    connection: &mut Connection,
    migrations: &[Migration],
) -> Result<MigrationReport, StorageError> {
    validate_catalog(migrations)?;
    let latest_version = migrations.last().map_or(0, |migration| migration.version);
    let current_version = read_user_version(connection)?;
    if current_version > latest_version {
        return Err(migration_failure(
            format!(
                "database schema version {current_version} is newer than supported version {latest_version}"
            ),
            Some(current_version),
        ));
    }
    if current_version > 0 {
        verify_applied_migrations(connection, migrations, current_version)?;
    }

    for migration in migrations
        .iter()
        .copied()
        .filter(|migration| migration.version > current_version)
    {
        apply_migration(connection, migration)?;
    }

    let final_version = read_user_version(connection)?;
    if final_version != latest_version {
        return Err(migration_failure(
            format!(
                "database schema version {final_version} does not match latest version {latest_version}"
            ),
            Some(final_version),
        ));
    }
    let records = if final_version == 0 {
        Vec::new()
    } else {
        verify_applied_migrations(connection, migrations, final_version)?
    };
    Ok(MigrationReport {
        schema_version: final_version,
        records,
    })
}

fn validate_catalog(migrations: &[Migration]) -> Result<(), StorageError> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected_version = u32::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| migration_failure("migration catalog is too large", None))?;
        if migration.version != expected_version || migration.sql.trim().is_empty() {
            return Err(migration_failure(
                "migration catalog must be contiguous, ordered, and nonempty",
                None,
            ));
        }
    }
    Ok(())
}

fn apply_migration(connection: &mut Connection, migration: Migration) -> Result<(), StorageError> {
    let applied_at_ms = current_unix_millis()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            map_sqlite_error(
                StorageOperation::Migration,
                Some(migration.version.saturating_sub(1)),
                &error,
            )
        })?;
    transaction.execute_batch(migration.sql).map_err(|error| {
        map_sqlite_error(
            StorageOperation::Migration,
            Some(migration.version.saturating_sub(1)),
            &error,
        )
    })?;
    transaction
        .execute(
            "INSERT INTO schema_migrations(version, applied_at_ms, checksum) VALUES (?1, ?2, ?3)",
            params![
                i64::from(migration.version),
                applied_at_ms,
                migration.checksum()
            ],
        )
        .map_err(|error| {
            map_sqlite_error(
                StorageOperation::Migration,
                Some(migration.version.saturating_sub(1)),
                &error,
            )
        })?;
    transaction
        .pragma_update(None, "user_version", migration.version)
        .map_err(|error| {
            map_sqlite_error(
                StorageOperation::Migration,
                Some(migration.version.saturating_sub(1)),
                &error,
            )
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Migration, Some(migration.version), &error)
    })
}

fn verify_applied_migrations(
    connection: &Connection,
    migrations: &[Migration],
    schema_version: u32,
) -> Result<Vec<MigrationRecord>, StorageError> {
    let records = read_migration_records(connection, schema_version)?;
    let expected_count = usize::try_from(schema_version).map_err(|_| {
        migration_failure(
            "schema version does not fit memory limits",
            Some(schema_version),
        )
    })?;
    if records.len() != expected_count {
        return Err(migration_failure(
            format!(
                "schema version {schema_version} has {} migration records; expected {expected_count}",
                records.len()
            ),
            Some(schema_version),
        ));
    }

    for (record, migration) in records.iter().zip(migrations.iter()) {
        if record.version != migration.version {
            return Err(migration_failure(
                "migration history contains a missing or unexpected version",
                Some(schema_version),
            ));
        }
        let expected_checksum = migration.checksum();
        if record.checksum != expected_checksum {
            return Err(migration_failure(
                format!(
                    "migration {} checksum does not match the compiled migration",
                    record.version
                ),
                Some(schema_version),
            ));
        }
    }
    Ok(records)
}

fn read_migration_records(
    connection: &Connection,
    schema_version: u32,
) -> Result<Vec<MigrationRecord>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT version, applied_at_ms, checksum
             FROM schema_migrations
             ORDER BY version ASC",
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Migration, Some(schema_version), &error)
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Migration, Some(schema_version), &error)
        })?;

    let mut records = Vec::new();
    for row in rows {
        let (version, applied_at_ms, checksum) = row.map_err(|error| {
            map_sqlite_error(StorageOperation::Migration, Some(schema_version), &error)
        })?;
        let version = u32::try_from(version).map_err(|_| {
            migration_failure(
                "migration history contains an invalid version",
                Some(schema_version),
            )
        })?;
        let applied_at_ms = u64::try_from(applied_at_ms).map_err(|_| {
            migration_failure(
                "migration history contains an invalid timestamp",
                Some(schema_version),
            )
        })?;
        records.push(MigrationRecord {
            version,
            applied_at_ms,
            checksum,
        });
    }
    Ok(records)
}

fn read_user_version(connection: &Connection) -> Result<u32, StorageError> {
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| map_sqlite_error(StorageOperation::Migration, None, &error))?;
    u32::try_from(version).map_err(|_| {
        migration_failure("database user_version is outside the supported range", None)
    })
}

fn current_unix_millis() -> Result<i64, StorageError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        migration_failure(
            "system clock is before the Unix epoch; migration timestamp is unavailable",
            None,
        )
    })?;
    i64::try_from(duration.as_millis()).map_err(|_| {
        migration_failure(
            "system clock timestamp exceeds the SQLite integer range",
            None,
        )
    })
}

fn migration_failure(message: impl Into<String>, schema_version: Option<u32>) -> StorageError {
    StorageError::new(
        StorageErrorKind::Migration,
        StorageOperation::Migration,
        message,
        schema_version,
    )
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= u64::from(bytes[index]);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, OptionalExtension};

    use super::{
        LATEST_SCHEMA_VERSION, MIGRATION_V1_SQL, Migration, fnv1a64, run_migration_catalog,
        run_migrations,
    };
    use crate::storage::{StorageErrorKind, test_support::TestDatabasePath};

    const BAD_MIGRATION_SQL: &str = r"
CREATE TABLE should_rollback (
    id INTEGER PRIMARY KEY
) STRICT;
THIS IS NOT VALID SQL;
";

    #[test]
    fn compiled_catalog_matches_the_declared_latest_version() {
        assert_eq!(
            super::MIGRATIONS.last().map(|migration| migration.version),
            Some(LATEST_SCHEMA_VERSION)
        );
    }

    #[test]
    fn empty_database_migrates_to_latest_and_reopens() {
        let test_path = TestDatabasePath::new("migration-latest");
        let mut connection = Connection::open(test_path.path()).expect("open temporary database");
        let first = run_migrations(&mut connection).expect("empty database migrates");
        assert_eq!(first.schema_version, LATEST_SCHEMA_VERSION);
        assert_eq!(
            first.records.len(),
            usize::try_from(LATEST_SCHEMA_VERSION).expect("schema version fits usize")
        );
        drop(connection);

        let mut reopened = Connection::open(test_path.path()).expect("reopen temporary database");
        let second = run_migrations(&mut reopened).expect("latest database reopens");
        assert_eq!(second, first);
    }

    #[test]
    fn failed_migration_rolls_back_only_that_version() {
        let test_path = TestDatabasePath::new("migration-rollback");
        let mut connection = Connection::open(test_path.path()).expect("open temporary database");
        let migrations = [
            Migration {
                version: 1,
                sql: MIGRATION_V1_SQL,
            },
            Migration {
                version: 2,
                sql: BAD_MIGRATION_SQL,
            },
        ];

        let error = run_migration_catalog(&mut connection, &migrations)
            .expect_err("invalid migration must fail");
        assert_eq!(error.kind, StorageErrorKind::Migration);

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read user version");
        assert_eq!(version, 1);
        let rolled_back: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'should_rollback'",
                [],
                |row| row.get(0),
            )
            .optional()
            .expect("query schema");
        assert_eq!(rolled_back, None);
        let migration_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("count migration rows");
        assert_eq!(migration_count, 1);
    }

    #[test]
    fn checksum_mismatch_and_newer_schema_are_rejected() {
        let test_path = TestDatabasePath::new("migration-compatibility");
        let mut connection = Connection::open(test_path.path()).expect("open temporary database");
        run_migrations(&mut connection).expect("create latest schema");
        connection
            .execute(
                "UPDATE schema_migrations SET checksum = 'tampered' WHERE version = 1",
                [],
            )
            .expect("tamper checksum");
        let checksum_error =
            run_migrations(&mut connection).expect_err("checksum mismatch must fail");
        assert_eq!(checksum_error.kind, StorageErrorKind::Migration);

        connection
            .execute(
                "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
                [format!(
                    "fnv1a64:{:016x}",
                    fnv1a64(MIGRATION_V1_SQL.as_bytes())
                )],
            )
            .expect("restore checksum");
        connection
            .pragma_update(None, "user_version", LATEST_SCHEMA_VERSION + 1)
            .expect("set newer schema");
        let newer_error = run_migrations(&mut connection).expect_err("newer schema must fail");
        assert_eq!(newer_error.kind, StorageErrorKind::Migration);
    }
}
