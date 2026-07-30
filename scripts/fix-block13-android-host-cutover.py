#!/usr/bin/env python3
"""Apply compile- and test-safe fixups after the Block 13 Android cutover."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}")
    target.write_text(content.replace(old, new), encoding="utf-8")


def normalize_eof(path: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    target.write_text(content.rstrip() + "\n", encoding="utf-8")


def remove_stale_test(path: str) -> None:
    target = ROOT / path
    if not target.exists():
        raise SystemExit(f"{path}: expected stale Kotlin authority test")
    target.unlink()


def main() -> None:
    replace_once(
        "app/src/main/java/com/ekkus/silentdisco/core/rust/HostCoreController.kt",
        "        handle.createHostSession(snapshot.revision)\n    }\n",
        "        handle.createHostSession(snapshot.revision)\n        Unit\n    }\n",
    )
    normalize_eof("app/src/main/java/com/ekkus/silentdisco/app/AppState.kt")
    normalize_eof("app/src/main/java/com/ekkus/silentdisco/app/MainViewModelHostActions.kt")
    remove_stale_test(
        "app/src/test/java/com/ekkus/silentdisco/app/HostStartupValidationTest.kt",
    )


if __name__ == "__main__":
    main()
