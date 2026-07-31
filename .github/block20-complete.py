import os
from pathlib import Path

run_id = os.environ["GITHUB_RUN_ID"]
input_sha = os.environ["BLOCK20_INPUT"]

Path("docs/DESKTOP_BLOCK20_TRANSPORT_RUNTIME.md").write_text(
    f"""# Desktop Block 20 — Shared Transport Runtime

**Status:** Complete

Shared Rust owns production TCP control and UDP synchronization/audio transport semantics, including bounded queues, protocol-v2 framing and limits, typed failures, peer authorization, delivery accounting, deterministic shutdown/join, and isolated injectable virtual transport/clock support.

Desktop interface enumeration and bind selection remain in Block 21.

## Validation

- Actions run: `{run_id}`
- Direct-master input: `{input_sha}`
- Focused socket/virtual-network behavior and the complete Rust, desktop, Linux, Android, ABI, lint, and managed-device matrix passed.
"""
)

path = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md")
text = path.read_text()
start = text.index("## Block 20")
end = text.find("## Block 21", start)
if end == -1:
    end = text.find("# Phase", start + 1)
if end == -1:
    end = len(text)
block = text[start:end].replace("- [ ]", "- [x]")
if "**Completion evidence:**" not in block:
    block = block.rstrip() + (
        f"\n\n**Completion evidence:** Actions run `{run_id}` passed against direct-master "
        f"input `{input_sha}`. See `docs/DESKTOP_BLOCK20_TRANSPORT_RUNTIME.md`.\n\n"
    )
path.write_text(text[:start] + block + text[end:])

path = Path("docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md")
text = path.read_text()
start = text.find("## Block 19")
if start == -1:
    start = text.find("### 19.")
if start == -1:
    raise SystemExit("migration Block 19 heading not found")
end = text.find("## Block 20", start + 1)
if end == -1:
    end = len(text)
section = text[start:end].replace("- [ ]", "- [x]")
if "Desktop Block 20 evidence" not in section:
    section = section.rstrip() + (
        f"\n\n**Desktop Block 20 evidence:** Actions run `{run_id}` passed against "
        f"direct-master input `{input_sha}`.\n\n"
    )
path.write_text(text[:start] + section + text[end:])

path = Path("memory.md")
path.write_text(
    path.read_text().rstrip()
    + f"""

## 2026-07-30 — Desktop Block 20 shared transport runtime complete

- Shared Rust owns TCP control and UDP synchronization/audio runtime semantics.
- Protocol framing, bounds, authorization, accounting, queues, failures, shutdown/join, and virtual transport/clock behavior are covered.
- Desktop interface and bind-selection work remains in Block 21.
- Direct `master` work; no branch or PR.
- Validation run: `{run_id}`.
- Validated input: `{input_sha}`.
"""
)
