use super::{
    DatabaseCheckpoint, DatabaseClient, DatabaseCommand, DatabaseMetadata, DatabaseReply, DeviceId,
    DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
    LegacyAndroidImport, LegacyImportOutcome, Ordering, SessionEnd, SessionHistory, SessionId,
    SessionStart, SessionUpdate, StorageError, StorageOperation, StoredSettings, TrustedDevice,
    TrySendError, sync_channel,
};

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

    /// Transactionally imports known Android legacy values exactly once.
    ///
    /// # Errors
    ///
    /// Returns a visible validation, queue, transaction, corruption, or worker error.
    pub fn import_legacy_android(
        &self,
        value: &LegacyAndroidImport,
    ) -> Result<LegacyImportOutcome, StorageError> {
        let value = value.clone();
        self.request(StorageOperation::Transaction, |reply| {
            DatabaseCommand::ImportLegacyAndroid { value, reply }
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

    pub(super) fn request_shutdown(&self) -> Result<(), StorageError> {
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
