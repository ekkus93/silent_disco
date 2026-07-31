#!/usr/bin/env python3
from __future__ import annotations

import os
from pathlib import Path

root = Path(__file__).resolve().parents[1]
run_id = os.environ["GITHUB_RUN_ID"]
input_sha = os.environ["BLOCK23_INPUT"]

todo_path = root / "docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md"
todo = todo_path.read_text(encoding="utf-8")
start = todo.index("## Block 23 — Implement desktop join approval and listener management UI")
end = todo.index("## Block 24 — First physical Android control interoperability", start)
section = todo[start:end]
section = section.replace("- [ ]", "- [x]")
marker = "**Acceptance:** Desktop listener management uses the shared delivery-first policy.\n"
evidence = (
    marker
    + "\n**Completion evidence:** Actions run `"
    + run_id
    + "` passed against direct-master input `"
    + input_sha
    + "`. The run validated revision-aware approval/rejection/removal, real pending-control delivery, trusted-device persistence, authoritative UI reconciliation, Linux bundle creation, shared Rust, Android builds/tests/lint, ABI packaging, and Android instrumentation. See `docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md`. Physical Android-to-desktop control interoperability remains Block 24.\n"
)
if marker not in section:
    raise SystemExit("Block 23 acceptance marker was not found")
section = section.replace(marker, evidence, 1)
if "- [ ]" in section:
    raise SystemExit("Block 23 still contains unchecked tasks")
todo_path.write_text(todo[:start] + section + todo[end:], encoding="utf-8")

(root / "docs/DESKTOP_BLOCK23_LISTENER_MANAGEMENT.md").write_text(
    f"""# Desktop Block 23 — Delivery-First Listener Management

**Status:** Complete  
**Validation run:** `{run_id}`  
**Validated direct-master input:** `{input_sha}`

## Implemented behavior

- The desktop exposes revision-aware `approve_join_request`, `reject_join_request`, and `remove_listener` commands. Request and listener identifiers are validated before actor submission.
- `DesktopCoreObserver` routes transport effects to the active host transport and storage effects to a dedicated bounded database-effect worker. Neither effect category is forwarded to React for execution.
- The host transport owns a bounded outbound effect queue on the same thread that owns the shared `HostTransportNode`. Approval, rejection, and disconnect messages use the identified control peer and submit exact `DeliveryCompleted` evidence to the authoritative actor.
- Failed targeted sends report one intended peer, zero successful peers, and one failed peer. The actor therefore keeps the join request or listener visible and publishes a structured delivery failure rather than claiming success.
- Trusted-device persistence completes through the Rust `DatabaseWorker` before approval delivery is emitted. Existing trusted-device metadata is updated transactionally and storage failures remain correlated to the original operation.
- The host-session DTO now includes request age, synchronization confidence, offset/RTT/drift summaries, last delivery evidence, recoverable capability, and Rust-derived removal permission.
- `HostSessionScreen` keeps requests and listeners visible while commands are pending. It changes presentation only after a newer authoritative snapshot confirms completion or failure.
- `ListenerDetailScreen.tsx` exposes lifecycle, trust, last contact, synchronization, delivery state, retry/resync capability, structured errors, and a core-gated removal action.

## Automated evidence

The guarded run executed:

- focused desktop backend tests for command validation, DTO projection, storage effects, real socket approval delivery, missing-peer delivery failure, and the Block 22 loopback regression;
- frontend tests for delivery-confirmed approval, zero-recipient failure, partial delivery, rejection/stale failure, trusted-device policy, duplicate-click prevention, persistent errors, listener detail, and removal confirmation;
- generated-binding regeneration and stale-binding verification;
- shared Rust formatting, strict Clippy, and all-feature tests;
- desktop formatting, lint, TypeScript, all Vitest tests, production frontend build, backend formatting, strict Clippy, all-feature tests/check, and Linux Tauri bundle build;
- Android debug, POC, release, and instrumentation builds, unit tests, lint, four-ABI native-library checks, and the managed Pixel 2 API 29 instrumentation suite;
- source-file line-count and lockfile invariants.

## Scope boundary

This block proves desktop-owned listener admission and management, including real control-message delivery through the shared socket transport. It does not claim physical Android interoperability. Connecting a physical Android listener to the desktop host over the LAN and recording that evidence is Desktop Block 24.
""",
    encoding="utf-8",
)

memory_path = root / "memory.md"
memory = memory_path.read_text(encoding="utf-8")
entry = f"""

## 2026-07-31 — Desktop Block 23 listener management complete

- Completed revision-aware desktop approval, rejection, and listener removal on direct `master`.
- Added bounded desktop transport-effect and storage-effect execution. Delivery and persistence failures are correlated and fail-visible; React never executes transport or storage effects.
- Added authoritative request age, listener synchronization/delivery details, pending-operation reconciliation, trusted-device policy, duplicate-action prevention, and accessible listener-management UI.
- Guarded Actions run `{run_id}` passed against exact input `{input_sha}` with the complete Rust, desktop, Linux bundle, Android build/test/lint/ABI, managed-device instrumentation, generated-binding, lockfile, and source-size matrix.
- Physical Android control-plane interoperability remains Desktop Block 24.
"""
if "Desktop Block 23 listener management complete" not in memory:
    memory_path.write_text(memory.rstrip() + entry + "\n", encoding="utf-8")
