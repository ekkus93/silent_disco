use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
};

use super::{
    database::{DatabaseCheckpoint, DatabaseConfig, DatabaseConnection, DatabaseMetadata},
    error::{StorageError, StorageErrorKind, StorageOperation},
};

type DatabaseReply<T> = SyncSender<Result<T, StorageError>>;

enum DatabaseCommand {
    ReadMetadata {
        reply: DatabaseReply<DatabaseMetadata>,
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
    schema_version: Arc<AtomicU32>,
}

impl DatabaseClient {
    /// Returns verified `SQLite` diagnostics from the worker-owned connection.
    ///
    /// # Errors
    ///
    /// Returns `StorageBusy` when the bounded queue is full and a worker error
    /// when the request cannot be processed. A rejected request is never dropped
    /// after being reported as accepted.
    pub fn metadata(&self) -> Result<DatabaseMetadata, StorageError> {
        self.request(StorageOperation::ReadMetadata, |reply| {
            DatabaseCommand::ReadMetadata { reply }
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
    /// Starts a worker and does not return until `SQLite` is open and every
    /// required connection setting has been verified.
    ///
    /// # Errors
    ///
    /// Returns a structured configuration, thread-start, open, pragma, or
    /// corruption failure. Failed initialization never produces a usable client.
    pub fn start(config: DatabaseConfig) -> Result<Self, StorageError> {
        config.validate()?;
        let (command_sender, command_receiver) = sync_channel(config.queue_capacity);
        let (startup_sender, startup_receiver) = sync_channel(1);
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
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
            mpsc::sync_channel,
        },
        thread,
    };

    use super::{DatabaseCommand, DatabaseWorker};
    use crate::storage::{DatabaseConfig, StorageErrorKind};

    static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);

    struct TestDatabasePath {
        path: PathBuf,
    }

    impl TestDatabasePath {
        fn new(label: &str) -> Self {
            let unique = NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "silent-disco-worker-{label}-{}-{unique}.sqlite3",
                std::process::id()
            ));
            Self { path }
        }
    }

    impl Drop for TestDatabasePath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(PathBuf::from(format!("{}-wal", self.path.display())));
            let _ = fs::remove_file(PathBuf::from(format!("{}-shm", self.path.display())));
        }
    }

    #[test]
    fn one_worker_thread_owns_every_connection_operation() {
        let test_path = TestDatabasePath::new("ownership");
        let config = DatabaseConfig::new(&test_path.path)
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
        let test_path = TestDatabasePath::new("queue");
        let config = DatabaseConfig::new(&test_path.path)
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
    fn explicit_stop_then_join_closes_worker_and_disconnects_clients() {
        let test_path = TestDatabasePath::new("lifecycle");
        let config = DatabaseConfig::new(&test_path.path).expect("valid worker config");
        let mut worker = DatabaseWorker::start(config).expect("worker starts");
        let client = worker.client();

        worker.stop().expect("stop checkpoints and closes");
        let error = client
            .metadata()
            .expect_err("stopped worker rejects requests");
        assert_eq!(error.kind, StorageErrorKind::WorkerStopped);
        worker.join().expect("join succeeds");
    }

    #[test]
    fn startup_open_failure_is_returned_without_a_client() {
        let test_path = TestDatabasePath::new("missing-parent");
        let missing = test_path.path.join("missing").join("database.sqlite3");
        let config = DatabaseConfig::new(missing).expect("valid path shape");
        let error = match DatabaseWorker::start(config) {
            Ok(worker) => {
                let _ = worker.stop_and_join();
                panic!("database in a missing directory unexpectedly opened");
            }
            Err(error) => error,
        };

        assert_eq!(error.kind, StorageErrorKind::Open);
    }
}
