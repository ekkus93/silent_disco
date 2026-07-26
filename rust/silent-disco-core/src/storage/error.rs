use core::fmt;
use std::error::Error;

use rusqlite::{Error as SqliteError, ErrorCode};

use crate::error::{
    CoreError, CoreErrorCode, ErrorContextEntry, ErrorSeverity, MAX_ERROR_CONTEXT_VALUE_BYTES,
    MAX_ERROR_MESSAGE_BYTES,
};

/// Stable storage operations included in diagnostics and structured failures.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageOperation {
    ValidateConfiguration = 1,
    StartWorker = 2,
    OpenDatabase = 3,
    ConfigureForeignKeys = 4,
    ConfigureJournalMode = 5,
    ConfigureBusyTimeout = 6,
    ConfigureSynchronousPolicy = 7,
    ReadMetadata = 8,
    Checkpoint = 9,
    Migration = 10,
    Query = 11,
    Transaction = 12,
    CloseDatabase = 13,
    StopWorker = 14,
    JoinWorker = 15,
    ImportLegacyData = 16,
}

impl StorageOperation {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::ValidateConfiguration => "validate_configuration",
            Self::StartWorker => "start_worker",
            Self::OpenDatabase => "open_database",
            Self::ConfigureForeignKeys => "configure_foreign_keys",
            Self::ConfigureJournalMode => "configure_journal_mode",
            Self::ConfigureBusyTimeout => "configure_busy_timeout",
            Self::ConfigureSynchronousPolicy => "configure_synchronous_policy",
            Self::ReadMetadata => "read_metadata",
            Self::Checkpoint => "checkpoint",
            Self::Migration => "migration",
            Self::Query => "query",
            Self::Transaction => "transaction",
            Self::CloseDatabase => "close_database",
            Self::StopWorker => "stop_worker",
            Self::JoinWorker => "join_worker",
            Self::ImportLegacyData => "import_legacy_data",
        }
    }

    const fn default_error_kind(self) -> StorageErrorKind {
        match self {
            Self::ValidateConfiguration => StorageErrorKind::InvalidConfiguration,
            Self::StartWorker => StorageErrorKind::ThreadStart,
            Self::OpenDatabase => StorageErrorKind::Open,
            Self::ConfigureForeignKeys
            | Self::ConfigureJournalMode
            | Self::ConfigureBusyTimeout
            | Self::ConfigureSynchronousPolicy => StorageErrorKind::Pragma,
            Self::Migration => StorageErrorKind::Migration,
            Self::ReadMetadata | Self::Checkpoint | Self::Query => StorageErrorKind::Query,
            Self::Transaction | Self::ImportLegacyData => StorageErrorKind::Transaction,
            Self::CloseDatabase => StorageErrorKind::Close,
            Self::StopWorker | Self::JoinWorker => StorageErrorKind::WorkerStopped,
        }
    }
}

impl fmt::Display for StorageOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

/// Non-overlapping storage failure categories used by tests, diagnostics, and UI policy.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageErrorKind {
    InvalidConfiguration = 1,
    Open = 2,
    Pragma = 3,
    Migration = 4,
    Query = 5,
    Transaction = 6,
    Constraint = 7,
    Busy = 8,
    Corruption = 9,
    Close = 10,
    QueueFull = 11,
    ThreadStart = 12,
    WorkerStopped = 13,
    WorkerPanicked = 14,
    ReplyDisconnected = 15,
    ShutdownInProgress = 16,
}

impl StorageErrorKind {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_configuration",
            Self::Open => "open",
            Self::Pragma => "pragma",
            Self::Migration => "migration",
            Self::Query => "query",
            Self::Transaction => "transaction",
            Self::Constraint => "constraint",
            Self::Busy => "busy",
            Self::Corruption => "corruption",
            Self::Close => "close",
            Self::QueueFull => "queue_full",
            Self::ThreadStart => "thread_start",
            Self::WorkerStopped => "worker_stopped",
            Self::WorkerPanicked => "worker_panicked",
            Self::ReplyDisconnected => "reply_disconnected",
            Self::ShutdownInProgress => "shutdown_in_progress",
        }
    }

    #[must_use]
    pub const fn core_error_code(self) -> CoreErrorCode {
        match self {
            Self::InvalidConfiguration => CoreErrorCode::InvalidArgument,
            Self::Open => CoreErrorCode::StorageOpenFailed,
            Self::Pragma => CoreErrorCode::StoragePragmaFailed,
            Self::Migration => CoreErrorCode::StorageMigrationFailed,
            Self::Query => CoreErrorCode::StorageQueryFailed,
            Self::Transaction => CoreErrorCode::StorageTransactionFailed,
            Self::Constraint => CoreErrorCode::StorageConstraintViolation,
            Self::Busy | Self::QueueFull => CoreErrorCode::StorageBusy,
            Self::Corruption => CoreErrorCode::StorageCorrupt,
            Self::Close => CoreErrorCode::StorageCloseFailed,
            Self::ThreadStart | Self::WorkerStopped | Self::ReplyDisconnected => {
                CoreErrorCode::WorkerStopped
            }
            Self::WorkerPanicked => CoreErrorCode::ShutdownFailed,
            Self::ShutdownInProgress => CoreErrorCode::ShutdownInProgress,
        }
    }

    const fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::Busy | Self::QueueFull | Self::WorkerStopped | Self::ReplyDisconnected
        )
    }

    const fn core_remains_usable(self) -> bool {
        !matches!(
            self,
            Self::Open
                | Self::Pragma
                | Self::Migration
                | Self::Corruption
                | Self::Close
                | Self::ThreadStart
                | Self::WorkerPanicked
        )
    }

    const fn severity(self) -> ErrorSeverity {
        if self.core_remains_usable() {
            ErrorSeverity::Error
        } else {
            ErrorSeverity::Fatal
        }
    }
}

impl fmt::Display for StorageErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

/// Structured storage failure that preserves operation and schema context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    pub kind: StorageErrorKind,
    pub operation: StorageOperation,
    pub message: String,
    pub schema_version: Option<u32>,
    pub retryable: bool,
    pub core_remains_usable: bool,
}

impl StorageError {
    #[must_use]
    pub(crate) fn new(
        kind: StorageErrorKind,
        operation: StorageOperation,
        message: impl Into<String>,
        schema_version: Option<u32>,
    ) -> Self {
        Self {
            kind,
            operation,
            message: truncate_utf8(message.into(), MAX_ERROR_MESSAGE_BYTES),
            schema_version,
            retryable: kind.default_retryable(),
            core_remains_usable: kind.core_remains_usable(),
        }
    }

    #[must_use]
    pub(crate) fn queue_full(operation: StorageOperation, schema_version: u32) -> Self {
        Self::new(
            StorageErrorKind::QueueFull,
            operation,
            "database worker queue is full; request was rejected without being dropped",
            Some(schema_version),
        )
    }

    #[must_use]
    pub(crate) fn worker_stopped(operation: StorageOperation, schema_version: Option<u32>) -> Self {
        Self::new(
            StorageErrorKind::WorkerStopped,
            operation,
            "database worker is not accepting requests",
            schema_version,
        )
    }

    #[must_use]
    pub(crate) fn reply_disconnected(operation: StorageOperation, schema_version: u32) -> Self {
        Self::new(
            StorageErrorKind::ReplyDisconnected,
            operation,
            "database worker stopped before returning the requested result",
            Some(schema_version),
        )
    }

    #[must_use]
    pub(crate) fn worker_panicked() -> Self {
        Self::new(
            StorageErrorKind::WorkerPanicked,
            StorageOperation::JoinWorker,
            "database worker thread panicked",
            None,
        )
    }

    #[must_use]
    pub(crate) fn shutdown_in_progress() -> Self {
        Self::new(
            StorageErrorKind::ShutdownInProgress,
            StorageOperation::StopWorker,
            "database worker shutdown was already requested",
            None,
        )
    }

    /// Converts the storage failure into the shared stable core error contract.
    #[must_use]
    pub fn to_core_error(&self) -> CoreError {
        let schema_value = self
            .schema_version
            .map_or_else(|| "unavailable".to_owned(), |value| value.to_string());
        CoreError {
            code: self.kind.core_error_code(),
            message: truncate_utf8(self.message.clone(), MAX_ERROR_MESSAGE_BYTES),
            subsystem: self.kind.core_error_code().subsystem(),
            severity: self.kind.severity(),
            retryable: self.retryable,
            operation_id: None,
            context: vec![
                ErrorContextEntry {
                    key: "storage_operation".into(),
                    value: truncate_utf8(
                        self.operation.stable_name().to_owned(),
                        MAX_ERROR_CONTEXT_VALUE_BYTES,
                    ),
                },
                ErrorContextEntry {
                    key: "storage_error_kind".into(),
                    value: truncate_utf8(
                        self.kind.stable_name().to_owned(),
                        MAX_ERROR_CONTEXT_VALUE_BYTES,
                    ),
                },
                ErrorContextEntry {
                    key: "schema_version".into(),
                    value: truncate_utf8(schema_value, MAX_ERROR_CONTEXT_VALUE_BYTES),
                },
                ErrorContextEntry {
                    key: "core_remains_usable".into(),
                    value: self.core_remains_usable.to_string(),
                },
            ],
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "storage:{}:{}: {}",
            self.operation, self.kind, self.message
        )
    }
}

impl Error for StorageError {}

#[must_use]
pub(crate) fn map_sqlite_error(
    operation: StorageOperation,
    schema_version: Option<u32>,
    error: &SqliteError,
) -> StorageError {
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => StorageErrorKind::Busy,
        Some(ErrorCode::ConstraintViolation) => StorageErrorKind::Constraint,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => StorageErrorKind::Corruption,
        _ => operation.default_error_kind(),
    };
    StorageError::new(
        kind,
        operation,
        format!("SQLite {} failed: {error}", operation.stable_name()),
        schema_version,
    )
}

fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut boundary = maximum_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

#[cfg(test)]
mod tests {
    use rusqlite::{Error as SqliteError, ffi};

    use super::{
        StorageError, StorageErrorKind, StorageOperation, map_sqlite_error, truncate_utf8,
    };
    use crate::error::{CoreErrorCode, CoreSubsystem};

    fn sqlite_failure(code: i32) -> SqliteError {
        SqliteError::SqliteFailure(ffi::Error::new(code), None)
    }

    #[test]
    fn maps_sqlite_failure_categories_without_collapsing_them() {
        assert_eq!(
            map_sqlite_error(
                StorageOperation::Query,
                Some(7),
                &sqlite_failure(ffi::SQLITE_BUSY),
            )
            .kind,
            StorageErrorKind::Busy
        );
        assert_eq!(
            map_sqlite_error(
                StorageOperation::Transaction,
                Some(7),
                &sqlite_failure(ffi::SQLITE_CONSTRAINT),
            )
            .kind,
            StorageErrorKind::Constraint
        );
        assert_eq!(
            map_sqlite_error(
                StorageOperation::Query,
                Some(7),
                &sqlite_failure(ffi::SQLITE_CORRUPT),
            )
            .kind,
            StorageErrorKind::Corruption
        );
        assert_eq!(
            map_sqlite_error(
                StorageOperation::Migration,
                Some(7),
                &sqlite_failure(ffi::SQLITE_ERROR),
            )
            .kind,
            StorageErrorKind::Migration
        );
        assert_eq!(
            map_sqlite_error(
                StorageOperation::Transaction,
                Some(7),
                &sqlite_failure(ffi::SQLITE_ERROR),
            )
            .kind,
            StorageErrorKind::Transaction
        );
        assert_eq!(
            map_sqlite_error(
                StorageOperation::CloseDatabase,
                Some(7),
                &sqlite_failure(ffi::SQLITE_ERROR),
            )
            .kind,
            StorageErrorKind::Close
        );
    }

    #[test]
    fn core_error_preserves_operation_and_schema_context() {
        let error = StorageError::new(
            StorageErrorKind::Constraint,
            StorageOperation::Transaction,
            "constraint failure",
            Some(4),
        )
        .to_core_error();

        assert_eq!(error.code, CoreErrorCode::StorageConstraintViolation);
        assert_eq!(error.subsystem, CoreSubsystem::Storage);
        assert!(
            error
                .context
                .iter()
                .any(|entry| { entry.key == "storage_operation" && entry.value == "transaction" })
        );
        assert!(
            error
                .context
                .iter()
                .any(|entry| { entry.key == "schema_version" && entry.value == "4" })
        );
    }

    #[test]
    fn core_error_subsystem_always_matches_its_stable_code() {
        let invalid_configuration = StorageError::new(
            StorageErrorKind::InvalidConfiguration,
            StorageOperation::ValidateConfiguration,
            "invalid configuration",
            None,
        )
        .to_core_error();
        assert_eq!(invalid_configuration.code, CoreErrorCode::InvalidArgument);
        assert_eq!(invalid_configuration.subsystem, CoreSubsystem::Validation);
        assert_eq!(
            invalid_configuration.subsystem,
            invalid_configuration.code.subsystem()
        );

        let worker_stopped =
            StorageError::worker_stopped(StorageOperation::ReadMetadata, Some(0)).to_core_error();
        assert_eq!(worker_stopped.code, CoreErrorCode::WorkerStopped);
        assert_eq!(worker_stopped.subsystem, CoreSubsystem::Runtime);
        assert_eq!(worker_stopped.subsystem, worker_stopped.code.subsystem());
    }

    #[test]
    fn truncation_preserves_utf8_boundaries() {
        let truncated = truncate_utf8("ééé".to_owned(), 5);
        assert_eq!(truncated, "éé");
    }
}
