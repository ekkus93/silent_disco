//! Rust-owned `SQLite` worker, schema, migration, and repository layer.
//!
//! The `SQLite` connection is private to one dedicated thread. Public callers
//! receive typed control-plane operations only; raw SQL and the connection
//! object never cross this module boundary. Schema and repository internals
//! remain private; callers interact through [`DatabaseClient`].

mod database;
mod diagnostics_repository;
mod error;
mod legacy_import_repository;
mod migrations;
mod models;
mod repository_support;
mod session_read_repository;
mod session_write_repository;
mod settings_repository;
mod trusted_device_repository;
mod worker;

#[cfg(test)]
pub(crate) mod test_support;

pub use database::{
    DEFAULT_BUSY_TIMEOUT_MS, DEFAULT_DATABASE_QUEUE_CAPACITY, DatabaseCheckpoint, DatabaseConfig,
    DatabaseMetadata, SynchronousPolicy,
};
pub use error::{StorageError, StorageErrorKind, StorageOperation};
pub use migrations::{LATEST_SCHEMA_VERSION, MigrationRecord};
pub use models::{
    DiagnosticExport, DiagnosticExportCursor, DiagnosticExportRequest, DiagnosticQuery,
    DiagnosticRunSummary, LEGACY_ANDROID_IMPORT_SOURCE, LEGACY_ANDROID_IMPORT_VERSION,
    LegacyAndroidImport, LegacyImportOutcome, MAX_DIAGNOSTIC_EXPORT_LIMIT,
    MAX_DIAGNOSTIC_QUERY_LIMIT, MAX_DIAGNOSTIC_SUMMARY_BYTES, MAX_DISPLAY_NAME_BYTES,
    MAX_FAILURE_CODE_BYTES, MAX_FAILURE_MESSAGE_BYTES, MAX_PRIVATE_KEY_REFERENCE_BYTES,
    MAX_PUBLIC_KEY_BYTES, MAX_SESSION_NAME_BYTES, SessionEnd, SessionHistory, SessionOutcome,
    SessionStart, SessionUpdate, StorageModelValidationError, StoredSettings, TrustedDevice,
};
pub use worker::{DatabaseClient, DatabaseWorker};
