from __future__ import annotations

from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def patch_error_integrity() -> None:
    path = Path("rust/silent-disco-core/src/storage/error.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    CoreError, CoreErrorCode, CoreSubsystem, ErrorContextEntry, ErrorSeverity,
""",
        """    CoreError, CoreErrorCode, ErrorContextEntry, ErrorSeverity,
""",
        "production CoreSubsystem import",
    )
    text = replace_once(
        text,
        """            subsystem: CoreSubsystem::Storage,
""",
        """            subsystem: self.kind.core_error_code().subsystem(),
""",
        "CoreError subsystem derivation",
    )
    marker = """    #[test]
    fn truncation_preserves_utf8_boundaries() {
"""
    test = """    #[test]
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

        let worker_stopped = StorageError::worker_stopped(
            StorageOperation::ReadMetadata,
            Some(0),
        )
        .to_core_error();
        assert_eq!(worker_stopped.code, CoreErrorCode::WorkerStopped);
        assert_eq!(worker_stopped.subsystem, CoreSubsystem::Runtime);
        assert_eq!(worker_stopped.subsystem, worker_stopped.code.subsystem());
    }

"""
    text = replace_once(text, marker, test + marker, "subsystem regression test marker")
    path.write_text(text)


def patch_worker_admission() -> None:
    path = Path("rust/silent-disco-core/src/storage/worker.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """        atomic::{AtomicU32, Ordering},
""",
        """        atomic::{AtomicBool, AtomicU32, Ordering},
""",
        "worker atomic imports",
    )
    text = replace_once(
        text,
        """pub struct DatabaseClient {
    sender: SyncSender<DatabaseCommand>,
    schema_version: Arc<AtomicU32>,
}
""",
        """pub struct DatabaseClient {
    sender: SyncSender<DatabaseCommand>,
    accepting_requests: Arc<AtomicBool>,
    schema_version: Arc<AtomicU32>,
}
""",
        "database client request admission field",
    )
    text = replace_once(
        text,
        """    ) -> Result<T, StorageError> {
        let (reply_sender, reply_receiver) = sync_channel(1);
""",
        """    ) -> Result<T, StorageError> {
        if !self.accepting_requests.load(Ordering::Acquire) {
            return Err(StorageError::worker_stopped(
                operation,
                Some(self.schema_version.load(Ordering::Acquire)),
            ));
        }
        let (reply_sender, reply_receiver) = sync_channel(1);
""",
        "request admission check",
    )
    text = replace_once(
        text,
        """    fn request_shutdown(&self) -> Result<(), StorageError> {
        let (reply_sender, reply_receiver) = sync_channel(1);
""",
        """    fn request_shutdown(&self) -> Result<(), StorageError> {
        if !self.accepting_requests.swap(false, Ordering::AcqRel) {
            return Err(StorageError::shutdown_in_progress());
        }
        let (reply_sender, reply_receiver) = sync_channel(1);
""",
        "shutdown admission close",
    )
    text = replace_once(
        text,
        """        let schema_version = Arc::new(AtomicU32::new(0));
        let thread_schema_version = Arc::clone(&schema_version);
""",
        """        let accepting_requests = Arc::new(AtomicBool::new(true));
        let schema_version = Arc::new(AtomicU32::new(0));
        let thread_schema_version = Arc::clone(&schema_version);
""",
        "worker admission state initialization",
    )
    text = replace_once(
        text,
        """                client: DatabaseClient {
                    sender: command_sender,
                    schema_version,
                },
""",
        """                client: DatabaseClient {
                    sender: command_sender,
                    accepting_requests,
                    schema_version,
                },
""",
        "worker client admission state",
    )
    path.write_text(text)


def main() -> None:
    patch_error_integrity()
    patch_worker_admission()

    Path(".github/scripts/apply_storage_error_integrity_fix.py").unlink()
    Path(".github/workflows/storage-error-integrity-fix.yml").unlink()


if __name__ == "__main__":
    main()
