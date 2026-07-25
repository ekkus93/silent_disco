from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new)


def update_todo() -> None:
    path = Path("docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md")
    text = path.read_text()
    text = replace_once(
        text,
        """- [ ] Queue is bounded.
- [ ] Full queue returns visible `StorageBusy`; no command is dropped.
- [ ] Worker has explicit start, stop, and join lifecycle.
- [ ] Database connection never crosses into the audio callback.
""",
        """- [x] Queue is bounded.
- [x] Full queue returns visible `StorageBusy`; no command is dropped.
- [x] Worker has explicit start, stop, and join lifecycle.
- [x] Database connection never crosses into the audio callback.
""",
        "Block 7.1 checklist",
    )
    text = replace_once(
        text,
        """- [ ] Enable and verify foreign keys.
- [ ] Request and verify WAL mode where supported.
- [ ] Set bounded busy timeout.
- [ ] Select and document synchronous policy.
- [ ] Record SQLite library version in diagnostics.
- [ ] Fail initialization if required durability settings cannot be established.
""",
        """- [x] Enable and verify foreign keys.
- [x] Request and verify WAL mode where supported.
- [x] Set bounded busy timeout.
- [x] Select and document synchronous policy.
- [x] Record SQLite library version in diagnostics.
- [x] Fail initialization if required durability settings cannot be established.
""",
        "Block 7.2 checklist",
    )
    text = replace_once(
        text,
        """- [ ] Map open, pragma, migration, query, transaction, constraint, busy, corruption, and close errors separately.
- [ ] Preserve operation and schema version context.
- [ ] No `unwrap` or `expect` on production database results.

**Acceptance:** Database worker tests prove serialized ownership, bounded queue behavior, close/join, and explicit failure mapping.
""",
        """- [x] Map open, pragma, migration, query, transaction, constraint, busy, corruption, and close errors separately.
- [x] Preserve operation and schema version context.
- [x] No `unwrap` or `expect` on production database results.

**Acceptance:** Database worker tests prove serialized ownership, bounded queue behavior, close/join, and explicit failure mapping.

**Implementation status:** Complete. Block 7 provides worker infrastructure and connection policy only. Schema migrations, tables, repositories, and Android data import remain deferred to Block 8 and later blocks.
""",
        "Block 7.3 checklist and status",
    )
    path.write_text(text)


def update_memory() -> None:
    path = Path("memory.md")
    text = path.read_text()
    prefix = "# memory.md — `silent_disco`\n\n"
    if not text.startswith(prefix):
        raise SystemExit("memory.md header changed unexpectedly")
    if "Rust SQLite worker Block 7 complete" in text:
        raise SystemExit("Block 7 memory entry already exists")
    timestamp = (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )
    entry = f"""## {timestamp} - GPT-5.6 Thinking - Rust SQLite worker Block 7 complete

- Added a Rust-owned SQLite worker in `silent-disco-core`; one dedicated thread owns the only connection and callers receive typed control-plane operations rather than raw SQL or connection access.
- The command queue is bounded with a default capacity of 32. Normal requests use nonblocking admission and return visible `StorageBusy` when full; accepted commands are tested to receive a result rather than being dropped.
- Shutdown closes request admission before queuing the shutdown command, making post-stop clients reject deterministically instead of racing into `ReplyDisconnected`. The worker exposes explicit start, checkpoint, stop, close, and join behavior; dropping an unjoined worker is fail-visible rather than silently detaching it.
- Pinned `rusqlite` 0.40.1 with bundled SQLite and committed the regenerated lockfile. Startup enables and verifies foreign keys, requires WAL, applies a 2,000 ms busy timeout, requires `synchronous=FULL`, records the SQLite library version and connection policy in diagnostics metadata, and fails initialization if any required policy cannot be established.
- Added separate storage categories for open, pragma, migration, query, transaction, constraint, busy/queue-full, corruption, close, thread start, stopped worker, panic, reply disconnect, and shutdown state. `CoreError` conversion preserves operation/schema context and derives subsystem from the stable error code to prevent code/subsystem mismatch.
- Tests use temporary database files and cover serialized thread ownership, bounded queue saturation, accepted-command completion, deterministic shutdown rejection, explicit stop/join, WAL-policy rejection, corruption detection, invalid configuration, SQLite error mapping, and stable error-subsystem integrity.
- PR #28 merged as `32ca46b1062b0e85f477f03d54541502145f348a`. CI run `30179055667` passed Rust formatting, Clippy with warnings denied, all Rust tests, debug/PoC-debug/release APK builds, instrumentation-test APK compilation, four-ABI Rust packaging, Android unit tests, and Android lint.
- Block 7 intentionally creates no schema or repository SQL. Ordered migrations, tables, repositories, and legacy Android data import remain Block 8 and later work.

"""
    path.write_text(prefix + entry + text[len(prefix) :])


def remove_temporary_files() -> None:
    Path(".github/scripts/record_sqlite_block7.py").unlink()
    Path(".github/workflows/record-sqlite-block7.yml").unlink()


def main() -> None:
    update_todo()
    update_memory()
    remove_temporary_files()


if __name__ == "__main__":
    main()
