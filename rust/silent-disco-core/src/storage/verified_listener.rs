//! Durable trust in a verified listener key (remediation F1): the model and
//! its repository. Rust owns this table; the apps never write it directly.

use rusqlite::{Connection, OptionalExtension, params};

use super::{
    error::{StorageError, StorageOperation, map_sqlite_error},
    models::{
        MAX_DISPLAY_NAME_BYTES, StorageModelValidationError, validate_human_name,
        validate_sql_millis,
    },
    repository_support::{corrupt_row, from_sql_u64, invalid_model, to_sql_i64},
};

/// One listener key the host chose to "Always allow", scoped to the host's
/// own key. Fingerprints are SHA-256 digests of the keys' SPKI DER.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedListenerRecord {
    pub host_key_fingerprint: [u8; 32],
    pub listener_key_fingerprint: [u8; 32],
    pub display_name: String,
    pub enrolled_at_ms: u64,
    /// Set once the host revokes the key; a revoked key is never trusted.
    pub revoked_at_ms: Option<u64>,
}

impl VerifiedListenerRecord {
    /// Whether the host currently trusts this key.
    #[must_use]
    pub const fn is_trusted(&self) -> bool {
        self.revoked_at_ms.is_none()
    }

    /// Validates the display name and timestamps.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for unsupported values.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_human_name(&self.display_name, MAX_DISPLAY_NAME_BYTES)
            .map_err(|()| StorageModelValidationError::DisplayName)?;
        validate_sql_millis(self.enrolled_at_ms, StorageModelValidationError::Timestamp)?;
        if let Some(revoked) = self.revoked_at_ms {
            validate_sql_millis(revoked, StorageModelValidationError::Timestamp)?;
            if revoked < self.enrolled_at_ms {
                return Err(StorageModelValidationError::TimestampOrder);
            }
        }
        Ok(())
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn unhex(text: &str, schema_version: u32) -> Result<[u8; 32], StorageError> {
    let bad = || {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            "a stored key fingerprint is not 32 hex bytes",
        )
    };
    if text.len() != 64 {
        return Err(bad());
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).map_err(|_| bad())?;
    }
    Ok(out)
}

/// Every record for `host_key_fingerprint`, revoked ones included, by name.
pub(crate) fn list(
    connection: &Connection,
    host_key_fingerprint: &[u8; 32],
    schema_version: u32,
) -> Result<Vec<VerifiedListenerRecord>, StorageError> {
    let query = |error: rusqlite::Error| {
        map_sqlite_error(StorageOperation::Query, Some(schema_version), &error)
    };
    let mut statement = connection
        .prepare(
            "SELECT listener_key_fingerprint, display_name, enrolled_at_ms, revoked_at_ms
             FROM verified_listeners
             WHERE host_key_fingerprint = ?1
             ORDER BY display_name COLLATE NOCASE ASC, listener_key_fingerprint ASC",
        )
        .map_err(query)?;
    let rows = statement
        .query_map([hex(host_key_fingerprint)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(query)?;
    let mut records = Vec::new();
    for row in rows {
        let (listener, display_name, enrolled, revoked) = row.map_err(query)?;
        let record = VerifiedListenerRecord {
            host_key_fingerprint: *host_key_fingerprint,
            listener_key_fingerprint: unhex(&listener, schema_version)?,
            display_name,
            enrolled_at_ms: from_sql_u64(
                enrolled,
                StorageOperation::Query,
                schema_version,
                "enrolled_at_ms",
            )?,
            revoked_at_ms: revoked
                .map(|value| {
                    from_sql_u64(
                        value,
                        StorageOperation::Query,
                        schema_version,
                        "revoked_at_ms",
                    )
                })
                .transpose()?,
        };
        record.validate().map_err(|error| {
            corrupt_row(
                StorageOperation::Query,
                schema_version,
                format!("stored verified listener is invalid: {error}"),
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

/// Records "Always allow" for a key. Enrolling a revoked key again clears its
/// revocation: that is the host explicitly trusting it again.
pub(crate) fn enroll(
    connection: &mut Connection,
    record: &VerifiedListenerRecord,
    schema_version: u32,
) -> Result<(), StorageError> {
    record
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    if record.revoked_at_ms.is_some() {
        return Err(invalid_model(
            StorageOperation::Transaction,
            schema_version,
            "a new enrollment cannot already be revoked",
        ));
    }
    let enrolled = to_sql_i64(
        record.enrolled_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "enrolled_at_ms",
    )?;
    let write = |error: rusqlite::Error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    };
    let transaction = connection.transaction().map_err(write)?;
    transaction
        .execute(
            "INSERT INTO verified_listeners
                 (host_key_fingerprint, listener_key_fingerprint, display_name, enrolled_at_ms, revoked_at_ms)
             VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT (host_key_fingerprint, listener_key_fingerprint) DO UPDATE SET
                 display_name = excluded.display_name,
                 enrolled_at_ms = excluded.enrolled_at_ms,
                 revoked_at_ms = NULL",
            params![
                hex(&record.host_key_fingerprint),
                hex(&record.listener_key_fingerprint),
                record.display_name.as_str(),
                enrolled,
            ],
        )
        .map_err(write)?;
    transaction.commit().map_err(write)
}

/// Revokes a trusted key, keeping its row. Returns whether a trusted row was
/// revoked; an unknown or already revoked key reports `false`.
pub(crate) fn revoke(
    connection: &mut Connection,
    host_key_fingerprint: &[u8; 32],
    listener_key_fingerprint: &[u8; 32],
    revoked_at_ms: u64,
    schema_version: u32,
) -> Result<bool, StorageError> {
    let revoked = to_sql_i64(
        revoked_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "revoked_at_ms",
    )?;
    let write = |error: rusqlite::Error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    };
    let transaction = connection.transaction().map_err(write)?;
    let enrolled: Option<i64> = transaction
        .query_row(
            "SELECT enrolled_at_ms FROM verified_listeners
             WHERE host_key_fingerprint = ?1 AND listener_key_fingerprint = ?2 AND revoked_at_ms IS NULL",
            params![hex(host_key_fingerprint), hex(listener_key_fingerprint)],
            |row| row.get(0),
        )
        .optional()
        .map_err(write)?;
    let Some(enrolled) = enrolled else {
        return Ok(false);
    };
    // A revocation is never dated before its enrollment.
    let revoked = revoked.max(enrolled);
    let affected = transaction
        .execute(
            "UPDATE verified_listeners SET revoked_at_ms = ?3
             WHERE host_key_fingerprint = ?1 AND listener_key_fingerprint = ?2 AND revoked_at_ms IS NULL",
            params![hex(host_key_fingerprint), hex(listener_key_fingerprint), revoked],
        )
        .map_err(write)?;
    transaction.commit().map_err(write)?;
    Ok(affected == 1)
}

#[cfg(test)]
#[path = "verified_listener_tests.rs"]
mod tests;
