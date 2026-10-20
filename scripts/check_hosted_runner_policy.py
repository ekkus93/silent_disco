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

HARDWARE_WORKFLOWS = frozenset({
    "block46-self-hosted-stage.yml",
    "self-hosted-android-device-probe.yml",
})
HOSTED_UBUNTU = re.compile(r"ubuntu-(?:latest|[0-9]{2}\.[0-9]{2})\Z")
RUNS_ON = re.compile(r"^(?P<indent>[ \t]*)runs-on:[ \t]*(?P<value>.*?)[ \t]*$")
DISABLED = "if: ${{ false }}"



def is_manual_only_hardware_workflow(source: str) -> bool:
    """Fail closed on inline, non-manual, missing, or ambiguous event syntax.

    Hardware staging is deliberately restricted to the canonical multiline
    'on:' block with exactly one 'workflow_dispatch:' event. Nested inputs
    under that event are permitted; extra top-level events are never allowed.
    """
    in_events = False
    saw_on = False
    event_count = 0
    for line in source.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" \\t"))
        if indent == 0:
            in_events = False
            if re.match(r"^on[ \\t]*:", line):
                if saw_on or not re.fullmatch(r"on[ \\t]*:[ \\t]*(?:#.*)?", line):
                    return False
                saw_on = True
                in_events = True
            continue
        if in_events and indent == 2:
            if not re.fullmatch(r"workflow_dispatch:[ \\t]*(?:#.*)?", stripped):
                return False
            event_count += 1
    return saw_on and event_count == 1

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
    if name not in HARDWARE_WORKFLOWS:
        for number, _, value in runners:
            if not HOSTED_UBUNTU.fullmatch(value):
                errors.append(
                    f"{name}:{number}: routine job must use a literal "
                    f"GitHub-hosted Ubuntu runner, not {value!r}"
                )
        return errors

    if len(runners) != 1:
        errors.append(f"{name}: expected exactly one disabled hardware-stage job")
    if not is_manual_only_hardware_workflow(source):
        errors.append(
            f"{name}: hardware stage must declare exactly one top-level "
            "'on: workflow_dispatch' event block (no inline or automatic triggers)"
        )

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
