#!/usr/bin/env python3
from pathlib import Path
import subprocess


def git_show(spec: str) -> str:
    return subprocess.check_output(["git", "show", spec], text=True)


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one replacement target, found {count}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


# Restore every temporarily repurposed workflow from a known-good revision.
restores = {
    Path(".github/workflows/desktop-block23-runner.yml"): (
        "HEAD^:.github/workflows/desktop-block23-runner.yml"
    ),
    Path(".github/workflows/desktop-block23-diagnostics.yml"): (
        "3ce4f2b4a36c4c4cf1aae8d1697988571dbc6c3a:.github/workflows/desktop-block23-diagnostics.yml"
    ),
    Path(".github/workflows/desktop-block23-frontend-diagnostics.yml"): (
        "3ce4f2b4a36c4c4cf1aae8d1697988571dbc6c3a:.github/workflows/desktop-block23-frontend-diagnostics.yml"
    ),
    Path(".github/workflows/desktop-block23-source-snapshot.yml"): (
        "ab7f11394ab32a86462ff8c28514f50b96503102:.github/workflows/desktop-block23-source-snapshot.yml"
    ),
}
for path, spec in restores.items():
    path.write_text(git_show(spec), encoding="utf-8")

storage_call = "python3 .github/block23-storage-runner-adapt.py\n"
clippy_call = storage_call + "python3 .github/block23-tauri-clippy-adapt.py\n"
for name in (
    ".github/block23-validation.sh",
    ".github/workflows/desktop-block23-diagnostics.yml",
    ".github/workflows/desktop-block23-frontend-diagnostics.yml",
    ".github/workflows/desktop-block23-source-snapshot.yml",
):
    replace_once(
        Path(name),
        storage_call,
        clippy_call,
        f"{name} adapter invocation",
    )

validation = Path(".github/block23-validation.sh")
replace_once(
    validation,
    "  .github/block23-storage-runner-adapt.py \\\n",
    "  .github/block23-storage-runner-adapt.py \\\n  .github/block23-tauri-clippy-adapt.py \\\n",
    "validation cleanup registration",
)
replace_once(
    validation,
    "    '.github/block23-storage-runner-adapt.py',\n",
    "    '.github/block23-storage-runner-adapt.py',\n    '.github/block23-tauri-clippy-adapt.py',\n",
    "validation allowed-path registration",
)

standalone = Path(".github/workflows/desktop-block23-tauri-clippy-diagnostic.yml")
if standalone.exists():
    standalone.unlink()

# The wiring helper is temporary and must not survive the atomic commit.
Path(__file__).unlink()
print("restored Block 23 workflows and wired the Tauri clippy adapter everywhere")
