from __future__ import annotations

from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def main() -> None:
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

    Path(".github/scripts/apply_storage_error_integrity_fix.py").unlink()
    Path(".github/workflows/storage-error-integrity-fix.yml").unlink()


if __name__ == "__main__":
    main()
