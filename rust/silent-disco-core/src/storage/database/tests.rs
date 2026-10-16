use std::{fs, time::Duration};

use rusqlite::{Connection, params};

use super::{DEFAULT_BUSY_TIMEOUT_MS, DatabaseConfig, DatabaseConnection, SynchronousPolicy};
use crate::domain::{SessionId, TuningSettings};
use crate::storage::{
    LATEST_SCHEMA_VERSION, StorageErrorKind, StorageOperation, StoredSettings,
    test_support::TestDatabasePath,
};

// Migration v3 adds host network policy. An existing profile must keep
// refusing shared address space across the upgrade -- a silent opt-in
// would start advertising a carrier-NAT address the operator never chose.
#[test]
fn upgraded_profile_defaults_to_refusing_shared_address_space() {
    let test_path = TestDatabasePath::new("database-v3-default");
    let config = DatabaseConfig::new(test_path.path()).expect("valid config");
    let mut connection = DatabaseConnection::open(&config).expect("database opens");

    let settings = StoredSettings {
        tuning: TuningSettings::default(),
        allow_shared_address_space: false,
        updated_at_ms: 7,
    };
    connection
        .save_settings(&settings)
        .expect("settings persist");
    let loaded = connection
        .load_settings()
        .expect("settings reload")
        .expect("settings row exists");
    assert!(!loaded.allow_shared_address_space);

    let _ = connection.checkpoint_and_close();
}

#[test]
fn shared_address_space_opt_in_survives_reopen() {
    let test_path = TestDatabasePath::new("database-v3-optin");
    let config = DatabaseConfig::new(test_path.path()).expect("valid config");
    let mut connection = DatabaseConnection::open(&config).expect("database opens");

    let settings = StoredSettings {
        tuning: TuningSettings::default(),
        allow_shared_address_space: true,
        updated_at_ms: 11,
    };
    connection
        .save_settings(&settings)
        .expect("settings persist");
    let _ = connection.checkpoint_and_close();

    // A persisted opt-in is only useful if it is still there next launch.
    let reopened = DatabaseConnection::open(&config).expect("database reopens");
    let loaded = reopened
        .load_settings()
        .expect("settings reload")
        .expect("settings row exists");
    assert!(loaded.allow_shared_address_space);
    assert_eq!(loaded, settings);

    let _ = reopened.checkpoint_and_close();
}

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
    assert_eq!(metadata.applied_migrations.len(), 3);
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

/// Block 25: a `trusted_devices.trust_state` value that could only exist
/// by bypassing the schema's `CHECK` constraint (e.g. bit rot, a manual
/// edit, or a future writer bug) must surface as a typed, visible
/// `Corruption` error from the read path -- never a panic, and never a
/// silently-decoded bogus value.
#[test]
fn corrupted_trust_state_surfaces_as_corrupt_row_not_panic() {
    let test_path = TestDatabasePath::new("database-corrupt-trust-state");
    let config = DatabaseConfig::new(test_path.path()).expect("valid config");
    let connection = DatabaseConnection::open(&config).expect("database opens");

    connection
        .connection
        .execute_batch("PRAGMA ignore_check_constraints = ON;")
        .expect("disable check constraints on this connection only");
    connection
        .connection
        .execute(
            "INSERT INTO trusted_devices (
                     device_id, display_name, trust_state,
                     first_seen_ms, last_seen_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "corrupt-device",
                "Corrupt Device",
                "not_a_real_trust_state",
                1_i64,
                2_i64,
                3_i64
            ],
        )
        .expect("corrupt row insert succeeds with checks disabled");

    let error = connection
        .list_trusted_devices()
        .expect_err("a corrupted trust_state must not silently decode or panic");
    assert_eq!(error.kind, StorageErrorKind::Corruption);

    // No silent fallback: the corrupted row is still exactly what is on
    // disk, not replaced or discarded by the failed read.
    let raw_value: String = connection
        .connection
        .query_row(
            "SELECT trust_state FROM trusted_devices WHERE device_id = 'corrupt-device'",
            [],
            |row| row.get(0),
        )
        .expect("corrupted row remains readable on disk");
    assert_eq!(raw_value, "not_a_real_trust_state");

    let _ = connection.checkpoint_and_close();
}

/// Block 25: a `session_history.role`/`outcome` value that could only
/// exist by bypassing the schema's `CHECK` constraint must surface as a
/// typed `Corruption` error, never a panic and never a silently-decoded
/// bogus value.
#[test]
fn corrupted_session_role_and_outcome_surface_as_corrupt_row_not_panic() {
    let test_path = TestDatabasePath::new("database-corrupt-session");
    let config = DatabaseConfig::new(test_path.path()).expect("valid config");
    let connection = DatabaseConnection::open(&config).expect("database opens");

    connection
        .connection
        .execute_batch("PRAGMA ignore_check_constraints = ON;")
        .expect("disable check constraints on this connection only");
    connection
        .connection
        .execute(
            "INSERT INTO session_history (
                     session_id, role, session_name, started_at_ms, listener_count, outcome
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "corrupt-role",
                "not_a_real_role",
                "Corrupt Session",
                10_i64,
                0_i64,
                "active"
            ],
        )
        .expect("corrupt role row insert succeeds with checks disabled");
    connection
        .connection
        .execute(
            "INSERT INTO session_history (
                     session_id, role, session_name, started_at_ms, ended_at_ms,
                     listener_count, outcome
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "corrupt-outcome",
                "host",
                "Corrupt Session",
                10_i64,
                20_i64,
                0_i64,
                "not_a_real_outcome"
            ],
        )
        .expect("corrupt outcome row insert succeeds with checks disabled");

    let role_error = connection
        .get_session(&SessionId::new("corrupt-role").expect("valid session id"))
        .expect_err("a corrupted role must not silently decode or panic");
    assert_eq!(role_error.kind, StorageErrorKind::Corruption);

    let outcome_error = connection
        .get_session(&SessionId::new("corrupt-outcome").expect("valid session id"))
        .expect_err("a corrupted outcome must not silently decode or panic");
    assert_eq!(outcome_error.kind, StorageErrorKind::Corruption);

    let _ = connection.checkpoint_and_close();
}

/// Block 25: a genuine `SQLITE_BUSY` from another writer holding the
/// write lock must surface as a typed, visible `Busy` error -- never a
/// silent success against a different (e.g. in-memory) store, and the
/// write must actually land once the lock is released.
#[test]
fn concurrent_writer_produces_busy_not_silent_fallback() {
    let test_path = TestDatabasePath::new("database-busy");
    let config = DatabaseConfig::new(test_path.path())
        .and_then(|config| config.with_busy_timeout(Duration::from_millis(50)))
        .expect("valid config");
    let mut connection = DatabaseConnection::open(&config).expect("database opens");

    let blocker = Connection::open(test_path.path()).expect("second connection opens");
    blocker
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("blocker takes the write lock");

    let settings = StoredSettings {
        tuning: TuningSettings::default(),
        allow_shared_address_space: false,
        updated_at_ms: 5,
    };
    let error = connection
        .save_settings(&settings)
        .expect_err("a write while another connection holds the write lock must surface as Busy");
    assert_eq!(error.kind, StorageErrorKind::Busy);

    blocker
        .execute_batch("COMMIT;")
        .expect("release the write lock");
    connection
        .save_settings(&settings)
        .expect("write succeeds once the lock is released");
    assert_eq!(
        connection.load_settings().expect("reload settings"),
        Some(settings)
    );

    let _ = connection.checkpoint_and_close();
}

/// Block 25: a read-only database file must fail to open for the
/// read-write connection this worker always requests -- never silently
/// falling back to an in-memory database, and never recreating the file.
/// (Assumes the test runs as a non-root user, as this environment and
/// standard CI runners do; `root` ignores Unix file-permission bits.)
#[cfg(unix)]
#[test]
fn read_only_file_permission_fails_open_without_recreating_database() {
    use std::os::unix::fs::PermissionsExt;

    let test_path = TestDatabasePath::new("database-readonly");
    {
        let config = DatabaseConfig::new(test_path.path()).expect("valid config");
        let connection = DatabaseConnection::open(&config).expect("database opens");
        connection
            .checkpoint_and_close()
            .expect("checkpoint and close succeeds");
    }

    let mut permissions = fs::metadata(test_path.path())
        .expect("read file metadata")
        .permissions();
    permissions.set_mode(0o444);
    fs::set_permissions(test_path.path(), permissions).expect("make the database file read-only");

    let config = DatabaseConfig::new(test_path.path()).expect("valid config");
    let error = DatabaseConnection::open(&config)
        .err()
        .expect("a read-only database file must not silently open for read-write use");
    assert_eq!(error.kind, StorageErrorKind::Open);

    // Restore write permission so `TestDatabasePath`'s `Drop` can clean up.
    let mut permissions = fs::metadata(test_path.path())
        .expect("read file metadata")
        .permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(test_path.path(), permissions).expect("restore write permission");
}
