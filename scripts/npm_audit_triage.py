#!/usr/bin/env python3
"""Apply the supply-chain triage policy to `npm audit --json` output.

Every advisory fails unless it is listed in the exceptions file with a reason
and an unexpired `until` date. An exception that matches nothing also fails, so
a fixed advisory's entry has to be removed. An audit that could not run (for
example, the registry was unreachable) fails rather than passing as clean.

Usage: npm audit --json | npm_audit_triage.py EXCEPTIONS_JSON [TODAY]
TODAY (YYYY-MM-DD) defaults to the current UTC date; tests pass it explicitly.
"""

from __future__ import annotations

import datetime
import json
import sys


def advisories(report: dict) -> dict[str, dict]:
    """Advisory id -> {title, severity, packages} for every reported advisory."""
    found: dict[str, dict] = {}
    for package, entry in report.get("vulnerabilities", {}).items():
        for via in entry.get("via", []):
            if not isinstance(via, dict):
                continue  # a package name: the advisory is reported on that package
            url = via.get("url", "")
            advisory_id = url.rsplit("/", 1)[-1] if url else str(via.get("source"))
            item = found.setdefault(
                advisory_id,
                {"title": via.get("title", ""), "severity": via.get("severity", ""), "packages": set()},
            )
            item["packages"].add(package)
    return found


def triage(report: dict, exceptions: list[dict], today: datetime.date) -> list[str]:
    """Problems that fail the gate; empty when every finding is excepted."""
    if "error" in report:
        return [f"npm audit did not run: {json.dumps(report['error'])}"]
    if "vulnerabilities" not in report:
        return ["npm audit output has no 'vulnerabilities' section"]

    problems: list[str] = []
    allowed: dict[str, dict] = {}
    for exception in exceptions:
        advisory_id = exception.get("advisory", "")
        reason = exception.get("reason", "").strip()
        until = exception.get("until", "")
        if not advisory_id or not reason or not until:
            problems.append(f"exception {exception!r} needs advisory, reason and until")
            continue
        if datetime.date.fromisoformat(until) < today:
            problems.append(f"exception for {advisory_id} expired on {until}")
            continue
        allowed[advisory_id] = exception

    found = advisories(report)
    for advisory_id, item in sorted(found.items()):
        packages = ", ".join(sorted(item["packages"]))
        if advisory_id in allowed:
            print(f"excepted {advisory_id} ({item['severity']}) in {packages}: {allowed[advisory_id]['reason']}")
        else:
            problems.append(f"{advisory_id} ({item['severity']}) in {packages}: {item['title']}")
    for advisory_id in sorted(set(allowed) - set(found)):
        problems.append(f"exception for {advisory_id} matches no finding; remove it")
    return problems


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as handle:
        exceptions = json.load(handle)
    today = (
        datetime.date.fromisoformat(argv[2])
        if len(argv) == 3
        else datetime.datetime.now(datetime.timezone.utc).date()
    )
    try:
        report = json.load(sys.stdin)
    except json.JSONDecodeError as error:
        print(f"npm audit output is not JSON: {error}", file=sys.stderr)
        return 1
    problems = triage(report, exceptions, today)
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    if not problems:
        print("npm audit: no unexcepted advisories")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
