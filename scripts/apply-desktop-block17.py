#!/usr/bin/env python3
from __future__ import annotations

import os
from pathlib import Path
import subprocess

BLOCK17_TRANSFORM_COMMIT = "f1aceea1a8813b0103ae6a8e1141ad91cc8d5aa8"
BLOCK17_VALIDATED_INPUT = "7948e62a6526a84c3b4fceacc7971acd9c8e9bbb"
BLOCK17_VALIDATION_RUN = "30576293784"


def run(*command: str, cwd: str | None = None) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def output(*command: str) -> str:
    return subprocess.check_output(command, text=True).strip()


def block17_pending_count() -> int:
    text = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md").read_text()
    start = text.index("## Block 17 — Implement atomic source staging")
    end = text.index("## Block 18 — Resolve the decoder decision gate", start)
    return text[start:end].count("- [ ]")


def verify_block17_validation_ancestry() -> None:
    run("git", "cat-file", "-e", f"{BLOCK17_VALIDATED_INPUT}^{{commit}}")
    run("git", "merge-base", "--is-ancestor", BLOCK17_VALIDATED_INPUT, "HEAD")
    run("git", "cat-file", "-e", f"{BLOCK17_TRANSFORM_COMMIT}^{{commit}}")


def apply_validated_block17_transform() -> None:
    source = subprocess.check_output(
        [
            "git",
            "show",
            f"{BLOCK17_TRANSFORM_COMMIT}:scripts/apply-desktop-block17.py",
        ],
        text=True,
    )
    namespace = {"__name__": "__main__", "__file__": "scripts/apply-desktop-block17.py"}
    exec(compile(source, "scripts/apply-desktop-block17.py", "exec"), namespace)


def record_block17_completion() -> None:
    todo = Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md")
    text = todo.read_text()
    start = text.index("## Block 17 — Implement atomic source staging")
    end = text.index("## Block 18 — Resolve the decoder decision gate", start)
    block = text[start:end]
    if block.count("- [ ]") != 19:
        raise SystemExit("Block 17 checklist changed during finalization")
    block = block.replace("- [ ]", "- [x]")
    evidence = (
        "**Completion evidence:** Atomic, content-addressed source staging; bounded progress; "
        "explicit cancellation; verified reuse; strict owned-temp startup cleanup; frontend "
        f"integration; and the complete regression matrix passed in GitHub Actions run "
        f"`{BLOCK17_VALIDATION_RUN}` from validated input commit "
        f"`{BLOCK17_VALIDATED_INPUT}`. Native file-dialog interaction was not performed by "
        "this CI run.\n\n"
    )
    marker = (
        "**Acceptance:** The core receives a stable app-owned source path with no destructive "
        "or silent recovery."
    )
    if block.count(marker) != 1:
        raise SystemExit("Block 17 acceptance marker mismatch")
    todo.write_text(text[:start] + block.replace(marker, evidence + marker) + text[end:])

    memory = Path("memory.md")
    existing = memory.read_text()
    heading = "## 2026-07-30 — Desktop Block 17 atomic source staging complete"
    if heading in existing:
        raise SystemExit("Block 17 completion entry already exists")
    entry = f"""

{heading}

- Validated input commit: `{BLOCK17_VALIDATED_INPUT}`.
- Guarded validation run: `{BLOCK17_VALIDATION_RUN}`.
- Source selection copies the inspected file into the active profile's `sources/` directory through an owned temporary file, fixed 64 KiB buffers, streaming SHA-256, length/signature verification, file and directory synchronization, and no-clobber atomic publication.
- Stable source IDs and filenames are content-addressed. Existing staged content is reused only after full regular-file, length, and hash verification; mismatches and collisions fail visibly without overwriting data.
- Staging supports bounded 10 Hz progress events, explicit cancellation, profile-close cancellation/join, and deterministic cleanup that preserves both primary and cleanup failures.
- Startup removes only strict, provably owned incomplete regular temporary files. Unrelated files, symbolic links, and non-file entries are never silently deleted.
- Tests cover success, cancellation, source failure during copy, destination write failure, hash mismatch, collision, verified reuse, incomplete-temp cleanup, cleanup refusal, original preservation, progress throttling, and cancellation control.
- Validation passed source-size enforcement; shared Rust format/strict Clippy/tests; Android builds, ABI packaging, unit tests, lint, and instrumentation; generated desktop bindings, format/lint/typecheck/tests/build; desktop Rust strict gates; exact lockfiles; and Linux Tauri bundle creation. Native file-dialog interaction on a physical desktop session remains unclaimed.
"""
    memory.write_text(existing.rstrip() + entry)


def configure_git() -> None:
    run("git", "config", "user.name", "github-actions[bot]")
    run(
        "git",
        "config",
        "user.email",
        "41898282+github-actions[bot]@users.noreply.github.com",
    )


def push_current_commit(parent: str) -> None:
    remote = output("git", "ls-remote", "origin", "refs/heads/master").split()[0]
    if remote != parent:
        raise SystemExit(f"master moved from {parent} to {remote}; refusing to push")
    run("git", "push", "origin", "HEAD:master")


def finalize_block17_if_needed() -> None:
    pending = block17_pending_count()
    if pending == 0:
        memory = Path("memory.md").read_text()
        if "## 2026-07-30 — Desktop Block 17 atomic source staging complete" not in memory:
            raise SystemExit("Block 17 is checked but its completion record is missing")
        return
    if pending != 19:
        raise SystemExit(f"unexpected pending Block 17 task count: {pending}")

    verify_block17_validation_ancestry()
    apply_validated_block17_transform()
    run("cargo", "fmt", "--manifest-path", "rust/Cargo.toml", "--all")
    run("cargo", "fmt", "--manifest-path", "desktop/src-tauri/Cargo.toml", "--all")
    run("npm", "install", "--package-lock-only", "--ignore-scripts", cwd="desktop")
    run("npm", "ci", cwd="desktop")
    run("npm", "run", "format", cwd="desktop")
    run("cargo", "generate-lockfile", cwd="desktop/src-tauri")
    run("bash", "scripts/check-source-file-line-counts.sh")
    record_block17_completion()

    for path in (
        ".github/workflows/desktop-block17-finalize-validated.yml",
        ".github/workflows/finalize-desktop-block17-push.yml",
        ".github/workflows/desktop-block17-workflow-run-finalize.yml",
        "scripts/finalize-desktop-block17.trigger",
    ):
        run("git", "rm", "--ignore-unmatch", path)
    run("git", "rm", "-r", "--ignore-unmatch", "scripts/block17-payload")

    run("git", "add", "-A")
    run("git", "diff", "--cached", "--check")
    configure_git()
    parent = output("git", "rev-parse", "HEAD")
    run("git", "commit", "-m", "Complete Desktop Block 17 atomic source staging")
    push_current_commit(parent)


def install_spike_dependencies() -> None:
    run("sudo", "apt-get", "update")
    run(
        "sudo",
        "apt-get",
        "install",
        "--yes",
        "--no-install-recommends",
        "ffmpeg",
        "time",
    )


def commit_block18_evidence() -> None:
    install_spike_dependencies()
    run("bash", "scripts/run-desktop-block18-spike.sh")
    run("bash", "scripts/check-source-file-line-counts.sh")

    evidence_paths = (
        "tools/decoder-spike/Cargo.lock",
        "docs/DESKTOP_BLOCK18_DECODER_DECISION.md",
        "docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.json",
        "docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.md",
    )
    run("git", "add", *evidence_paths)
    run("git", "diff", "--cached", "--check")
    if subprocess.run(["git", "diff", "--cached", "--quiet"], check=False).returncode == 0:
        return

    configure_git()
    parent = output("git", "rev-parse", "HEAD")
    run("git", "commit", "-m", "Record Desktop Block 18 decoder spike evidence")
    push_current_commit(parent)


def main() -> None:
    finalize_block17_if_needed()
    commit_block18_evidence()


if __name__ == "__main__":
    main()
