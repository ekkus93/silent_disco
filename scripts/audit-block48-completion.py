#!/usr/bin/env python3
"""Fail-closed Block 48 documentation/reference/ignored-test audit.

This gate intentionally uses only the Python standard library so it can run
before Node, Rust, Gradle, or Tauri dependencies are installed.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
GUIDANCE_FILES = (
    Path("README.md"),
    Path("CLAUDE.md"),
    Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_SPEC.md"),
    Path("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md"),
)
REFERENCE_ROOTS = (
    "app/",
    "desktop/",
    "docs/",
    "gradle/",
    "rust/",
    "scripts/",
    "tools/",
    ".github/",
)
SOURCE_EXTENSIONS = (
    "js",
    "json",
    "kt",
    "kts",
    "md",
    "mjs",
    "py",
    "rs",
    "sh",
    "toml",
    "ts",
    "tsx",
    "yaml",
    "yml",
)
BACKTICK_RE = re.compile(r"`([^`\n]+)`")
LINK_RE = re.compile(r"\]\(([^)#]+)(?:#[^)]+)?\)")
LINE_SUFFIX_RE = re.compile(
    r"^(.*\.(?:" + "|".join(SOURCE_EXTENSIONS) + r")):[0-9,-]+$"
)
IGNORE_RE = re.compile(r'^\s*#\[ignore\s*=\s*"([^"]*)"\]\s*$')


class AuditFailure(Exception):
    """One or more completion-audit invariants failed."""


def normalize_repo_reference(reference: str) -> Path | None:
    candidate = reference.strip()
    if not candidate.startswith(REFERENCE_ROOTS):
        return None
    if any(token in candidate for token in (" ", "\n", "*", "{", "}")):
        return None
    candidate = candidate.split("::", 1)[0]
    line_match = LINE_SUFFIX_RE.match(candidate)
    if line_match:
        candidate = line_match.group(1)
    return Path(candidate)


def audit_guidance_references() -> tuple[int, list[str]]:
    checked: set[Path] = set()
    failures: list[str] = []

    for relative in GUIDANCE_FILES:
        source = REPO_ROOT / relative
        if not source.is_file():
            failures.append(f"required guidance file is missing: {relative}")
            continue
        text = source.read_text(encoding="utf-8")

        for match in LINK_RE.finditer(text):
            target = match.group(1).strip()
            if "://" in target or target.startswith("#"):
                continue
            resolved = (source.parent / target).resolve()
            try:
                resolved.relative_to(REPO_ROOT)
            except ValueError:
                # Deliberate examples such as /tmp/... are not repository files.
                continue
            checked.add(resolved)
            if not resolved.exists():
                failures.append(
                    f"{relative}: markdown link target does not exist: {target}"
                )

        for match in BACKTICK_RE.finditer(text):
            normalized = normalize_repo_reference(match.group(1))
            if normalized is None:
                continue
            resolved = REPO_ROOT / normalized
            checked.add(resolved)
            if not resolved.exists():
                failures.append(
                    f"{relative}: exact repository path does not exist: {match.group(1)}"
                )

    return len(checked), failures


def audit_ignored_tests() -> tuple[int, list[str]]:
    checked = 0
    failures: list[str] = []
    for root in ("rust", "desktop/src-tauri/src"):
        for path in sorted((REPO_ROOT / root).rglob("*.rs")):
            for line_number, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                match = IGNORE_RE.match(line)
                if match is None:
                    continue
                checked += 1
                reason = match.group(1)
                if "reason:" not in reason:
                    failures.append(
                        f"{path.relative_to(REPO_ROOT)}:{line_number}: ignored test lacks reason:"
                    )
                if "owner:" not in reason:
                    failures.append(
                        f"{path.relative_to(REPO_ROOT)}:{line_number}: ignored test lacks owner:"
                    )

    return checked, failures


def audit_readme_release_sections() -> list[str]:
    text = (REPO_ROOT / "README.md").read_text(encoding="utf-8")
    required_fragments = (
        "## Prerequisites",
        "## Clean builds",
        "## Development launch",
        "## Production bundle",
        "## Test and quality gates",
        "## Physical desktop-to-Android interoperability",
        "## Lab Mode and deterministic scenarios",
        "## Diagnostics",
        "## Secure-store troubleshooting",
        "validate-block47-android-interoperability.sh",
        "npm run tauri build",
        "npm run tauri dev",
        "scripts/check-rust.sh",
        "pixel2api29DebugAndroidTest",
    )
    return [
        f"README.md is missing required Block 48 developer guidance: {fragment}"
        for fragment in required_fragments
        if fragment not in text
    ]


def main() -> int:
    reference_count, reference_failures = audit_guidance_references()
    ignored_count, ignored_failures = audit_ignored_tests()
    failures = reference_failures + ignored_failures + audit_readme_release_sections()

    if failures:
        print("Block 48 completion audit FAILED:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print(
        "Block 48 completion audit passed: "
        f"{reference_count} repository references exist; "
        f"{ignored_count} ignored Rust tests carry reason and owner; "
        "required developer-guidance sections are present."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
