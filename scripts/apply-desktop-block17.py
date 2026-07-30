#!/usr/bin/env python3
from __future__ import annotations

import json
from pathlib import Path
import subprocess

EVIDENCE_COMMIT = "0cecbc38cfca68620131ed4c072968896fac2e65"
VALIDATION_RUN = "30576293784"
TODO_PATH = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md")
SHARED_TODO_PATH = Path("docs/SILENT_DISCO_RUST_CORE_MIGRATION_TODO.md")
MEMORY_PATH = Path("memory.md")
RESULTS_PATH = Path("docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.json")


def run(*command: str) -> None:
    subprocess.run(command, check=True)


def output(*command: str) -> str:
    return subprocess.check_output(command, text=True).strip()


def verify_evidence() -> dict[str, object]:
    run("git", "cat-file", "-e", f"{EVIDENCE_COMMIT}^{{commit}}")
    run("git", "merge-base", "--is-ancestor", EVIDENCE_COMMIT, "HEAD")
    results = json.loads(RESULTS_PATH.read_text())
    candidate = results["candidate"]
    decision = results["decision"]
    if candidate["crate"] != "symphonia" or candidate["version"] != "0.6.0":
        raise SystemExit("unexpected Block 18 decoder candidate")
    if decision["ownership"] != "shared Rust streaming decoder":
        raise SystemExit("unexpected Block 18 ownership decision")
    return results


def complete_desktop_todo() -> None:
    text = TODO_PATH.read_text()
    start = text.index("## Block 18 — Resolve the decoder decision gate")
    end = text.index("## Block 19 — Implement bounded streaming decode", start)
    block = text[start:end]
    pending = block.count("- [ ]")
    if pending == 0:
        raise SystemExit("Desktop Block 18 is already checked")
    if pending != 23:
        raise SystemExit(f"expected 23 pending Desktop Block 18 entries, found {pending}")
    block = block.replace("- [ ]", "- [x]")
    marker = "**Acceptance:** One explicit decoder path is selected with executable evidence."
    if block.count(marker) != 1:
        raise SystemExit("Desktop Block 18 acceptance marker mismatch")
    evidence = (
        "**Completion evidence:** Symphonia `0.6.0` with minimal WAV/PCM, FLAC, MP3, "
        "and ID3 features was compiled and measured against deterministic valid, corrupt, "
        "truncated, oversized-metadata, and cancellation fixtures. Shared Rust streaming "
        f"decode was selected after the complete regression matrix passed in GitHub Actions "
        f"run `{VALIDATION_RUN}` from evidence commit `{EVIDENCE_COMMIT}`. Results are recorded "
        "in `docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.md`; measurements are "
        "specific to the CI host and are not universal product limits.\n\n"
    )
    TODO_PATH.write_text(text[:start] + block.replace(marker, evidence + marker) + text[end:])


def coordinate_shared_block() -> None:
    text = SHARED_TODO_PATH.read_text()
    marker = (
        "**Acceptance:** One documented production decoder ownership model exists with "
        "performance/device evidence."
    )
    note = (
        "**Desktop Block 18 coordination:** Path B (shared Rust decoding) is selected. The "
        "desktop Symphonia spike and decision record are complete, but shared Block 23 remains "
        "open until Android bridge overhead, mobile physical-device format parity, iOS file-access "
        "constraints, and removal of the temporary platform decoder path are recorded. No hidden "
        "fallback is introduced during that migration.\n\n"
    )
    if note.strip() in text:
        raise SystemExit("shared Block 23 coordination note already exists")
    if text.count(marker) != 1:
        raise SystemExit("shared Block 23 acceptance marker mismatch")
    SHARED_TODO_PATH.write_text(text.replace(marker, note + marker))


def record_memory(results: dict[str, object]) -> None:
    existing = MEMORY_PATH.read_text()
    heading = "## 2026-07-30 — Desktop Block 18 decoder decision complete"
    if heading in existing:
        raise SystemExit("Desktop Block 18 memory entry already exists")
    cases = results["cases"]
    features = ", ".join(f"`{feature}`" for feature in results["candidate"]["features"])
    entry = f"""

{heading}

- Evidence commit: `{EVIDENCE_COMMIT}`.
- Guarded validation run: `{VALIDATION_RUN}`.
- Selected decoder: `symphonia = 0.6.0`, default features disabled, features {features}; license `MPL-2.0`.
- Selected ownership: shared Rust streaming decoder (shared Block 23 Path B), with no automatic platform, HTML, Web Audio, TypeScript, or FFmpeg fallback.
- Initial formats: WAV/PCM, native FLAC, and MP3. Desktop Block 19 will convert source-native planar buffers incrementally into bounded 48 kHz stereo PCM16 little-endian chunks.
- Valid-fixture realtime factors on this CI host: WAV `{cases['wav']['realtime_factor']:.1f}x`, FLAC `{cases['flac']['realtime_factor']:.1f}x`, MP3 `{cases['mp3']['realtime_factor']:.1f}x`.
- Peak RSS on this CI host: WAV `{cases['wav']['peak_rss_kib'] / 1024:.1f}` MiB, FLAC `{cases['flac']['peak_rss_kib'] / 1024:.1f}` MiB, MP3 `{cases['mp3']['peak_rss_kib'] / 1024:.1f}` MiB, and the 2 MiB metadata MP3 `{cases['metadata']['peak_rss_kib'] / 1024:.1f}` MiB.
- Corrupt and truncated fixtures failed visibly; cooperative cancellation stopped at a decoder packet boundary. These measurements are environment-specific evidence, not product-wide performance limits.
- Shared Block 23 remains open for Android bridge overhead, physical mobile evidence, iOS file-access constraints, and removal of the temporary platform decoder path.
"""
    MEMORY_PATH.write_text(existing.rstrip() + entry)


def commit_completion() -> None:
    run("bash", "scripts/check-source-file-line-counts.sh")
    run("git", "rm", ".github/workflows/desktop-block17-observable.yml")
    run("git", "rm", "scripts/apply-desktop-block17.py")
    run("git", "add", "-A")
    run("git", "diff", "--cached", "--check")
    run("git", "config", "user.name", "github-actions[bot]")
    run(
        "git",
        "config",
        "user.email",
        "41898282+github-actions[bot]@users.noreply.github.com",
    )
    parent = output("git", "rev-parse", "HEAD")
    remote = output("git", "ls-remote", "origin", "refs/heads/master").split()[0]
    if remote != parent:
        raise SystemExit(f"master moved from {parent} to {remote}; refusing completion")
    run("git", "commit", "-m", "Complete Desktop Block 18 decoder decision")
    run("git", "push", "origin", "HEAD:master")


def main() -> None:
    results = verify_evidence()
    complete_desktop_todo()
    coordinate_shared_block()
    record_memory(results)
    commit_completion()


if __name__ == "__main__":
    main()
