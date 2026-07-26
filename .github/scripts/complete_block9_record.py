from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new)


def update_todo() -> None:
    path = Path("docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md")
    text = path.read_text()
    old = """## Block 9 — Integrate Rust database with Android and migrate current persisted data

### 9.1 Add Android path provider

Create `AndroidDatabasePathProvider` that:

- [ ] selects an application-private database path;
- [ ] creates only the parent directory;
- [ ] returns the complete path to Rust;
- [ ] never opens SQLite from Kotlin;
- [ ] applies Android backup policy intentionally.

### 9.2 Add one-time legacy import

Current tuning/trust data in `SharedPreferences` must be handled explicitly.

- [ ] Define a versioned `LegacyAndroidImport` record.
- [ ] Kotlin reads only the known legacy keys.
- [ ] Kotlin passes typed legacy values to Rust once.
- [ ] Rust validates and imports them transactionally.
- [ ] Rust records import completion in SQLite.
- [ ] Kotlin deletes legacy domain keys only after Rust reports committed success.
- [ ] Failure leaves legacy data intact and surfaces an error.
- [ ] Repeated startup is idempotent.

### 9.3 Remove direct domain persistence

- [ ] `MainViewModel` no longer writes tuning/trusted-device domain state to `SharedPreferences` after successful migration.
- [ ] Platform-only preferences may remain if documented.
- [ ] No silent fallback to old preferences if Rust database fails.

### 9.4 Add Android instrumentation tests

- [ ] First run creates database.
- [ ] Legacy settings import.
- [ ] Legacy trust import.
- [ ] Import failure preserves legacy values.
- [ ] Reopen loads Rust values.
- [ ] Database migration failure displays fatal/recoverable state.

**Acceptance:** All SQLite domain access is Rust-owned; Android has no production SQL and no duplicate domain persistence.
"""
    new = """## Block 9 — Integrate Rust database with Android and migrate current persisted data

### 9.1 Add Android path provider

Create `AndroidDatabasePathProvider` that:

- [x] selects an application-private database path;
- [x] creates only the parent directory;
- [x] returns the complete path to Rust;
- [x] never opens SQLite from Kotlin;
- [x] applies Android backup policy intentionally.

### 9.2 Add one-time legacy import

Current tuning/trust data in `SharedPreferences` must be handled explicitly.

- [x] Define a versioned `LegacyAndroidImport` record.
- [x] Kotlin reads only the known legacy keys.
- [x] Kotlin passes typed legacy values to Rust once.
- [x] Rust validates and imports them transactionally.
- [x] Rust records import completion in SQLite.
- [x] Kotlin deletes legacy domain keys only after Rust reports committed success.
- [x] Failure leaves legacy data intact and surfaces an error.
- [x] Repeated startup is idempotent.

### 9.3 Remove direct domain persistence

- [x] `MainViewModel` no longer writes tuning/trusted-device domain state to `SharedPreferences` after successful migration.
- [x] Platform-only preferences may remain if documented.
- [x] No silent fallback to old preferences if Rust database fails.

### 9.4 Add Android instrumentation tests

- [x] First run creates database.
- [x] Legacy settings import.
- [x] Legacy trust import.
- [x] Import failure preserves legacy values.
- [x] Reopen loads Rust values.
- [x] Database migration failure displays fatal/recoverable state.

**Acceptance:** All SQLite domain access is Rust-owned; Android has no production SQL and no duplicate domain persistence.

**Implementation status:** Complete. PR #35 merged as `5fc5ae966b1157b2cd5887c10d3522da81856f8f`. Permanent CI run `30187155765` passed Rust formatting, strict Clippy, all Rust tests, Android debug/PoC-debug/release and instrumentation-APK builds, four-ABI JNI packaging, Android unit tests, and Android lint. `AndroidRustDomainStoreInstrumentedTest` is compiled and packaged but physical-device execution is **NOT RUN**; device acceptance remains open until its command and device details are recorded.
"""
    path.write_text(replace_once(text, old, new, "Block 9 TODO section"))


def update_memory() -> None:
    path = Path("memory.md")
    text = path.read_text()
    prefix = "# memory.md — `silent_disco`\n\n"
    if not text.startswith(prefix):
        raise SystemExit("memory.md header changed unexpectedly")
    if "Rust-owned Android persistence Block 9 complete" in text:
        raise SystemExit("Block 9 memory entry already exists")
    timestamp = (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )
    entry = f"""## {timestamp} - GPT-5.6 Thinking - Rust-owned Android persistence Block 9 complete

- Added `AndroidDatabasePathProvider`, which uses `noBackupFilesDir/domain/silent-disco.sqlite3`, creates only the parent directory, returns the complete path to Rust, never opens SQLite, and intentionally excludes the domain database from Android backup.
- Added schema migration v2 and the `legacy_imports` marker. `LegacyAndroidImport` is versioned, validated before transaction start, imports tuning/trust atomically, records committed completion, rejects conflicting marker versions, rolls back invalid input, and is idempotent on repeated startup.
- Added a pinned `jni` 0.21.1 control-plane bridge for database open/close, typed legacy import, settings load/save, and trusted-device upsert/query. Rust owns all SQL, migrations, connection state, and worker lifecycle; Kotlin receives stable explicit status codes and no raw SQL surface.
- Added `AndroidRustDomainStore`. It reads only documented tuning keys and the documented dynamic `trusted:` namespace, rejects malformed known values visibly, preserves legacy values on failure, deletes legacy domain keys only after Rust reports committed import success, and retries cleanup after a failed Android preference commit.
- Removed direct tuning and trust persistence from `MainViewModel`. Persistence-dependent host, scan, join, tuning, and trust actions are blocked until Rust initialization succeeds. A database failure is shown as persistent-storage unavailable; there is no fallback to legacy preferences.
- Database shutdown is explicit and fail-visible. Initialization retains a database-close failure as a suppressed exception rather than dropping it, and `MainViewModel.onCleared()` does not convert close failure into log-only success.
- Added Android instrumentation source covering first-run database creation, tuning/trust import, reopen from Rust values, invalid import preservation, malformed trust preservation, and corrupt-database visibility. Legacy trust preferences contained only device IDs/booleans, so imported display names intentionally use the device ID until richer metadata is learned later.
- PR #35 merged as `5fc5ae966b1157b2cd5887c10d3522da81856f8f`. Permanent CI run `30187155765` passed Rust formatting, Clippy with warnings denied, all Rust tests, Android debug/PoC-debug/release and instrumentation-APK builds, four-ABI Rust/JNI packaging, Android unit tests, and Android lint.
- Physical execution of `AndroidRustDomainStoreInstrumentedTest` is **NOT RUN** because no Android device is attached. Do not claim device validation until the exact command, device model, Android version, ABI, and result are recorded.

"""
    path.write_text(prefix + entry + text[len(prefix) :])


def restore_ci_and_remove_script() -> None:
    ci_path = Path(".github/workflows/ci.yml")
    text = ci_path.read_text()
    branch_guard = (
        "    if: github.event_name != 'pull_request' || "
        "github.event.pull_request.head.ref != 'docs/rust-storage-block9-record'\n"
    )
    if text.count(branch_guard) != 2:
        raise SystemExit("temporary documentation branch guards changed unexpectedly")
    text = text.replace(branch_guard, "")
    start_marker = "  # BEGIN BLOCK 9 DOCUMENTATION JOB\n"
    end_marker = "  # END BLOCK 9 DOCUMENTATION JOB\n"
    if text.count(start_marker) != 1 or text.count(end_marker) != 1:
        raise SystemExit("Block 9 documentation job markers changed unexpectedly")
    start = text.index(start_marker)
    end = text.index(end_marker, start) + len(end_marker)
    ci_path.write_text((text[:start] + text[end:]).rstrip() + "\n")
    Path(".github/scripts/complete_block9_record.py").unlink()


def main() -> None:
    update_todo()
    update_memory()
    restore_ci_and_remove_script()


if __name__ == "__main__":
    main()
