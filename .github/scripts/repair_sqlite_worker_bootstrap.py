from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def main() -> None:
    bootstrap_path = Path(".github/scripts/bootstrap_sqlite_worker.py")
    text = bootstrap_path.read_text()
    old = '''    text = text.replace(
        "map_sqlite_error(StorageOperation::OpenDatabase, None, error)",
        "map_sqlite_error(StorageOperation::OpenDatabase, None, &error)",
    )
    text = text.replace(
        "map_sqlite_error(StorageOperation::ReadMetadata, None, error)",
        "map_sqlite_error(StorageOperation::ReadMetadata, None, &error)",
    )
    text, multiline_count = re.subn(
        r"\\n(?P<indent>\\s*)error,\\n(?P<close>\\s*)\\)\\)",
        r"\\n\\g<indent>&error,\\n\\g<close>))",
        text,
    )
    if multiline_count != 5:
        raise SystemExit(
            f"multiline SQLite error arguments: expected 5, found {multiline_count}"
        )
'''
    new = '''    single_count = text.count(", error)")
    if single_count != 8:
        raise SystemExit(
            f"single-line SQLite error arguments: expected 8, found {single_count}"
        )
    text = text.replace(", error)", ", &error)")
    text = replace_once(
        text,
        "Err((_connection, &error))",
        "Err((_connection, error))",
        "SQLite close error ownership pattern",
    )
    text, multiline_count = re.subn(
        r"(?m)^(\\s*)error,$",
        r"\\1&error,",
        text,
    )
    if multiline_count != 4:
        raise SystemExit(
            f"multiline SQLite error arguments: expected 4, found {multiline_count}"
        )
'''
    bootstrap_path.write_text(
        replace_once(text, old, new, "SQLite error borrowing guard")
    )

    subprocess.run([sys.executable, str(bootstrap_path)], check=True)

    database_path = Path("rust/silent-disco-core/src/storage/database.rs")
    database_text = database_path.read_text()
    database_path.write_text(
        replace_once(
            database_text,
            """        assert_eq!(metadata.busy_timeout_ms, DEFAULT_BUSY_TIMEOUT_MS as u32);
""",
            """        assert_eq!(
            u64::from(metadata.busy_timeout_ms),
            DEFAULT_BUSY_TIMEOUT_MS
        );
""",
            "busy timeout widening assertion",
        )
    )

    for temporary_path in (
        Path(".github/workflows/sqlite-worker-diagnostics.yml"),
        Path(".github/workflows/sqlite-worker-bootstrap-repair.yml"),
        Path(".github/scripts/repair_sqlite_worker_bootstrap.py"),
    ):
        temporary_path.unlink()


if __name__ == "__main__":
    main()
