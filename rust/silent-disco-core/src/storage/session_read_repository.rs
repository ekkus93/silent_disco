use rusqlite::{Connection, OptionalExtension};

use crate::domain::{AppRole, SessionId};

use super::{
    error::{StorageError, StorageErrorKind, StorageOperation, map_sqlite_error},
    models::{
        MAX_RECENT_SESSION_HISTORY_LIMIT, SessionEnd, SessionHistory, SessionOutcome, SessionStart,
    },
    repository_support::{corrupt_row, from_sql_u32, from_sql_u64},
};

struct RawSessionHistory {
    session_id: String,
    role: String,
    session_name: String,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    listener_count: i64,
    outcome: String,
    failure_code: Option<String>,
    failure_message: Option<String>,
}

pub(crate) fn get(
    connection: &Connection,
    session_id: &SessionId,
    schema_version: u32,
) -> Result<Option<SessionHistory>, StorageError> {
    let raw = connection
        .query_row(
            "SELECT
                 session_id,
                 role,
                 session_name,
                 started_at_ms,
                 ended_at_ms,
                 listener_count,
                 outcome,
                 failure_code,
                 failure_message
             FROM session_history
             WHERE session_id = ?1",
            [session_id.as_str()],
            read_raw,
        )
        .optional()
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    raw.map(|value| decode(value, schema_version)).transpose()
}

/// Lists the most recent session-history rows in deterministic newest-first order.
///
/// The caller-supplied limit is validated before preparing SQL so a future IPC
/// adapter cannot accidentally turn this into an unbounded history read.
pub(crate) fn list_recent(
    connection: &Connection,
    limit: u32,
    schema_version: u32,
) -> Result<Vec<SessionHistory>, StorageError> {
    if limit == 0 || limit > MAX_RECENT_SESSION_HISTORY_LIMIT {
        return Err(StorageError::new(
            StorageErrorKind::InvalidConfiguration,
            StorageOperation::Query,
            format!(
                "recent session history limit must be between 1 and {MAX_RECENT_SESSION_HISTORY_LIMIT}"
            ),
            Some(schema_version),
        ));
    }

    let mut statement = connection
        .prepare(
            "SELECT
                 session_id,
                 role,
                 session_name,
                 started_at_ms,
                 ended_at_ms,
                 listener_count,
                 outcome,
                 failure_code,
                 failure_message
             FROM session_history
             ORDER BY started_at_ms DESC, session_id ASC
             LIMIT ?1",
        )
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let rows = statement
        .query_map([i64::from(limit)], read_raw)
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let mut histories = Vec::with_capacity(limit as usize);
    for row in rows {
        let raw = row.map_err(|error| {
            map_sqlite_error(StorageOperation::Query, Some(schema_version), &error)
        })?;
        histories.push(decode(raw, schema_version)?);
    }
    Ok(histories)
}

fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawSessionHistory> {
    Ok(RawSessionHistory {
        session_id: row.get(0)?,
        role: row.get(1)?,
        session_name: row.get(2)?,
        started_at_ms: row.get(3)?,
        ended_at_ms: row.get(4)?,
        listener_count: row.get(5)?,
        outcome: row.get(6)?,
        failure_code: row.get(7)?,
        failure_message: row.get(8)?,
    })
}

fn decode(raw: RawSessionHistory, schema_version: u32) -> Result<SessionHistory, StorageError> {
    let session_id = SessionId::new(raw.session_id).map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored session identifier is invalid: {error}"),
        )
    })?;
    let role = AppRole::from_wire_name(&raw.role).map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored session role is invalid: {error}"),
        )
    })?;
    let outcome = SessionOutcome::from_wire_name(&raw.outcome).map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored session outcome is invalid: {error}"),
        )
    })?;
    let history = SessionHistory {
        session_id,
        role,
        session_name: raw.session_name,
        started_at_ms: from_sql_u64(
            raw.started_at_ms,
            StorageOperation::Query,
            schema_version,
            "started_at_ms",
        )?,
        ended_at_ms: raw
            .ended_at_ms
            .map(|value| {
                from_sql_u64(
                    value,
                    StorageOperation::Query,
                    schema_version,
                    "ended_at_ms",
                )
            })
            .transpose()?,
        listener_count: from_sql_u32(
            raw.listener_count,
            StorageOperation::Query,
            schema_version,
            "listener_count",
        )?,
        outcome,
        failure_code: raw.failure_code,
        failure_message: raw.failure_message,
    };
    validate_history(&history, schema_version)?;
    Ok(history)
}

fn validate_history(history: &SessionHistory, schema_version: u32) -> Result<(), StorageError> {
    let start = SessionStart {
        session_id: history.session_id.clone(),
        role: history.role,
        session_name: history.session_name.clone(),
        started_at_ms: history.started_at_ms,
    };
    start.validate().map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored session row is invalid: {error}"),
        )
    })?;
    match history.outcome {
        SessionOutcome::Active => validate_active(history, schema_version),
        SessionOutcome::Completed | SessionOutcome::Cancelled | SessionOutcome::Failed => {
            validate_terminal(history, schema_version)
        }
    }
}

fn validate_active(history: &SessionHistory, schema_version: u32) -> Result<(), StorageError> {
    if history.ended_at_ms.is_some()
        || history.failure_code.is_some()
        || history.failure_message.is_some()
    {
        return Err(corrupt_row(
            StorageOperation::Query,
            schema_version,
            "active session contains terminal fields",
        ));
    }
    Ok(())
}

fn validate_terminal(history: &SessionHistory, schema_version: u32) -> Result<(), StorageError> {
    let ended_at_ms = history.ended_at_ms.ok_or_else(|| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            "terminal session has no end timestamp",
        )
    })?;
    let end = SessionEnd {
        session_id: history.session_id.clone(),
        ended_at_ms,
        listener_count: history.listener_count,
        outcome: history.outcome,
        failure_code: history.failure_code.clone(),
        failure_message: history.failure_message.clone(),
    };
    end.validate().map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored terminal session row is invalid: {error}"),
        )
    })?;
    if ended_at_ms < history.started_at_ms {
        return Err(corrupt_row(
            StorageOperation::Query,
            schema_version,
            "session end timestamp precedes its start timestamp",
        ));
    }
    Ok(())
}
