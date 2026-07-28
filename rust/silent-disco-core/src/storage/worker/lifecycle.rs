use super::{
    Arc, AtomicBool, AtomicU32, DatabaseClient, DatabaseConfig, DatabaseMetadata, DatabaseWorker,
    Ordering, StorageError, StorageErrorKind, StorageOperation, run_database_worker, sync_channel,
    thread,
};

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
