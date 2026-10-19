#!/usr/bin/env python3
"""Fail CI if a routine Actions job could run on the owner's machine.

The only exception is the explicitly disabled, manual hardware-staging job.
Use a conservative runner-literal check: dynamic expressions and runner labels
must be reviewed instead of silently passing this guard.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

STAGE_WORKFLOW = "block46-self-hosted-stage.yml"
HOSTED_UBUNTU = re.compile(r"ubuntu-(?:latest|[0-9]{2}\.[0-9]{2})\Z")
RUNS_ON = re.compile(r"^(?P<indent>[ \t]*)runs-on:[ \t]*(?P<value>.*?)[ \t]*$")
DISABLED = "if: ${{ false }}"


def check_workflow(name: str, source: str) -> list[str]:
    """Return actionable policy violations for a single workflow."""
    lines = source.splitlines()
    runners: list[tuple[int, int, str]] = []
    for number, line in enumerate(lines, start=1):
        match = RUNS_ON.match(line)
        if match:
            value = match.group("value").split(" #", 1)[0].strip()
            runners.append((number, len(match.group("indent")), value))

    errors: list[str] = []
    if name != STAGE_WORKFLOW:
        for number, _, value in runners:
            if not HOSTED_UBUNTU.fullmatch(value):
                errors.append(
                    f"{name}:{number}: routine job must use a literal "
                    f"GitHub-hosted Ubuntu runner, not {value!r}"
                )
        return errors

    if len(runners) != 1:
        errors.append(f"{name}: expected exactly one disabled hardware-stage job")
    if re.search(r"^  (?:push|pull_request|schedule|workflow_run):", source, re.MULTILINE):
        errors.append(f"{name}: hardware stage must remain manual-only")

    for number, indent, _ in runners:
        disabled = False
        for preceding in reversed(lines[: number - 1]):
            stripped = preceding.strip()
            level = len(preceding) - len(preceding.lstrip(" \t"))
            if stripped and level < indent:
                break
            if level == indent and stripped.startswith("if:"):
                disabled = stripped == DISABLED
                break
        if not disabled:
            errors.append(
                f"{name}:{number}: self-hosted hardware stage requires "
                "a job-level 'if: ${{ false }}' before runs-on"
            )
    return errors


def main() -> int:
    directory = Path(__file__).resolve().parents[1] / ".github" / "workflows"
    workflows = sorted((*directory.glob("*.yml"), *directory.glob("*.yaml")))
    if not workflows:
        print(f"No workflow files found in {directory}", file=sys.stderr)
        return 1

    errors: list[str] = []
    for path in workflows:
        errors.extend(check_workflow(path.name, path.read_text(encoding="utf-8")))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Verified hosted Ubuntu runner policy across {len(workflows)} workflows.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
