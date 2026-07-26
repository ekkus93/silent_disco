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
    legacy_import::{LegacyAndroidImport, LegacyImportOutcome},
    models::{
        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
        SessionEnd, SessionHistory, SessionStart, SessionUpdate, StoredSettings, TrustedDevice,
    },
};

type DatabaseReply<T> = SyncSender<Result<T, StorageError>>;

enum DatabaseCommand {
    ReadMetadata {
        reply: DatabaseReply<DatabaseMetadata>,
    },
    ImportLegacyAndroidData {
        import: LegacyAndroidImport,
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

impl DatabaseClient {
    /// Returns verified `SQLite` diagnostics from the worker-owned connection.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, query, or worker lifecycle error.
    pub fn metadata(&self) -> Result<DatabaseMetadata, StorageError> {
        self.request(StorageOperation::ReadMetadata, |reply| {
            DatabaseCommand::ReadMetadata { reply }
        })
    }

    /// Atomically imports the typed legacy Android persistence record once.
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
    ///
    /// # Errors
    ///
    /// Returns a visible queue, query, corruption, or worker lifecycle error.
    pub fn load_settings(&self) -> Result<Option<StoredSettings>, StorageError> {
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::LoadSettings { reply }
        })
    }

    /// Validates and transactionally saves the singleton settings row.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, transaction, constraint, or worker error.
    pub fn save_settings(&self, settings: &StoredSettings) -> Result<(), StorageError> {
        let settings = settings.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::SaveSettings { settings, reply }
        })
    }

    /// Lists trusted devices in deterministic display-name order.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, query, corruption, or worker lifecycle error.
    pub fn list_trusted_devices(&self) -> Result<Vec<TrustedDevice>, StorageError> {
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::ListTrustedDevices { reply }
        })
    }

    /// Loads one trusted device by validated identifier.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, query, corruption, or worker lifecycle error.
    pub fn get_trusted_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Option<TrustedDevice>, StorageError> {
        let device_id = device_id.clone();
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::GetTrustedDevice { device_id, reply }
        })
    }

    /// Validates and transactionally inserts or updates trusted-device metadata.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, transaction, constraint, or worker error.
    pub fn upsert_trusted_device(&self, device: &TrustedDevice) -> Result<(), StorageError> {
        let device = device.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::UpsertTrustedDevice { device, reply }
        })
    }

    /// Deletes one trusted-device row and reports whether it existed.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, transaction, or worker lifecycle error.
    pub fn delete_trusted_device(&self, device_id: &DeviceId) -> Result<bool, StorageError> {
        let device_id = device_id.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::DeleteTrustedDevice { device_id, reply }
        })
    }

    /// Begins a new active session-history record.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, transaction, constraint, or worker error.
    pub fn begin_session(&self, session: &SessionStart) -> Result<(), StorageError> {
        let session = session.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::BeginSession { session, reply }
        })
    }

    /// Updates mutable counters for an active session.
    ///
    /// Returns `false` when no active session has the requested identifier.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, transaction, or worker lifecycle error.
    pub fn update_session(&self, update: &SessionUpdate) -> Result<bool, StorageError> {
        let update = update.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::UpdateSession { update, reply }
        })
    }

    /// Ends one active session and reports whether an active row was updated.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, transaction, constraint, or worker error.
    pub fn end_session(&self, end: &SessionEnd) -> Result<bool, StorageError> {
        let end = end.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::EndSession { end, reply }
        })
    }

    /// Loads one session-history row by identifier.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, query, corruption, or worker lifecycle error.
    pub fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionHistory>, StorageError> {
        let session_id = session_id.clone();
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::GetSession { session_id, reply }
        })
    }

    /// Validates and transactionally inserts one summarized diagnostic run.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, transaction, constraint, or worker error.
    pub fn insert_diagnostic_run(&self, run: &DiagnosticRunSummary) -> Result<(), StorageError> {
        let run = run.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::InsertDiagnosticRun { run, reply }
        })
    }

    /// Queries a bounded set of diagnostic summaries.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, query, corruption, or worker error.
    pub fn query_diagnostic_runs(
        &self,
        query: &DiagnosticQuery,
    ) -> Result<Vec<DiagnosticRunSummary>, StorageError> {
        let query = query.clone();
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::QueryDiagnosticRuns { query, reply }
        })
    }

    /// Exports every diagnostic summary in deterministic chronological order.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, query, corruption, or worker lifecycle error.
    pub fn export_diagnostic_runs(
        &self,
        request: &DiagnosticExportRequest,
    ) -> Result<DiagnosticExport, StorageError> {
        let request = request.clone();
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::ExportDiagnosticRuns { request, reply }
        })
    }

    /// Requests a passive WAL checkpoint on the worker thread.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, `SQLite`, or worker lifecycle error.
    pub fn checkpoint(&self) -> Result<DatabaseCheckpoint, StorageError> {
        self.request(StorageOperation::Checkpoint, |reply| {
            DatabaseCommand::Checkpoint { reply }
        })
    }

    fn request<T>(
        &self,
        operation: StorageOperation,
        command: impl FnOnce(DatabaseReply<T>) -> DatabaseCommand,
    ) -> Result<T, StorageError> {
        if !self.accepting_requests.load(Ordering::Acquire) {
            return Err(StorageError::worker_stopped(
                operation,
                Some(self.schema_version.load(Ordering::Acquire)),
            ));
        }
        let (reply_sender, reply_receiver) = sync_channel(1);
        match self.sender.try_send(command(reply_sender)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(StorageError::queue_full(
                    operation,
                    self.schema_version.load(Ordering::Acquire),
                ));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(StorageError::worker_stopped(
                    operation,
                    Some(self.schema_version.load(Ordering::Acquire)),
                ));
            }
        }
        reply_receiver.recv().map_err(|_| {
            StorageError::reply_disconnected(operation, self.schema_version.load(Ordering::Acquire))
        })?
    }

    fn request_shutdown(&self) -> Result<(), StorageError> {
        if !self.accepting_requests.swap(false, Ordering::AcqRel) {
            return Err(StorageError::shutdown_in_progress());
        }
        let (reply_sender, reply_receiver) = sync_channel(1);
        self.sender
            .send(DatabaseCommand::Shutdown {
                reply: reply_sender,
            })
            .map_err(|_| {
                StorageError::worker_stopped(
                    StorageOperation::StopWorker,
                    Some(self.schema_version.load(Ordering::Acquire)),
                )
            })?;
        reply_receiver.recv().map_err(|_| {
            StorageError::reply_disconnected(
                StorageOperation::StopWorker,
                self.schema_version.load(Ordering::Acquire),
            )
        })?
    }
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

impl DatabaseWorker {
    /// Starts a worker and does not return until `SQLite` is open, migrations
    /// are complete, and every required connection setting has been verified.
    ///
    /// # Errors
    ///
    /// Returns a structured configuration, thread-start, open, pragma, migration,
    /// integrity, or corruption failure. Failed initialization produces no client.
    pub fn start(config: DatabaseConfig) -> Result<Self, StorageError> {
        config.validate()?;
        let (command_sender, command_receiver) = sync_channel(config.queue_capacity);
        let (startup_sender, startup_receiver) = sync_channel(1);
        let accepting_requests = Arc::new(AtomicBool::new(true));
        let schema_version = Arc::new(AtomicU32::new(0));
        let thread_schema_version = Arc::clone(&schema_version);
        let join_handle = thread::Builder::new()
            .name("silent-disco-database".into())
            .spawn(move || {
                run_database_worker(
                    &config,
                    &command_receiver,
                    &startup_sender,
                    &thread_schema_version,
                )
            })
            .map_err(|error| {
                StorageError::new(
                    StorageErrorKind::ThreadStart,
                    StorageOperation::StartWorker,
                    format!("failed to start database worker thread: {error}"),
                    None,
                )
            })?;

        match startup_receiver.recv() {
            Ok(Ok(metadata)) => Ok(Self {
                client: DatabaseClient {
                    sender: command_sender,
                    accepting_requests,
                    schema_version,
                },
                initial_metadata: metadata,
                join_handle: Some(join_handle),
                stop_result: None,
            }),
            Ok(Err(error)) => {
                let _ = join_handle.join();
                Err(error)
            }
            Err(_) => match join_handle.join() {
                Ok(Err(error)) => Err(error),
                Ok(Ok(())) => Err(StorageError::worker_stopped(
                    StorageOperation::StartWorker,
                    None,
                )),
                Err(_) => Err(StorageError::worker_panicked()),
            },
        }
    }

    #[must_use]
    pub fn client(&self) -> DatabaseClient {
        self.client.clone()
    }

    #[must_use]
    pub fn initial_metadata(&self) -> &DatabaseMetadata {
        &self.initial_metadata
    }

    /// Requests checkpoint, close, and worker termination. This does not detach
    /// the thread; call [`Self::join`] or [`Self::stop_and_join`] to consume it.
    ///
    /// # Errors
    ///
    /// Returns a visible shutdown or `SQLite` close/checkpoint failure.
    pub fn stop(&mut self) -> Result<(), StorageError> {
        if self.stop_result.is_some() {
            return Err(StorageError::shutdown_in_progress());
        }
        let result = self.client.request_shutdown();
        self.stop_result = Some(result.clone());
        result
    }

    /// Joins the worker thread, requesting stop first when necessary.
    ///
    /// # Errors
    ///
    /// Returns any stop, close, worker, or panic failure.
    pub fn join(mut self) -> Result<(), StorageError> {
        if self.stop_result.is_none() {
            let _ = self.stop();
        }
        self.finish_join()
    }

    /// Convenience lifecycle operation that performs both explicit phases.
    ///
    /// # Errors
    ///
    /// Returns any stop, close, worker, or panic failure.
    pub fn stop_and_join(mut self) -> Result<(), StorageError> {
        let _ = self.stop();
        self.finish_join()
    }

    fn finish_join(&mut self) -> Result<(), StorageError> {
        let stop_result = self.stop_result.clone().unwrap_or_else(|| {
            Err(StorageError::worker_stopped(
                StorageOperation::JoinWorker,
                Some(self.client.schema_version.load(Ordering::Acquire)),
            ))
        });
        let Some(join_handle) = self.join_handle.take() else {
            return stop_result;
        };
        let thread_result = join_handle
            .join()
            .map_err(|_| StorageError::worker_panicked())?;
        match (stop_result, thread_result) {
            (Err(error), _) | (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

impl Drop for DatabaseWorker {
    fn drop(&mut self) {
        if self.join_handle.is_none() {
            return;
        }
        if self.stop_result.is_none() {
            let _ = self.stop();
        }
        if let Err(error) = self.finish_join() {
            if thread::panicking() {
                std::process::abort();
            }
            panic!("database worker failed to shut down during drop: {error}");
        }
    }
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
        DatabaseCommand::ImportLegacyAndroidData { import, reply } => {
            process_import_legacy_android_data(&reply, &import, connection, version)?;
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

fn process_import_legacy_android_data(
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

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc::sync_channel},
        thread,
    };

    use super::{DatabaseCommand, DatabaseWorker};
    use crate::{
        domain::{AppRole, DeviceId, DiagnosticRunId, SessionId, TrustState, TuningSettings},
        storage::{
            DatabaseConfig, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
            SessionEnd, SessionOutcome, SessionStart, SessionUpdate, StorageErrorKind,
            StoredSettings, TrustedDevice, test_support::TestDatabasePath,
        },
    };

    #[test]
    fn one_worker_thread_owns_every_connection_operation() {
        let test_path = TestDatabasePath::new("worker-ownership");
        let config = DatabaseConfig::new(test_path.path())
            .and_then(|config| config.with_queue_capacity(16))
            .expect("valid worker config");
        let worker = DatabaseWorker::start(config).expect("worker starts");
        let expected_owner = worker.initial_metadata().owner_thread_id.clone();
        let client = Arc::new(worker.client());
        let mut calls = Vec::new();
        for _ in 0..8 {
            let client = Arc::clone(&client);
            calls.push(thread::spawn(move || {
                client.metadata().map(|metadata| metadata.owner_thread_id)
            }));
        }
        for call in calls {
            let owner = call
                .join()
                .expect("client thread does not panic")
                .expect("metadata succeeds");
            assert_eq!(owner, expected_owner);
        }
        worker.stop_and_join().expect("worker closes and joins");
    }

    #[test]
    fn full_queue_rejects_visibly_without_dropping_accepted_command() {
        let test_path = TestDatabasePath::new("worker-queue");
        let config = DatabaseConfig::new(test_path.path())
            .and_then(|config| config.with_queue_capacity(1))
            .expect("valid worker config");
        let worker = DatabaseWorker::start(config).expect("worker starts");
        let (entered_sender, entered_receiver) = sync_channel(1);
        let (release_sender, release_receiver) = sync_channel(1);
        worker
            .client
            .sender
            .send(DatabaseCommand::BlockForQueueTest {
                entered: entered_sender,
                release: release_receiver,
            })
            .expect("barrier command accepted");
        entered_receiver.recv().expect("worker entered barrier");

        let (queued_reply_sender, queued_reply_receiver) = sync_channel(1);
        worker
            .client
            .sender
            .try_send(DatabaseCommand::ReadMetadata {
                reply: queued_reply_sender,
            })
            .expect("one command fills queue");
        let overflow = worker
            .client
            .metadata()
            .expect_err("queue must reject overflow");
        assert_eq!(overflow.kind, StorageErrorKind::QueueFull);

        release_sender.send(()).expect("release worker");
        queued_reply_receiver
            .recv()
            .expect("accepted command receives reply")
            .expect("accepted command succeeds");
        worker.stop_and_join().expect("worker closes and joins");
    }

    #[test]
    fn typed_repositories_round_trip_domain_records() {
        let test_path = TestDatabasePath::new("worker-repositories");
        let worker = DatabaseWorker::start(
            DatabaseConfig::new(test_path.path()).expect("valid worker config"),
        )
        .expect("worker starts");
        let client = worker.client();
        assert_eq!(client.load_settings().expect("load settings"), None);

        let settings = StoredSettings {
            tuning: TuningSettings::default(),
            updated_at_ms: 100,
        };
        client.save_settings(&settings).expect("save settings");
        assert_eq!(
            client.load_settings().expect("reload settings"),
            Some(settings)
        );

        let device = sample_device();
        client
            .upsert_trusted_device(&device)
            .expect("upsert trusted device");
        assert_eq!(
            client
                .get_trusted_device(&device.device_id)
                .expect("get trusted device"),
            Some(device.clone())
        );
        assert_eq!(
            client.list_trusted_devices().expect("list trusted devices"),
            vec![device]
        );

        let session = sample_session_start();
        client.begin_session(&session).expect("begin session");
        assert!(
            client
                .update_session(&SessionUpdate {
                    session_id: session.session_id.clone(),
                    listener_count: 3,
                })
                .expect("update session")
        );
        assert!(
            client
                .end_session(&SessionEnd {
                    session_id: session.session_id.clone(),
                    ended_at_ms: 250,
                    listener_count: 3,
                    outcome: SessionOutcome::Completed,
                    failure_code: None,
                    failure_message: None,
                })
                .expect("end session")
        );

        let diagnostic = sample_diagnostic(&session.session_id);
        client
            .insert_diagnostic_run(&diagnostic)
            .expect("insert diagnostic run");
        let query = DiagnosticQuery {
            session_id: Some(session.session_id.clone()),
            limit: 10,
        };
        assert_eq!(
            client
                .query_diagnostic_runs(&query)
                .expect("query diagnostics"),
            vec![diagnostic.clone()]
        );
        let export = client
            .export_diagnostic_runs(&DiagnosticExportRequest {
                session_id: None,
                cursor: None,
                limit: 10,
            })
            .expect("export diagnostics");
        assert_eq!(export.runs, vec![diagnostic]);
        assert_eq!(export.next_cursor, None);
        worker.stop_and_join().expect("worker closes and joins");
    }

    #[test]
    fn diagnostic_export_is_bounded_and_cursor_paginated() {
        let test_path = TestDatabasePath::new("worker-export-pagination");
        let worker = DatabaseWorker::start(
            DatabaseConfig::new(test_path.path()).expect("valid worker config"),
        )
        .expect("worker starts");
        let client = worker.client();
        let session = sample_session_start();
        client.begin_session(&session).expect("begin session");

        for (suffix, started_at_ms) in [("a", 110), ("b", 120), ("c", 130)] {
            client
                .insert_diagnostic_run(&DiagnosticRunSummary {
                    run_id: DiagnosticRunId::new(format!("diagnostic-{suffix}"))
                        .expect("valid diagnostic identifier"),
                    session_id: Some(session.session_id.clone()),
                    started_at_ms,
                    ended_at_ms: Some(started_at_ms + 1),
                    summary_json: format!(r#"{{"run":"{suffix}"}}"#),
                })
                .expect("insert diagnostic run");
        }

        let first = client
            .export_diagnostic_runs(&DiagnosticExportRequest {
                session_id: Some(session.session_id.clone()),
                cursor: None,
                limit: 2,
            })
            .expect("first export page");
        assert_eq!(first.runs.len(), 2);
        let cursor = first.next_cursor.expect("more rows remain");

        let second = client
            .export_diagnostic_runs(&DiagnosticExportRequest {
                session_id: Some(session.session_id),
                cursor: Some(cursor),
                limit: 2,
            })
            .expect("second export page");
        assert_eq!(second.runs.len(), 1);
        assert_eq!(second.next_cursor, None);
        worker.stop_and_join().expect("worker closes and joins");
    }

    #[test]
    fn duplicate_session_maps_to_constraint_violation() {
        let test_path = TestDatabasePath::new("worker-constraint");
        let worker = DatabaseWorker::start(
            DatabaseConfig::new(test_path.path()).expect("valid worker config"),
        )
        .expect("worker starts");
        let client = worker.client();
        let session = sample_session_start();
        client
            .begin_session(&session)
            .expect("first insert succeeds");
        let error = client
            .begin_session(&session)
            .expect_err("duplicate primary key must fail");
        assert_eq!(error.kind, StorageErrorKind::Constraint);
        worker.stop_and_join().expect("worker closes and joins");
    }

    #[test]
    fn queued_write_order_is_serialized_by_the_worker() {
        let test_path = TestDatabasePath::new("worker-ordering");
        let config = DatabaseConfig::new(test_path.path())
            .and_then(|config| config.with_queue_capacity(4))
            .expect("valid worker config");
        let worker = DatabaseWorker::start(config).expect("worker starts");
        let (entered_sender, entered_receiver) = sync_channel(1);
        let (release_sender, release_receiver) = sync_channel(1);
        worker
            .client
            .sender
            .send(DatabaseCommand::BlockForQueueTest {
                entered: entered_sender,
                release: release_receiver,
            })
            .expect("barrier command accepted");
        entered_receiver.recv().expect("worker entered barrier");

        let first = StoredSettings {
            tuning: TuningSettings {
                scan_window_ms: 4_000,
                ..TuningSettings::default()
            },
            updated_at_ms: 1,
        };
        let second = StoredSettings {
            tuning: TuningSettings {
                scan_window_ms: 5_000,
                ..TuningSettings::default()
            },
            updated_at_ms: 2,
        };
        let (first_sender, first_receiver) = sync_channel(1);
        let (second_sender, second_receiver) = sync_channel(1);
        let (load_sender, load_receiver) = sync_channel(1);
        worker
            .client
            .sender
            .send(DatabaseCommand::SaveSettings {
                settings: first,
                reply: first_sender,
            })
            .expect("first save queued");
        worker
            .client
            .sender
            .send(DatabaseCommand::SaveSettings {
                settings: second.clone(),
                reply: second_sender,
            })
            .expect("second save queued");
        worker
            .client
            .sender
            .send(DatabaseCommand::LoadSettings { reply: load_sender })
            .expect("load queued");
        release_sender.send(()).expect("release worker");

        first_receiver
            .recv()
            .expect("first reply received")
            .expect("first save succeeds");
        second_receiver
            .recv()
            .expect("second reply received")
            .expect("second save succeeds");
        assert_eq!(
            load_receiver
                .recv()
                .expect("load reply received")
                .expect("load succeeds"),
            Some(second)
        );
        worker.stop_and_join().expect("worker closes and joins");
    }

    #[test]
    fn explicit_stop_then_join_rejects_cloned_clients_deterministically() {
        let test_path = TestDatabasePath::new("worker-lifecycle");
        let config = DatabaseConfig::new(test_path.path()).expect("valid worker config");
        let mut worker = DatabaseWorker::start(config).expect("worker starts");
        let client = worker.client();

        worker.stop().expect("stop checkpoints and closes");
        let error = client
            .metadata()
            .expect_err("stopped worker rejects requests");
        assert_eq!(error.kind, StorageErrorKind::WorkerStopped);
        worker.join().expect("join succeeds");
    }

    fn sample_device() -> TrustedDevice {
        TrustedDevice {
            device_id: DeviceId::new("listener-東京").expect("valid device identifier"),
            display_name: "Zoë 🎧 東京".into(),
            public_key: Some(vec![0, 1, 2, 0xff, 0]),
            private_key_ref: Some("keystore:listener-1".into()),
            trust_state: TrustState::Trusted,
            first_seen_ms: 10,
            last_seen_ms: 20,
            updated_at_ms: 30,
        }
    }

    fn sample_session_start() -> SessionStart {
        SessionStart {
            session_id: SessionId::new("session-1").expect("valid session identifier"),
            role: AppRole::Host,
            session_name: "Noche silenciosa 東京".into(),
            started_at_ms: 100,
        }
    }

    fn sample_diagnostic(session_id: &SessionId) -> DiagnosticRunSummary {
        DiagnosticRunSummary {
            run_id: DiagnosticRunId::new("diagnostic-1").expect("valid diagnostic identifier"),
            session_id: Some(session_id.clone()),
            started_at_ms: 110,
            ended_at_ms: Some(240),
            summary_json: r#"{"listeners":3,"quality":"good"}"#.into(),
        }
    }
}
