from __future__ import annotations

from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def patch_models() -> None:
    path = Path("rust/silent-disco-core/src/storage/models.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """pub const MAX_DIAGNOSTIC_SUMMARY_BYTES: usize = 1_048_576;
pub const MAX_DIAGNOSTIC_QUERY_LIMIT: u32 = 1_000;
""",
        """pub const MAX_DIAGNOSTIC_SUMMARY_BYTES: usize = 262_144;
pub const MAX_DIAGNOSTIC_QUERY_LIMIT: u32 = 32;
pub const MAX_DIAGNOSTIC_EXPORT_LIMIT: u32 = 32;
""",
        "diagnostic bounds",
    )
    text = replace_once(
        text,
        """/// Deterministic typed diagnostic export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExport {
    pub schema_version: u32,
    pub runs: Vec<DiagnosticRunSummary>,
}
""",
        """/// Stable exclusive cursor for deterministic diagnostic export pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExportCursor {
    pub started_at_ms: u64,
    pub run_id: DiagnosticRunId,
}

impl DiagnosticExportCursor {
    /// Validates the cursor timestamp.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure when the timestamp cannot be stored.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_sql_millis(self.started_at_ms, StorageModelValidationError::Timestamp)
    }
}

/// Bounded request for one deterministic diagnostic export page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExportRequest {
    pub session_id: Option<SessionId>,
    pub cursor: Option<DiagnosticExportCursor>,
    pub limit: u32,
}

impl DiagnosticExportRequest {
    /// Validates the page limit and optional cursor.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for an invalid page request.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        if self.limit == 0 || self.limit > MAX_DIAGNOSTIC_EXPORT_LIMIT {
            return Err(StorageModelValidationError::DiagnosticExportLimit);
        }
        if let Some(cursor) = &self.cursor {
            cursor.validate()?;
        }
        Ok(())
    }
}

/// One bounded deterministic diagnostic export page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExport {
    pub schema_version: u32,
    pub runs: Vec<DiagnosticRunSummary>,
    pub next_cursor: Option<DiagnosticExportCursor>,
}
""",
        "diagnostic export records",
    )
    text = replace_once(
        text,
        """    DiagnosticSummary,
    DiagnosticQueryLimit,
}
""",
        """    DiagnosticSummary,
    DiagnosticQueryLimit,
    DiagnosticExportLimit,
}
""",
        "diagnostic export validation variant",
    )
    text = replace_once(
        text,
        """            Self::DiagnosticQueryLimit => "diagnostic query limit is outside the supported range",
""",
        """            Self::DiagnosticQueryLimit => "diagnostic query limit is outside the supported range",
            Self::DiagnosticExportLimit => {
                "diagnostic export page limit is outside the supported range"
            }
""",
        "diagnostic export validation message",
    )
    path.write_text(text)


def patch_migration() -> None:
    path = Path("rust/silent-disco-core/src/storage/migrations.rs")
    text = path.read_text()
    text = replace_once(
        text,
        "length(summary_json) BETWEEN 1 AND 1048576",
        "length(summary_json) BETWEEN 1 AND 262144",
        "diagnostic schema payload bound",
    )
    path.write_text(text)


def patch_exports() -> None:
    path = Path("rust/silent-disco-core/src/storage/mod.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    DiagnosticExport, DiagnosticQuery, DiagnosticRunSummary, MAX_DIAGNOSTIC_QUERY_LIMIT,
    MAX_DIAGNOSTIC_SUMMARY_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_FAILURE_CODE_BYTES,
""",
        """    DiagnosticExport, DiagnosticExportCursor, DiagnosticExportRequest, DiagnosticQuery,
    DiagnosticRunSummary, MAX_DIAGNOSTIC_EXPORT_LIMIT, MAX_DIAGNOSTIC_QUERY_LIMIT,
    MAX_DIAGNOSTIC_SUMMARY_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_FAILURE_CODE_BYTES,
""",
        "storage public exports",
    )
    path.write_text(text)


def patch_database() -> None:
    path = Path("rust/silent-disco-core/src/storage/database.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """        DiagnosticExport, DiagnosticQuery, DiagnosticRunSummary, SessionEnd, SessionHistory,
        SessionStart, SessionUpdate, StoredSettings, TrustedDevice,
""",
        """        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
        SessionEnd, SessionHistory, SessionStart, SessionUpdate, StoredSettings, TrustedDevice,
""",
        "database diagnostic imports",
    )
    text = replace_once(
        text,
        """    pub(crate) fn export_diagnostic_runs(&self) -> Result<DiagnosticExport, StorageError> {
        diagnostics_repository::export(&self.connection, self.metadata.schema_version)
    }
""",
        """    pub(crate) fn export_diagnostic_runs(
        &self,
        request: &DiagnosticExportRequest,
    ) -> Result<DiagnosticExport, StorageError> {
        diagnostics_repository::export(
            &self.connection,
            request,
            self.metadata.schema_version,
        )
    }
""",
        "database bounded export method",
    )
    path.write_text(text)


def patch_worker() -> None:
    path = Path("rust/silent-disco-core/src/storage/worker.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """        DiagnosticExport, DiagnosticQuery, DiagnosticRunSummary, SessionEnd, SessionHistory,
        SessionStart, SessionUpdate, StoredSettings, TrustedDevice,
""",
        """        DiagnosticExport, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
        SessionEnd, SessionHistory, SessionStart, SessionUpdate, StoredSettings, TrustedDevice,
""",
        "worker diagnostic imports",
    )
    text = replace_once(
        text,
        """    ExportDiagnosticRuns {
        reply: DatabaseReply<DiagnosticExport>,
    },
""",
        """    ExportDiagnosticRuns {
        request: DiagnosticExportRequest,
        reply: DatabaseReply<DiagnosticExport>,
    },
""",
        "worker export command",
    )
    text = replace_once(
        text,
        """    pub fn export_diagnostic_runs(&self) -> Result<DiagnosticExport, StorageError> {
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::ExportDiagnosticRuns { reply }
        })
    }
""",
        """    pub fn export_diagnostic_runs(
        &self,
        request: &DiagnosticExportRequest,
    ) -> Result<DiagnosticExport, StorageError> {
        let request = request.clone();
        self.request(StorageOperation::Query, |reply| {
            DatabaseCommand::ExportDiagnosticRuns { request, reply }
        })
    }
""",
        "worker bounded export client",
    )
    text = replace_once(
        text,
        """        DatabaseCommand::ExportDiagnosticRuns { reply } => {
            process_export_diagnostic_runs(&reply, connection, version)?;
        }
""",
        """        DatabaseCommand::ExportDiagnosticRuns { request, reply } => {
            process_export_diagnostic_runs(&reply, &request, connection, version)?;
        }
""",
        "worker export dispatch",
    )
    text = replace_once(
        text,
        """fn process_export_diagnostic_runs(
    reply: &DatabaseReply<DiagnosticExport>,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(DatabaseConnection::export_diagnostic_runs);
""",
        """fn process_export_diagnostic_runs(
    reply: &DatabaseReply<DiagnosticExport>,
    request: &DiagnosticExportRequest,
    connection: &mut Option<DatabaseConnection>,
    schema_version: u32,
) -> Result<(), StorageError> {
    let result = connection_ref(connection.as_ref(), StorageOperation::Query, schema_version)
        .and_then(|database| database.export_diagnostic_runs(request));
""",
        "worker export handler",
    )
    text = replace_once(
        text,
        """fn send_reply<T: Clone>(
    reply: &DatabaseReply<T>,
    result: Result<T, StorageError>,
    connection: &mut Option<DatabaseConnection>,
    operation: StorageOperation,
    schema_version: u32,
) -> Result<(), StorageError> {
    if reply.send(result.clone()).is_err() {
        return close_after_reply_failure(
            connection.take(),
            result.err(),
            operation,
            schema_version,
        );
    }
    Ok(())
}
""",
        """fn send_reply<T>(
    reply: &DatabaseReply<T>,
    result: Result<T, StorageError>,
    connection: &mut Option<DatabaseConnection>,
    operation: StorageOperation,
    schema_version: u32,
) -> Result<(), StorageError> {
    match reply.send(result) {
        Ok(()) => Ok(()),
        Err(error) => close_after_reply_failure(
            connection.take(),
            error.0.err(),
            operation,
            schema_version,
        ),
    }
}
""",
        "move-only worker replies",
    )
    text = replace_once(
        text,
        """            DatabaseConfig, DiagnosticQuery, DiagnosticRunSummary, SessionEnd, SessionOutcome,
            SessionStart, SessionUpdate, StorageErrorKind, StoredSettings, TrustedDevice,
""",
        """            DatabaseConfig, DiagnosticExportRequest, DiagnosticQuery, DiagnosticRunSummary,
            SessionEnd, SessionOutcome, SessionStart, SessionUpdate, StorageErrorKind,
            StoredSettings, TrustedDevice,
""",
        "worker test imports",
    )
    text = replace_once(
        text,
        """        assert_eq!(
            client
                .export_diagnostic_runs()
                .expect("export diagnostics")
                .runs,
            vec![diagnostic]
        );
""",
        """        let export = client
            .export_diagnostic_runs(&DiagnosticExportRequest {
                session_id: None,
                cursor: None,
                limit: 10,
            })
            .expect("export diagnostics");
        assert_eq!(export.runs, vec![diagnostic]);
        assert_eq!(export.next_cursor, None);
""",
        "worker round-trip export assertion",
    )
    marker = """    #[test]
    fn duplicate_session_maps_to_constraint_violation() {
"""
    pagination_test = """    #[test]
    fn diagnostic_export_is_bounded_and_cursor_paginated() {
        let test_path = TestDatabasePath::new("worker-export-pagination");
        let worker = DatabaseWorker::start(
            DatabaseConfig::new(test_path.path()).expect("valid worker config"),
        )
        .expect("worker starts");
        let client = worker.client();
        let session = sample_session_start();
        client.begin_session(&session).expect("begin session");

        for (suffix, started_at_ms) in [("a", 110), ("b", 120), ("c", 130)] {
            client
                .insert_diagnostic_run(&DiagnosticRunSummary {
                    run_id: DiagnosticRunId::new(format!("diagnostic-{suffix}"))
                        .expect("valid diagnostic identifier"),
                    session_id: Some(session.session_id.clone()),
                    started_at_ms,
                    ended_at_ms: Some(started_at_ms + 1),
                    summary_json: format!(r#"{{"run":"{suffix}"}}"#),
                })
                .expect("insert diagnostic run");
        }

        let first = client
            .export_diagnostic_runs(&DiagnosticExportRequest {
                session_id: Some(session.session_id.clone()),
                cursor: None,
                limit: 2,
            })
            .expect("first export page");
        assert_eq!(first.runs.len(), 2);
        let cursor = first.next_cursor.expect("more rows remain");

        let second = client
            .export_diagnostic_runs(&DiagnosticExportRequest {
                session_id: Some(session.session_id),
                cursor: Some(cursor),
                limit: 2,
            })
            .expect("second export page");
        assert_eq!(second.runs.len(), 1);
        assert_eq!(second.next_cursor, None);
        worker.stop_and_join().expect("worker closes and joins");
    }

"""
    text = replace_once(
        text,
        marker,
        pagination_test + marker,
        "worker pagination test insertion",
    )
    path.write_text(text)


def main() -> None:
    patch_models()
    patch_migration()
    patch_exports()
    patch_database()
    patch_worker()
    Path(".github/scripts/harden_block8_diagnostics.py").unlink()
    Path(".github/workflows/harden-block8-diagnostics.yml").unlink()


if __name__ == "__main__":
    main()
