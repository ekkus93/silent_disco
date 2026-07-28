mod client;
mod lifecycle;
#[cfg(test)]
mod tests;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
};

use crate::domain::{DeviceId, SessionId};

use super::{
    database::{DatabaseCheckpoint, DatabaseConfig, DatabaseConnection, DatabaseMetadata},
    error::{StorageError, StorageErrorKind, StorageOperation},
    models::{
        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
        LegacyAndroidImport, LegacyImportOutcome, SessionEnd, SessionHistory, SessionStart,
        SessionUpdate, StoredSettings, TrustedDevice,
    },
};

type DatabaseReply<T> = SyncSender<Result<T, StorageError>>;

enum DatabaseCommand {
    ReadMetadata {
        reply: DatabaseReply<DatabaseMetadata>,
    },
    ImportLegacyAndroid {
        value: LegacyAndroidImport,
        reply: DatabaseReply<LegacyImportOutcome>,
    },
    LoadSettings {
        reply: DatabaseReply<Option<StoredSettings>>,
    },
    SaveSettings {
        settings: StoredSettings,
        reply: DatabaseReply<()>,
    },
    ListTrustedDevices {
        reply: DatabaseReply<Vec<TrustedDevice>>,
    },
    GetTrustedDevice {
        device_id: DeviceId,
        reply: DatabaseReply<Option<TrustedDevice>>,
    },
    UpsertTrustedDevice {
        device: TrustedDevice,
        reply: DatabaseReply<()>,
    },
    DeleteTrustedDevice {
        device_id: DeviceId,
        reply: DatabaseReply<bool>,
    },
    BeginSession {
        session: SessionStart,
        reply: DatabaseReply<()>,
    },
    UpdateSession {
        update: SessionUpdate,
        reply: DatabaseReply<bool>,
    },
    EndSession {
        end: SessionEnd,
        reply: DatabaseReply<bool>,
    },
    GetSession {
        session_id: SessionId,
        reply: DatabaseReply<Option<SessionHistory>>,
    },
    InsertDiagnosticRun {
        run: DiagnosticRunSummary,
        reply: DatabaseReply<()>,
    },
    QueryDiagnosticRuns {
        query: DiagnosticQuery,
        reply: DatabaseReply<Vec<DiagnosticRunSummary>>,
    },
    ExportDiagnosticRuns {
        request: DiagnosticExportRequest,
        reply: DatabaseReply<DiagnosticExport>,
    },
    Checkpoint {
        reply: DatabaseReply<DatabaseCheckpoint>,
    },
    Shutdown {
        reply: DatabaseReply<()>,
    },
    #[cfg(test)]
    BlockForQueueTest {
        entered: SyncSender<()>,
        release: Receiver<()>,
    },
}

/// Cloneable typed request endpoint for the dedicated database worker.
///
/// Calls are blocking control-plane operations and must never be made from the
/// real-time audio callback, playback timing loop, packet callback, or UI thread.
#[derive(Clone)]
pub struct DatabaseClient {
    sender: SyncSender<DatabaseCommand>,
    accepting_requests: Arc<AtomicBool>,
    schema_version: Arc<AtomicU32>,
}

/// Lifecycle owner for one dedicated `SQLite` worker thread.
///
/// The worker must be stopped and joined explicitly. `Drop` is a fail-visible
/// safety net that performs the same shutdown and panics if clean shutdown is
/// impossible, preventing an accidentally detached database thread.
#[must_use = "database workers must be stopped and joined"]
pub struct DatabaseWorker {
    client: DatabaseClient,
    initial_metadata: DatabaseMetadata,
    join_handle: Option<JoinHandle<Result<(), StorageError>>>,
    stop_result: Option<Result<(), StorageError>>,
}

fn run_database_worker(
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
        if process_command(command, &mut connection, schema_version)? {
            return Ok(());
        }
    }
}

fn process_command(
    command: DatabaseCommand,
    connection: &mut Option<DatabaseConnection>,
    schema_version: &Arc<AtomicU32>,
) -> Result<bool, StorageError> {
    let version = schema_version.load(Ordering::Acquire);
    match command {
        DatabaseCommand::ReadMetadata { reply } => {
            process_read_metadata(&reply, connection, version)?;
        }
        DatabaseCommand::ImportLegacyAndroid { value, reply } => {
            process_import_legacy_android(&reply, &value, connection, version)?;
        }
        DatabaseCommand::LoadSettings { reply } => {
            process_load_settings(&reply, connection, version)?;
        }
        DatabaseCommand::SaveSettings { settings, reply } => {
            process_save_settings(&reply, &settings, connection, version)?;
        }
        DatabaseCommand::ListTrustedDevices { reply } => {
            process_list_trusted_devices(&reply, connection, version)?;
        }
        DatabaseCommand::GetTrustedDevice { device_id, reply } => {
            process_get_trusted_device(&reply, &device_id, connection, version)?;
        }
        DatabaseCommand::UpsertTrustedDevice { device, reply } => {
            process_upsert_trusted_device(&reply, &device, connection, version)?;
        }
        DatabaseCommand::DeleteTrustedDevice { device_id, reply } => {
            process_delete_trusted_device(&reply, &device_id, connection, version)?;
        }
        DatabaseCommand::BeginSession { session, reply } => {
            process_begin_session(&reply, &session, connection, version)?;
        }
        DatabaseCommand::UpdateSession { update, reply } => {
            process_update_session(&reply, &update, connection, version)?;
        }
        DatabaseCommand::EndSession { end, reply } => {
            process_end_session(&reply, &end, connection, version)?;
        }
        DatabaseCommand::GetSession { session_id, reply } => {
            process_get_session(&reply, &session_id, connection, version)?;
        }
        DatabaseCommand::InsertDiagnosticRun { run, reply } => {
            process_insert_diagnostic_run(&reply, &run, connection, version)?;
        }
        DatabaseCommand::QueryDiagnosticRuns { query, reply } => {
            process_query_diagnostic_runs(&reply, &query, connection, version)?;
        }
        DatabaseCommand::ExportDiagnosticRuns { request, reply } => {
            process_export_diagnostic_runs(&reply, &request, connection, version)?;
        }
        DatabaseCommand::Checkpoint { reply } => {
            process_checkpoint(&reply, connection, version)?;
        }
        DatabaseCommand::Shutdown { reply } => {
            return process_shutdown(&reply, connection, version);
        }
        #[cfg(test)]
        DatabaseCommand::BlockForQueueTest { entered, release } => {
            process_queue_test_barrier(&entered, &release, connection, version)?;
        }
    }
    Ok(false)
}

fn process_read_metadata(
    reply: &DatabaseReply<DatabaseMetadata>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(
        connection.as_ref(),
        StorageOperation::ReadMetadata,
        schema_version,
    )
    .map(DatabaseConnection::metadata);
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::ReadMetadata,
        schema_version,
    )
}

fn process_import_legacy_android(
    reply: &DatabaseReply<LegacyImportOutcome>,
    value: &LegacyAndroidImport,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.import_legacy_android(value));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_load_settings(
    reply: &DatabaseReply<Option<StoredSettings>>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(DatabaseConnection::load_settings);
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Query,
        schema_version,
    )
}

fn process_save_settings(
    reply: &DatabaseReply<()>,
    settings: &StoredSettings,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.save_settings(settings));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_list_trusted_devices(
    reply: &DatabaseReply<Vec<TrustedDevice>>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(DatabaseConnection::list_trusted_devices);
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Query,
        schema_version,
    )
}

fn process_get_trusted_device(
    reply: &DatabaseReply<Option<TrustedDevice>>,
    device_id: &DeviceId,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(|database| database.get_trusted_device(device_id));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Query,
        schema_version,
    )
}

fn process_upsert_trusted_device(
    reply: &DatabaseReply<()>,
    device: &TrustedDevice,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.upsert_trusted_device(device));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_delete_trusted_device(
    reply: &DatabaseReply<bool>,
    device_id: &DeviceId,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.delete_trusted_device(device_id));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_begin_session(
    reply: &DatabaseReply<()>,
    session: &SessionStart,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.begin_session(session));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_update_session(
    reply: &DatabaseReply<bool>,
    update: &SessionUpdate,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.update_session(update));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_end_session(
    reply: &DatabaseReply<bool>,
    end: &SessionEnd,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.end_session(end));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_get_session(
    reply: &DatabaseReply<Option<SessionHistory>>,
    session_id: &SessionId,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(|database| database.get_session(session_id));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Query,
        schema_version,
    )
}

fn process_insert_diagnostic_run(
    reply: &DatabaseReply<()>,
    run: &DiagnosticRunSummary,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_mut(connection, StorageOperation::Transaction, schema_version)
        .and_then(|database| database.insert_diagnostic_run(run));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Transaction,
        schema_version,
    )
}

fn process_query_diagnostic_runs(
    reply: &DatabaseReply<Vec<DiagnosticRunSummary>>,
    query: &DiagnosticQuery,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(|database| database.query_diagnostic_runs(query));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Query,
        schema_version,
    )
}

fn process_export_diagnostic_runs(
    reply: &DatabaseReply<DiagnosticExport>,
    request: &DiagnosticExportRequest,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(|database| database.export_diagnostic_runs(request));
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Query,
        schema_version,
    )
}

fn process_checkpoint(
    reply: &DatabaseReply<DatabaseCheckpoint>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(
        connection.as_ref(),
        StorageOperation::Checkpoint,
        schema_version,
    )
    .and_then(DatabaseConnection::checkpoint);
    send_reply(
        reply,
        result,
        connection,
        StorageOperation::Checkpoint,
        schema_version,
    )
}

fn process_shutdown(
    reply: &DatabaseReply<()>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<bool, StorageError> {
    let result = connection.take().map_or_else(
        || {
            Err(StorageError::worker_stopped(
                StorageOperation::StopWorker,
                Some(schema_version),
            ))
        },
        DatabaseConnection::checkpoint_and_close,
    );
    if reply.send(result.clone()).is_err() {
        return result.map_or_else(Err, |()| {
            Err(StorageError::reply_disconnected(
                StorageOperation::StopWorker,
                schema_version,
            ))
        });
    }
    result.map(|()| true)
}

#[cfg(test)]
fn process_queue_test_barrier(
    entered: &SyncSender<()>,
    release: &Receiver<()>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    if entered.send(()).is_err() || release.recv().is_err() {
        return close_after_reply_failure(
            connection.take(),
            None,
            StorageOperation::Query,
            schema_version,
        );
    }
    Ok(())
}

fn connection_ref(
    connection: Option<&DatabaseConnection>,
    operation: StorageOperation,
    schema_version: u32,
) -> Result<&DatabaseConnection, StorageError> {
    connection.ok_or_else(|| StorageError::worker_stopped(operation, Some(schema_version)))
}

fn connection_mut(
    connection: &mut Option<DatabaseConnection>,
    operation: StorageOperation,
    schema_version: u32,
) -> Result<&mut DatabaseConnection, StorageError> {
    connection
        .as_mut()
        .ok_or_else(|| StorageError::worker_stopped(operation, Some(schema_version)))
}

fn send_reply<T>(
    reply: &DatabaseReply<T>,
    result: Result<T, StorageError>,
    connection: &mut Option<DatabaseConnection>,
    operation: StorageOperation,
    schema_version: u32,
) -> Result<(), StorageError> {
    match reply.send(result) {
        Ok(()) => Ok(()),
        Err(error) => {
            close_after_reply_failure(connection.take(), error.0.err(), operation, schema_version)
        }
    }
}

fn close_after_channel_disconnect(
    connection: Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    match connection {
        Some(connection) => connection.checkpoint_and_close(),
        None => Err(StorageError::worker_stopped(
            StorageOperation::CloseDatabase,
            Some(schema_version),
        )),
    }
}

fn close_after_reply_failure(
    connection: Option<DatabaseConnection>,
    database_error: Option<StorageError>,
    operation: StorageOperation,
    schema_version: u32,
) -> Result<(), StorageError> {
    let close_result = close_after_channel_disconnect(connection, schema_version);
    match (database_error, close_result) {
        (_, Err(close_error)) => Err(close_error),
        (Some(error), Ok(())) => Err(error),
        (None, Ok(())) => Err(StorageError::reply_disconnected(operation, schema_version)),
    }
}
