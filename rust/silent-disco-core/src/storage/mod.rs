//! Rust-owned `SQLite` worker infrastructure.
//!
//! The `SQLite` connection is private to one dedicated thread. Public callers
//! receive typed control-plane operations only; raw SQL and the connection
//! object never cross this module boundary.

mod database;
mod error;
mod worker;

pub use database::{
    DEFAULT_BUSY_TIMEOUT_MS, DEFAULT_DATABASE_QUEUE_CAPACITY, DatabaseCheckpoint, DatabaseConfig,
    DatabaseMetadata, SynchronousPolicy,
};
pub use error::{StorageError, StorageErrorKind, StorageOperation};
pub use worker::{DatabaseClient, DatabaseWorker};
