use rusqlite::{Connection, params};

use super::{
    error::{StorageError, StorageErrorKind, StorageOperation, map_sqlite_error},
    models::{SessionEnd, SessionStart, SessionUpdate},
    repository_support::{invalid_model, to_sql_i64},
};

pub(crate) fn begin(
    connection: &mut Connection,
    session: &SessionStart,
    schema_version: u32,
) -> Result<(), StorageError> {
    session
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    let started_at_ms = to_sql_i64(
        session.started_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "started_at_ms",
    )?;
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    transaction
        .execute(
            "INSERT INTO session_history (
                 session_id,
                 role,
                 session_name,
                 started_at_ms,
                 ended_at_ms,
                 listener_count,
                 outcome,
                 failure_code,
                 failure_message
             ) VALUES (?1, ?2, ?3, ?4, NULL, 0, 'active', NULL, NULL)",
            params![
                session.session_id.as_str(),
                session.role.wire_name(),
                session.session_name.as_str(),
                started_at_ms,
            ],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })
}

pub(crate) fn update(
    connection: &mut Connection,
    update: &SessionUpdate,
    schema_version: u32,
) -> Result<bool, StorageError> {
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    let affected = transaction
        .execute(
            "UPDATE session_history
             SET listener_count = ?1
             WHERE session_id = ?2 AND outcome = 'active'",
            params![i64::from(update.listener_count), update.session_id.as_str()],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(affected == 1)
}

pub(crate) fn end(
    connection: &mut Connection,
    end: &SessionEnd,
    schema_version: u32,
) -> Result<bool, StorageError> {
    end.validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    let ended_at_ms = to_sql_i64(
        end.ended_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "ended_at_ms",
    )?;
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    let affected = transaction
        .execute(
            "UPDATE session_history
             SET ended_at_ms = ?1,
                 listener_count = ?2,
                 outcome = ?3,
                 failure_code = ?4,
                 failure_message = ?5
             WHERE session_id = ?6 AND outcome = 'active'",
            params![
                ended_at_ms,
                i64::from(end.listener_count),
                end.outcome.wire_name(),
                end.failure_code.as_deref(),
                end.failure_message.as_deref(),
                end.session_id.as_str(),
            ],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(affected == 1)
}

pub(crate) fn interrupt_active_hosts(
    connection: &mut Connection,
    ended_at_ms: u64,
    schema_version: u32,
) -> Result<u32, StorageError> {
    let ended_at_ms = to_sql_i64(
        ended_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "ended_at_ms",
    )?;
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    let affected = transaction
        .execute(
            "UPDATE session_history
             SET ended_at_ms = MAX(?1, started_at_ms),
                 outcome = 'failed',
                 failure_code = 'desktop.host_session.interrupted',
                 failure_message = 'The desktop closed before the host session ended.'
             WHERE role = 'host' AND outcome = 'active'",
            [ended_at_ms],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    let affected = u32::try_from(affected).map_err(|_| {
        StorageError::new(
            StorageErrorKind::InvalidConfiguration,
            StorageOperation::Transaction,
            "interrupted host session count exceeds the supported range",
            Some(schema_version),
        )
    })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    Ok(affected)
}
