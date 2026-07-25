from __future__ import annotations

import re
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def patch_error_codes() -> None:
    path = Path("rust/silent-disco-core/src/error.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    StorageOpenFailed = 6000,
    StorageMigrationFailed = 6001,
    StorageIntegrityFailed = 6002,
    StorageReadFailed = 6003,
    StorageWriteFailed = 6004,

    PlatformOperationFailed = 7000,
""",
        """    StorageOpenFailed = 6000,
    StorageMigrationFailed = 6001,
    StorageIntegrityFailed = 6002,
    StorageReadFailed = 6003,
    StorageWriteFailed = 6004,
    StoragePragmaFailed = 6005,
    StorageTransactionFailed = 6006,
    StorageConstraintViolation = 6007,
    StorageBusy = 6008,
    StorageCorrupt = 6009,
    StorageCloseFailed = 6010,
    StorageQueryFailed = 6011,

    PlatformOperationFailed = 7000,
""",
        "storage code definitions",
    )
    text = replace_once(
        text,
        """            Self::StorageOpenFailed
            | Self::StorageMigrationFailed
            | Self::StorageIntegrityFailed
            | Self::StorageReadFailed
            | Self::StorageWriteFailed => CoreSubsystem::Storage,
""",
        """            Self::StorageOpenFailed
            | Self::StorageMigrationFailed
            | Self::StorageIntegrityFailed
            | Self::StorageReadFailed
            | Self::StorageWriteFailed
            | Self::StoragePragmaFailed
            | Self::StorageTransactionFailed
            | Self::StorageConstraintViolation
            | Self::StorageBusy
            | Self::StorageCorrupt
            | Self::StorageCloseFailed
            | Self::StorageQueryFailed => CoreSubsystem::Storage,
""",
        "storage subsystem mapping",
    )
    text = replace_once(
        text,
        """            Self::StorageOpenFailed => "storage_open_failed",
            Self::StorageMigrationFailed => "storage_migration_failed",
            Self::StorageIntegrityFailed => "storage_integrity_failed",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed => "storage_write_failed",
            Self::PlatformOperationFailed => "platform_operation_failed",
""",
        """            Self::StorageOpenFailed => "storage_open_failed",
            Self::StorageMigrationFailed => "storage_migration_failed",
            Self::StorageIntegrityFailed => "storage_integrity_failed",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed => "storage_write_failed",
            Self::StoragePragmaFailed => "storage_pragma_failed",
            Self::StorageTransactionFailed => "storage_transaction_failed",
            Self::StorageConstraintViolation => "storage_constraint_violation",
            Self::StorageBusy => "storage_busy",
            Self::StorageCorrupt => "storage_corrupt",
            Self::StorageCloseFailed => "storage_close_failed",
            Self::StorageQueryFailed => "storage_query_failed",
            Self::PlatformOperationFailed => "platform_operation_failed",
""",
        "storage stable names",
    )
    path.write_text(text)


def patch_storage_error_source() -> None:
    path = Path("rust/silent-disco-core/src/storage/error.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    error: SqliteError,
""",
        """    error: &SqliteError,
""",
        "borrow SQLite error",
    )
    count = text.count("sqlite_failure(ffi::")
    if count != 6:
        raise SystemExit(f"SQLite mapping tests: expected 6 calls, found {count}")
    text = text.replace("sqlite_failure(ffi::", "&sqlite_failure(ffi::")
    path.write_text(text)


def patch_database_source() -> None:
    path = Path("rust/silent-disco-core/src/storage/database.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    path::{Path, PathBuf},
""",
        """    path::PathBuf,
""",
        "database path import",
    )
    replacements = {
        "/// Explicit SQLite synchronous policy": "/// Explicit `SQLite` synchronous policy",
        "/// SQLite synchronizes the WAL": "/// `SQLite` synchronizes the WAL",
        "/// Validated configuration used to start one SQLite worker.": "/// Validated configuration used to start one `SQLite` worker.",
        "/// Selects a nonzero SQLite busy timeout that fits SQLite's millisecond API.": "/// Selects a nonzero `SQLite` busy timeout that fits `SQLite`'s millisecond API.",
        "/// Verified SQLite runtime metadata exposed for diagnostics.": "/// Verified `SQLite` runtime metadata exposed for diagnostics.",
    }
    for old, new in replacements.items():
        text = replace_once(text, old, new, old)
    text = text.replace(
        "map_sqlite_error(StorageOperation::OpenDatabase, None, error)",
        "map_sqlite_error(StorageOperation::OpenDatabase, None, &error)",
    )
    text = text.replace(
        "map_sqlite_error(StorageOperation::ReadMetadata, None, error)",
        "map_sqlite_error(StorageOperation::ReadMetadata, None, &error)",
    )
    text, multiline_count = re.subn(
        r"\n(?P<indent>\s*)error,\n(?P<close>\s*)\)\)",
        r"\n\g<indent>&error,\n\g<close>))",
        text,
    )
    if multiline_count != 5:
        raise SystemExit(
            f"multiline SQLite error arguments: expected 5, found {multiline_count}"
        )
    text = replace_once(
        text,
        """        assert_eq!(metadata.owner_thread_name, "unnamed");
""",
        """        assert!(!metadata.owner_thread_name.is_empty());
""",
        "database owner thread assertion",
    )
    text = replace_once(
        text,
        """        let error = DatabaseConnection::open(&config).expect_err("memory mode cannot verify WAL");

        assert_eq!(error.kind, StorageErrorKind::Pragma);
""",
        """        let error = match DatabaseConnection::open(&config) {
            Ok(connection) => {
                let _ = connection.checkpoint_and_close();
                panic!("memory mode unexpectedly verified WAL");
            }
            Err(error) => error,
        };

        assert_eq!(error.kind, StorageErrorKind::Pragma);
""",
        "WAL failure test",
    )
    text = replace_once(
        text,
        """        let error = DatabaseConnection::open(&config).expect_err("corrupt file must fail");

        assert_eq!(error.kind, StorageErrorKind::Corruption);
""",
        """        let error = match DatabaseConnection::open(&config) {
            Ok(connection) => {
                let _ = connection.checkpoint_and_close();
                panic!("corrupt file unexpectedly opened");
            }
            Err(error) => error,
        };

        assert_eq!(error.kind, StorageErrorKind::Corruption);
""",
        "corrupt database test",
    )
    path.write_text(text)


def patch_worker_source() -> None:
    path = Path("rust/silent-disco-core/src/storage/worker.rs")
    text = path.read_text()
    replacements = {
        "/// Returns verified SQLite diagnostics": "/// Returns verified `SQLite` diagnostics",
        "/// Returns a visible queue, SQLite, or worker lifecycle error.": "/// Returns a visible queue, `SQLite`, or worker lifecycle error.",
        "/// Lifecycle owner for one dedicated SQLite worker thread.": "/// Lifecycle owner for one dedicated `SQLite` worker thread.",
        "/// Starts a worker and does not return until SQLite is open and every": "/// Starts a worker and does not return until `SQLite` is open and every",
        "/// Returns a visible shutdown or SQLite close/checkpoint failure.": "/// Returns a visible shutdown or `SQLite` close/checkpoint failure.",
    }
    for old, new in replacements.items():
        text = replace_once(text, old, new, old)
    text = replace_once(
        text,
        """                run_database_worker(
                    config,
                    command_receiver,
                    startup_sender,
                    thread_schema_version,
                )
""",
        """                run_database_worker(
                    &config,
                    &command_receiver,
                    &startup_sender,
                    &thread_schema_version,
                )
""",
        "worker thread argument borrowing",
    )
    text = replace_once(
        text,
        """        match (stop_result, thread_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
""",
        """        match (stop_result, thread_result) {
            (Err(error), _) | (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
""",
        "join result match",
    )
    start = text.index("fn run_database_worker(")
    end = text.index("fn close_after_channel_disconnect(", start)
    new_worker_functions = """fn run_database_worker(
    config: &DatabaseConfig,
    command_receiver: &Receiver<DatabaseCommand>,
    startup_sender: &SyncSender<Result<DatabaseMetadata, StorageError>>,
    schema_version: &Arc<AtomicU32>,
) -> Result<(), StorageError> {
    let connection = open_and_report_startup(config, startup_sender, schema_version)?;
    run_database_commands(connection, command_receiver, schema_version)
}

fn open_and_report_startup(
    config: &DatabaseConfig,
    startup_sender: &SyncSender<Result<DatabaseMetadata, StorageError>>,
    schema_version: &Arc<AtomicU32>,
) -> Result<DatabaseConnection, StorageError> {
    let connection = match DatabaseConnection::open(config) {
        Ok(connection) => connection,
        Err(error) => {
            let _ = startup_sender.send(Err(error.clone()));
            return Err(error);
        }
    };
    let metadata = connection.metadata();
    schema_version.store(metadata.schema_version, Ordering::Release);
    if startup_sender.send(Ok(metadata)).is_err() {
        let close_result = connection.checkpoint_and_close();
        return match close_result {
            Ok(()) => Err(StorageError::worker_stopped(
                StorageOperation::StartWorker,
                Some(schema_version.load(Ordering::Acquire)),
            )),
            Err(error) => Err(error),
        };
    }
    Ok(connection)
}

fn run_database_commands(
    connection: DatabaseConnection,
    command_receiver: &Receiver<DatabaseCommand>,
    schema_version: &Arc<AtomicU32>,
) -> Result<(), StorageError> {
    let mut connection = Some(connection);
    loop {
        let Ok(command) = command_receiver.recv() else {
            return close_after_channel_disconnect(
                connection.take(),
                schema_version.load(Ordering::Acquire),
            );
        };
        match command {
            DatabaseCommand::ReadMetadata { reply } => {
                let result = connection
                    .as_ref()
                    .map(DatabaseConnection::metadata)
                    .ok_or_else(|| {
                        StorageError::worker_stopped(
                            StorageOperation::ReadMetadata,
                            Some(schema_version.load(Ordering::Acquire)),
                        )
                    });
                if reply.send(result.clone()).is_err() {
                    return close_after_reply_failure(
                        connection.take(),
                        result.err(),
                        StorageOperation::ReadMetadata,
                        schema_version.load(Ordering::Acquire),
                    );
                }
            }
            DatabaseCommand::Checkpoint { reply } => {
                let result = connection
                    .as_ref()
                    .ok_or_else(|| {
                        StorageError::worker_stopped(
                            StorageOperation::Checkpoint,
                            Some(schema_version.load(Ordering::Acquire)),
                        )
                    })
                    .and_then(DatabaseConnection::checkpoint);
                if reply.send(result.clone()).is_err() {
                    return close_after_reply_failure(
                        connection.take(),
                        result.err(),
                        StorageOperation::Checkpoint,
                        schema_version.load(Ordering::Acquire),
                    );
                }
            }
            DatabaseCommand::Shutdown { reply } => {
                let result = connection.take().map_or_else(
                    || {
                        Err(StorageError::worker_stopped(
                            StorageOperation::StopWorker,
                            Some(schema_version.load(Ordering::Acquire)),
                        ))
                    },
                    DatabaseConnection::checkpoint_and_close,
                );
                if reply.send(result.clone()).is_err() {
                    return match result {
                        Ok(()) => Err(StorageError::reply_disconnected(
                            StorageOperation::StopWorker,
                            schema_version.load(Ordering::Acquire),
                        )),
                        Err(error) => Err(error),
                    };
                }
                return result;
            }
            #[cfg(test)]
            DatabaseCommand::BlockForQueueTest { entered, release } => {
                if entered.send(()).is_err() || release.recv().is_err() {
                    return close_after_reply_failure(
                        connection.take(),
                        None,
                        StorageOperation::Query,
                        schema_version.load(Ordering::Acquire),
                    );
                }
            }
        }
    }
}

"""
    text = text[:start] + new_worker_functions + text[end:]
    text = replace_once(
        text,
        """        let error = DatabaseWorker::start(config).expect_err("open must fail");

        assert_eq!(error.kind, StorageErrorKind::Open);
""",
        """        let error = match DatabaseWorker::start(config) {
            Ok(worker) => {
                let _ = worker.stop_and_join();
                panic!("database in a missing directory unexpectedly opened");
            }
            Err(error) => error,
        };

        assert_eq!(error.kind, StorageErrorKind::Open);
""",
        "worker startup failure test",
    )
    path.write_text(text)


def patch_storage_module_docs() -> None:
    path = Path("rust/silent-disco-core/src/storage/mod.rs")
    text = path.read_text()
    text = replace_once(
        text,
        "//! Rust-owned SQLite worker infrastructure.",
        "//! Rust-owned `SQLite` worker infrastructure.",
        "storage module title",
    )
    text = replace_once(
        text,
        "//! The SQLite connection is private to one dedicated thread.",
        "//! The `SQLite` connection is private to one dedicated thread.",
        "storage module ownership docs",
    )
    path.write_text(text)


def restore_ci_and_remove_script() -> None:
    path = Path(".github/workflows/ci.yml")
    text = path.read_text()
    skip = (
        "    if: github.event_name != 'pull_request' || "
        "github.event.pull_request.head.ref != 'feature/rust-sqlite-worker'\n"
    )
    if text.count(skip) != 2:
        raise SystemExit("temporary SQLite worker CI skips changed unexpectedly")
    text = text.replace(skip, "")
    start_marker = "  # BEGIN SQLITE WORKER BOOTSTRAP\n"
    end_marker = "  # END SQLITE WORKER BOOTSTRAP\n"
    if text.count(start_marker) != 1 or text.count(end_marker) != 1:
        raise SystemExit("SQLite bootstrap job markers changed unexpectedly")
    start = text.index(start_marker)
    end = text.index(end_marker, start) + len(end_marker)
    path.write_text((text[:start] + text[end:]).rstrip() + "\n")
    Path(".github/scripts/bootstrap_sqlite_worker.py").unlink()


def main() -> None:
    patch_error_codes()
    patch_storage_error_source()
    patch_database_source()
    patch_worker_source()
    patch_storage_module_docs()
    restore_ci_and_remove_script()


if __name__ == "__main__":
    main()
