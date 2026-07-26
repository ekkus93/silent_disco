use rusqlite::{Connection, params};

use crate::domain::{DiagnosticRunId, SessionId};

use super::{
    error::{StorageError, StorageOperation, map_sqlite_error},
    models::{DiagnosticExport, DiagnosticQuery, DiagnosticRunSummary},
    repository_support::{corrupt_row, from_sql_u64, invalid_model, to_sql_i64},
};

struct RawDiagnosticRun {
    run_id: String,
    session_id: Option<String>,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    summary_json: String,
}

pub(crate) fn insert(
    connection: &mut Connection,
    run: &DiagnosticRunSummary,
    schema_version: u32,
) -> Result<(), StorageError> {
    run.validate()
        .map_err(|error| invalid_model(StorageOperation::Transaction, schema_version, error))?;
    verify_json(connection, run.summary_json.as_str(), schema_version)?;
    let started_at_ms = to_sql_i64(
        run.started_at_ms,
        StorageOperation::Transaction,
        schema_version,
        "started_at_ms",
    )?;
    let ended_at_ms = run
        .ended_at_ms
        .map(|value| {
            to_sql_i64(
                value,
                StorageOperation::Transaction,
                schema_version,
                "ended_at_ms",
            )
        })
        .transpose()?;
    let transaction = connection.transaction().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })?;
    transaction
        .execute(
            "INSERT INTO diagnostic_runs (
                 run_id,
                 session_id,
                 started_at_ms,
                 ended_at_ms,
                 summary_json
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run.run_id.as_str(),
                run.session_id.as_ref().map(SessionId::as_str),
                started_at_ms,
                ended_at_ms,
                run.summary_json.as_str(),
            ],
        )
        .map_err(|error| {
            map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
        })?;
    transaction.commit().map_err(|error| {
        map_sqlite_error(StorageOperation::Transaction, Some(schema_version), &error)
    })
}

pub(crate) fn query(
    connection: &Connection,
    query: &DiagnosticQuery,
    schema_version: u32,
) -> Result<Vec<DiagnosticRunSummary>, StorageError> {
    query
        .validate()
        .map_err(|error| invalid_model(StorageOperation::Query, schema_version, error))?;
    match &query.session_id {
        Some(session_id) => query_for_session(connection, session_id, query.limit, schema_version),
        None => query_all_with_limit(connection, query.limit, schema_version),
    }
}

pub(crate) fn export(
    connection: &Connection,
    schema_version: u32,
) -> Result<DiagnosticExport, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT run_id, session_id, started_at_ms, ended_at_ms, summary_json
             FROM diagnostic_runs
             ORDER BY started_at_ms ASC, run_id ASC",
        )
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let rows = statement
        .query_map([], read_raw)
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let runs = collect_rows(rows, schema_version)?;
    Ok(DiagnosticExport {
        schema_version,
        runs,
    })
}

fn verify_json(
    connection: &Connection,
    summary_json: &str,
    schema_version: u32,
) -> Result<(), StorageError> {
    let valid = connection
        .query_row("SELECT json_valid(?1)", [summary_json], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    if valid != 1 {
        return Err(invalid_model(
            StorageOperation::Transaction,
            schema_version,
            "diagnostic summary is not valid JSON",
        ));
    }
    Ok(())
}

fn query_for_session(
    connection: &Connection,
    session_id: &SessionId,
    limit: u32,
    schema_version: u32,
) -> Result<Vec<DiagnosticRunSummary>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT run_id, session_id, started_at_ms, ended_at_ms, summary_json
             FROM diagnostic_runs
             WHERE session_id = ?1
             ORDER BY started_at_ms DESC, run_id ASC
             LIMIT ?2",
        )
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let rows = statement
        .query_map(params![session_id.as_str(), i64::from(limit)], read_raw)
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    collect_rows(rows, schema_version)
}

fn query_all_with_limit(
    connection: &Connection,
    limit: u32,
    schema_version: u32,
) -> Result<Vec<DiagnosticRunSummary>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT run_id, session_id, started_at_ms, ended_at_ms, summary_json
             FROM diagnostic_runs
             ORDER BY started_at_ms DESC, run_id ASC
             LIMIT ?1",
        )
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    let rows = statement
        .query_map([i64::from(limit)], read_raw)
        .map_err(|error| map_sqlite_error(StorageOperation::Query, Some(schema_version), &error))?;
    collect_rows(rows, schema_version)
}

fn collect_rows<F>(
    rows: rusqlite::MappedRows<'_, F>,
    schema_version: u32,
) -> Result<Vec<DiagnosticRunSummary>, StorageError>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<RawDiagnosticRun>,
{
    let mut runs = Vec::new();
    for row in rows {
        let raw = row.map_err(|error| {
            map_sqlite_error(StorageOperation::Query, Some(schema_version), &error)
        })?;
        runs.push(decode(raw, schema_version)?);
    }
    Ok(runs)
}

fn read_raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawDiagnosticRun> {
    Ok(RawDiagnosticRun {
        run_id: row.get(0)?,
        session_id: row.get(1)?,
        started_at_ms: row.get(2)?,
        ended_at_ms: row.get(3)?,
        summary_json: row.get(4)?,
    })
}

fn decode(
    raw: RawDiagnosticRun,
    schema_version: u32,
) -> Result<DiagnosticRunSummary, StorageError> {
    let run_id = DiagnosticRunId::new(raw.run_id).map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored diagnostic-run identifier is invalid: {error}"),
        )
    })?;
    let session_id = raw
        .session_id
        .map(|value| {
            SessionId::new(value).map_err(|error| {
                corrupt_row(
                    StorageOperation::Query,
                    schema_version,
                    format!("stored diagnostic session identifier is invalid: {error}"),
                )
            })
        })
        .transpose()?;
    let run = DiagnosticRunSummary {
        run_id,
        session_id,
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
        summary_json: raw.summary_json,
    };
    run.validate().map_err(|error| {
        corrupt_row(
            StorageOperation::Query,
            schema_version,
            format!("stored diagnostic-run row is invalid: {error}"),
        )
    })?;
    Ok(run)
}
