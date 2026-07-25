from __future__ import annotations

from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def patch_error_codes() -> None:
    path = Path("rust/silent-disco-core/src/error.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    StorageOpenFailed = 6000,
    StorageMigrationFailed = 6001,
    StorageIntegrityFailed = 6002,
    StorageReadFailed = 6003,
    StorageWriteFailed = 6004,

    PlatformOperationFailed = 7000,
""",
        """    StorageOpenFailed = 6000,
    StorageMigrationFailed = 6001,
    StorageIntegrityFailed = 6002,
    StorageReadFailed = 6003,
    StorageWriteFailed = 6004,
    StoragePragmaFailed = 6005,
    StorageTransactionFailed = 6006,
    StorageConstraintViolation = 6007,
    StorageBusy = 6008,
    StorageCorrupt = 6009,
    StorageCloseFailed = 6010,
    StorageQueryFailed = 6011,

    PlatformOperationFailed = 7000,
""",
        "storage code definitions",
    )
    text = replace_once(
        text,
        """            Self::StorageOpenFailed
            | Self::StorageMigrationFailed
            | Self::StorageIntegrityFailed
            | Self::StorageReadFailed
            | Self::StorageWriteFailed => CoreSubsystem::Storage,
""",
        """            Self::StorageOpenFailed
            | Self::StorageMigrationFailed
            | Self::StorageIntegrityFailed
            | Self::StorageReadFailed
            | Self::StorageWriteFailed
            | Self::StoragePragmaFailed
            | Self::StorageTransactionFailed
            | Self::StorageConstraintViolation
            | Self::StorageBusy
            | Self::StorageCorrupt
            | Self::StorageCloseFailed
            | Self::StorageQueryFailed => CoreSubsystem::Storage,
""",
        "storage subsystem mapping",
    )
    text = replace_once(
        text,
        """            Self::StorageOpenFailed => "storage_open_failed",
            Self::StorageMigrationFailed => "storage_migration_failed",
            Self::StorageIntegrityFailed => "storage_integrity_failed",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed => "storage_write_failed",
            Self::PlatformOperationFailed => "platform_operation_failed",
""",
        """            Self::StorageOpenFailed => "storage_open_failed",
            Self::StorageMigrationFailed => "storage_migration_failed",
            Self::StorageIntegrityFailed => "storage_integrity_failed",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed => "storage_write_failed",
            Self::StoragePragmaFailed => "storage_pragma_failed",
            Self::StorageTransactionFailed => "storage_transaction_failed",
            Self::StorageConstraintViolation => "storage_constraint_violation",
            Self::StorageBusy => "storage_busy",
            Self::StorageCorrupt => "storage_corrupt",
            Self::StorageCloseFailed => "storage_close_failed",
            Self::StorageQueryFailed => "storage_query_failed",
            Self::PlatformOperationFailed => "platform_operation_failed",
""",
        "storage stable names",
    )
    path.write_text(text)


def remove_known_unused_import() -> None:
    path = Path("rust/silent-disco-core/src/storage/database.rs")
    text = path.read_text()
    text = replace_once(
        text,
        """    path::{Path, PathBuf},
""",
        """    path::PathBuf,
""",
        "database path import",
    )
    path.write_text(text)


def restore_ci_and_remove_script() -> None:
    path = Path(".github/workflows/ci.yml")
    text = path.read_text()
    skip = (
        "    if: github.event_name != 'pull_request' || "
        "github.event.pull_request.head.ref != 'feature/rust-sqlite-worker'\n"
    )
    if text.count(skip) != 2:
        raise SystemExit("temporary SQLite worker CI skips changed unexpectedly")
    text = text.replace(skip, "")
    start_marker = "  # BEGIN SQLITE WORKER BOOTSTRAP\n"
    end_marker = "  # END SQLITE WORKER BOOTSTRAP\n"
    if text.count(start_marker) != 1 or text.count(end_marker) != 1:
        raise SystemExit("SQLite bootstrap job markers changed unexpectedly")
    start = text.index(start_marker)
    end = text.index(end_marker, start) + len(end_marker)
    path.write_text((text[:start] + text[end:]).rstrip() + "\n")
    Path(".github/scripts/bootstrap_sqlite_worker.py").unlink()


def main() -> None:
    patch_error_codes()
    remove_known_unused_import()
    restore_ci_and_remove_script()


if __name__ == "__main__":
    main()
