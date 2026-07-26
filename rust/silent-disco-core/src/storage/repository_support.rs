use core::fmt;

use super::error::{StorageError, StorageErrorKind, StorageOperation};

pub(crate) fn invalid_model(
    operation: StorageOperation,
    schema_version: u32,
    error: impl fmt::Display,
) -> StorageError {
    StorageError::new(
        StorageErrorKind::InvalidConfiguration,
        operation,
        format!("persisted model validation failed: {error}"),
        Some(schema_version),
    )
}

pub(crate) fn corrupt_row(
    operation: StorageOperation,
    schema_version: u32,
    message: impl Into<String>,
) -> StorageError {
    StorageError::new(
        StorageErrorKind::Corruption,
        operation,
        message,
        Some(schema_version),
    )
}

pub(crate) fn to_sql_i64(
    value: u64,
    operation: StorageOperation,
    schema_version: u32,
    field: &'static str,
) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| {
        invalid_model(
            operation,
            schema_version,
            format!("{field} exceeds the SQLite integer range"),
        )
    })
}

pub(crate) fn from_sql_u64(
    value: i64,
    operation: StorageOperation,
    schema_version: u32,
    field: &'static str,
) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| {
        corrupt_row(
            operation,
            schema_version,
            format!("{field} is negative or outside the supported range"),
        )
    })
}

pub(crate) fn from_sql_u32(
    value: i64,
    operation: StorageOperation,
    schema_version: u32,
    field: &'static str,
) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| {
        corrupt_row(
            operation,
            schema_version,
            format!("{field} is outside the supported range"),
        )
    })
}

pub(crate) fn from_sql_u16(
    value: i64,
    operation: StorageOperation,
    schema_version: u32,
    field: &'static str,
) -> Result<u16, StorageError> {
    u16::try_from(value).map_err(|_| {
        corrupt_row(
            operation,
            schema_version,
            format!("{field} is outside the supported range"),
        )
    })
}
