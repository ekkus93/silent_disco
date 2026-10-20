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
        indent = len(line) - len(line.lstrip(" \t"))
        if indent == 0:
            in_events = False
            if re.match(r"^on[ \t]*:", line):
                if saw_on or not re.fullmatch(r"on[ \t]*:[ \t]*(?:#.*)?", line):
                    return False
                saw_on = True
                in_events = True
            continue
        if in_events and indent == 2:
            if not re.fullmatch(r"workflow_dispatch:[ \t]*(?:#.*)?", stripped):
                return False
            event_count += 1
    return saw_on and event_count == 1



def check_workflow(name: str, source: str) -> list[str]:
    """Reject unsafe job declarations, including reusable-workflow bypasses.

    Only literal, top-level jobs with exactly one runner are accepted.
    Job-level reusable workflows are intentionally disallowed because their
    runner selection cannot be verified by this file-level policy checker.
    """
    errors: list[str] = []
    if name in HARDWARE_WORKFLOWS and not is_manual_only_hardware_workflow(source):
        errors.append(
            f"{name}: hardware stage must declare exactly one top-level "
            "'on: workflow_dispatch' event block (no inline or automatic triggers)"
        )

    in_jobs = False
    saw_jobs = False
    job_count = 0
    current_job: str | None = None
    runner_values: list[str] = []
    disabled = False
    reusable = False

    def finish_job() -> None:
        if current_job is None:
            return
        if len(runner_values) != 1:
            errors.append(
                f"{name}: job {current_job!r} requires exactly one literal runs-on"
            )
        elif name not in HARDWARE_WORKFLOWS and not HOSTED_UBUNTU.fullmatch(runner_values[0]):
            errors.append(
                f"{name}: job {current_job!r} must use a literal "
                f"GitHub-hosted Ubuntu runner, not {runner_values[0]!r}"
            )
        if reusable:
            errors.append(
                f"{name}: job {current_job!r} must not delegate runner "
                "selection through a job-level reusable workflow"
            )
        if name in HARDWARE_WORKFLOWS and not disabled:
            errors.append(
                f"{name}: job {current_job!r} requires "
                "a job-level 'if: \${{ false }}'"
            )

    for number, line in enumerate(source.splitlines(), start=1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" \t"))
        if indent == 0:
            if in_jobs:
                finish_job()
            in_jobs = False
            if re.match(r"^jobs\s*:", line):
                if saw_jobs or not re.fullmatch(r"jobs:[ \t]*(?:#.*)?", line):
                    errors.append(f"{name}:{number}: invalid or duplicate jobs mapping")
                else:
                    saw_jobs = True
                    in_jobs = True
            continue
        if not in_jobs:
            continue
        if indent == 2:
            finish_job()
            current_job = None
            runner_values = []
            disabled = False
            reusable = False
            if not re.fullmatch(r"[A-Za-z0-9_-]+:[ \t]*(?:#.*)?", stripped):
                errors.append(f"{name}:{number}: nonliteral job declaration")
                continue
            current_job = stripped.split(":", 1)[0]
            job_count += 1
        elif indent == 4 and current_job is not None:
            if stripped.startswith("runs-on:"):
                runner_values.append(stripped.split(":", 1)[1].split(" #", 1)[0].strip())
            elif stripped.startswith("uses:"):
                reusable = True
            elif stripped.startswith("if:"):
                disabled = stripped == DISABLED
    if in_jobs:
        finish_job()
    if not saw_jobs or job_count == 0:
        errors.append(f"{name}: expected a nonempty literal jobs mapping")
    if name in HARDWARE_WORKFLOWS and job_count != 1:
        errors.append(f"{name}: expected exactly one disabled hardware-stage job")
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
