use super::failure::DesktopPlatformFailure;
use silent_disco_core::domain::{AppRole, SessionId};
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::storage::{
    DatabaseClient, SessionEnd, SessionOutcome, SessionStart, StorageError,
};

pub(super) fn recover_interrupted(database: &DatabaseClient) -> Result<u32, StorageError> {
    database.interrupt_active_host_sessions(unix_time_ms())
}

pub(super) fn begin(
    database: &DatabaseClient,
    session_id: SessionId,
    session_name: String,
) -> Result<(), DesktopPlatformFailure> {
    database
        .begin_session(&SessionStart {
            session_id,
            role: AppRole::Host,
            session_name,
            started_at_ms: unix_time_ms(),
        })
        .map_err(|error| {
            DesktopPlatformFailure::new(
                CoreErrorCode::PlatformOperationFailed,
                format!("failed to begin host session history: {}", error.message),
                ErrorSeverity::Error,
                error.retryable,
            )
        })
}

pub(super) fn end(
    database: &DatabaseClient,
    session_id: SessionId,
    listener_count: u32,
) -> Result<(), DesktopPlatformFailure> {
    let updated = database
        .end_session(&SessionEnd {
            session_id,
            ended_at_ms: unix_time_ms(),
            listener_count,
            outcome: SessionOutcome::Completed,
            failure_code: None,
            failure_message: None,
        })
        .map_err(|error| {
            DesktopPlatformFailure::new(
                CoreErrorCode::PlatformOperationFailed,
                format!("failed to finish host session history: {}", error.message),
                ErrorSeverity::Error,
                error.retryable,
            )
        })?;
    if !updated {
        return Err(DesktopPlatformFailure::new(
            CoreErrorCode::PlatformOperationFailed,
            "active host session history row was not found",
            ErrorSeverity::Error,
            false,
        ));
    }
    Ok(())
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |duration| {
            u64::try_from(duration.as_millis())
                .unwrap_or(i64::MAX as u64)
                .min(i64::MAX as u64)
                .max(1)
        })
}

#[cfg(test)]
mod tests {
    use super::{begin, end, recover_interrupted};
    use silent_disco_core::domain::SessionId;
    use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker, SessionOutcome};

    #[test]
    fn host_history_is_written_and_finished_through_database_worker() {
        let directory = tempfile::tempdir().expect("temporary profile directory");
        let path = directory.path().join("domain.sqlite3");
        let worker = DatabaseWorker::start(DatabaseConfig::new(path).expect("database config"))
            .expect("database worker");
        let database = worker.client();
        let session_id = SessionId::new("desktop-session-test").expect("session id");

        begin(&database, session_id.clone(), "Studio Mix".to_owned())
            .expect("start session history");
        let active = database
            .get_session(&session_id)
            .expect("query session")
            .expect("active history row");
        assert_eq!(active.session_name, "Studio Mix");
        assert_eq!(active.outcome, SessionOutcome::Active);

        end(&database, session_id.clone(), 1).expect("end session history");
        let finished = database
            .get_session(&session_id)
            .expect("query finished session")
            .expect("finished history row");
        assert_eq!(finished.outcome, SessionOutcome::Completed);
        assert_eq!(finished.listener_count, 1);
        drop(database);
        worker.stop_and_join().expect("database worker shutdown");
    }

    #[test]
    fn startup_recovery_marks_abandoned_host_history_as_failed() {
        let directory = tempfile::tempdir().expect("temporary profile directory");
        let path = directory.path().join("domain.sqlite3");
        let worker = DatabaseWorker::start(DatabaseConfig::new(path).expect("database config"))
            .expect("database worker");
        let database = worker.client();
        let session_id = SessionId::new("desktop-session-abandoned").expect("session id");

        begin(
            &database,
            session_id.clone(),
            "Abandoned Session".to_owned(),
        )
        .expect("begin session history");
        assert_eq!(
            recover_interrupted(&database).expect("recover abandoned session"),
            1
        );
        let history = database
            .get_session(&session_id)
            .expect("query session")
            .expect("history row");
        assert_eq!(history.outcome, SessionOutcome::Failed);
        assert_eq!(
            history.failure_code.as_deref(),
            Some("desktop.host_session.interrupted")
        );
        assert_eq!(
            recover_interrupted(&database).expect("recovery is idempotent"),
            0
        );
        drop(database);
        worker.stop_and_join().expect("database worker shutdown");
    }
}
