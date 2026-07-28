#!/usr/bin/env python3
"""Record validated shared-actor and Tauri desktop Block 10 completion."""

from pathlib import Path


def complete_block(path: str, heading: str, next_heading: str, status: str) -> None:
    target = Path(path)
    source = target.read_text()
    start = source.index(heading)
    end = source.index(next_heading, start)
    block = source[start:end]
    completed = block.replace("- [ ]", "- [x]")
    acceptance_index = completed.rfind("**Acceptance:")
    if acceptance_index < 0:
        raise SystemExit(f"acceptance marker missing for {heading}")
    acceptance_end = completed.find("\n", acceptance_index)
    if acceptance_end < 0:
        acceptance_end = len(completed)
    if status not in completed:
        completed = (
            completed[: acceptance_end + 1]
            + "\n"
            + status
            + "\n"
            + completed[acceptance_end + 1 :]
        )
    target.write_text(source[:start] + completed + source[end:])


complete_block(
    "docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md",
    "## Block 10 — Implement commands, events, effects, snapshots, and actor runtime",
    "## Block 11 — Add UniFFI API and generated Kotlin bindings",
    "**Implementation status:** Complete. PR #38 merged the authoritative actor runtime and its strict repair pass. Permanent Desktop CI run `30339287568` and repository CI run `30339287556` passed Rust formatting, Clippy with warnings denied, Rust tests, Android builds/tests/lint, generated-artifact checks, and Linux desktop bundle smoke validation. The actor remains platform-independent; UniFFI and Android `CoreFacade` work remain Block 11.",
)

complete_block(
    "docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md",
    "## Block 9 — Complete shared migration Block 10 before production desktop state",
    "## Block 10 — Implement direct `CoreHandle` ownership in Tauri",
    "**Implementation status:** Complete. The shared actor records/runtime are production code, remain free of desktop-specific types, and passed the host-independent actor tests plus the repository Rust/Android and desktop gates recorded for PR #38.",
)

complete_block(
    "docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md",
    "## Block 10 — Implement direct `CoreHandle` ownership in Tauri",
    "## Block 11 — Implement the Tauri notification channel",
    "**Implementation status:** Complete in PR #40. Tauri owns one real shared-core actor and one Rust database worker for the open profile; secure identity, storage, actor, and initial notification delivery must all succeed before `Ready`. Open/close work runs off the UI thread, duplicate opens fail, startup and shutdown cleanup is reverse-ordered and fail-visible, and the current snapshot preserves the authoritative revision. Guarded finalizer run `30393427074` passed desktop Rust formatting, strict Clippy, backend tests/check, generated bindings, Biome formatting/lint, TypeScript, frontend tests/build, and the repository source-size invariant.",
)

memory = Path("memory.md")
source = memory.read_text()
entry = """## 2026-07-28T19:54:33Z - GPT-5.6 Thinking - Tauri desktop Block 10 core ownership complete

- Completed shared migration Block 10 documentation and desktop Blocks 9–10 for `docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md`.
- Added direct Tauri ownership of one `CoreActorRuntime`, one `DatabaseWorker`, one exclusive `ProfileLease`, the public identity derived from OS-protected secret material, and a bounded notification buffer for each open production profile.
- Production identity uses the operating-system credential store and fails closed when unavailable, locked, malformed, or unwritable. There is no plaintext file, synthetic production identity, anonymous identity, or in-memory fallback.
- `open_profile` performs blocking startup off the Tauri/UI thread and does not report `Ready` until profile locking, secure identity, Rust storage, the authoritative actor, and initial snapshot delivery all succeed. `get_current_snapshot` reads the real actor snapshot and preserves its revision.
- Duplicate opens fail explicitly. Partial startup and normal close attempt actor, database, and profile-lock cleanup in reverse order; cleanup failures remain attached instead of overwriting the primary failure. Close is idempotent after closed/failed state cleanup.
- Added bounded DTOs and Rust-derived TypeScript bindings without exposing native paths, private key material, database handles, raw pointers, or audio payloads. Generated Tauri schemas are excluded from Biome because they are tool-owned artifacts; application and handwritten frontend files remain covered.
- Tests cover successful open/current snapshot, duplicate-open rejection, profile-lock lifetime, storage failure without fallback, observer setup failure, and idempotent shutdown after partial failure. Notification tests cover latest-snapshot coalescing and visible non-snapshot queue overflow.
- Guarded finalizer run `30393427074` passed `cargo fmt --check`, Clippy with warnings denied, desktop backend tests, `cargo check`, Rust-derived binding verification, Biome formatting/lint, TypeScript checks, frontend tests, production frontend build, and the tracked-source line-count gate before committing `e371813f144d81617b505d9435d58dd1c7d27994`.
- Actual Secret Service behavior in the user's Ubuntu desktop session and full application launch remain device/environment acceptance work. No physical desktop credential-store or Android-device result is claimed here.

"""
if entry not in source:
    marker = "# memory.md — `silent_disco`\n\n"
    if marker not in source:
        raise SystemExit("memory heading marker missing")
    memory.write_text(source.replace(marker, marker + entry, 1))
