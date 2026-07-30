#!/usr/bin/env python3
"""Complete the explicit approval lifetime export and actor routing edges."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, label: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path} [{label}]: expected one match, found {count}")
    target.write_text(content.replace(old, new), encoding="utf-8")


def main() -> None:
    replace_once(
        "rust/silent-disco-core/src/runtime/mod.rs",
        "explicit-approval-export",
        "    approval_after_persistence, classify_delivery, classify_join_request, prepare_approval,\n",
        (
            "    approval_after_persistence, classify_delivery, classify_join_request, prepare_approval,\n"
            "    prepare_explicit_approval,\n"
        ),
    )
    replace_once(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/mod.rs",
        "top-level-approval-routing",
        (
            "                    CoreCommand::ApproveJoin { request_id } => {\n"
            "                        self.approve_join(operation_id, request_id)\n"
            "                    }\n"
        ),
        (
            "                    CoreCommand::ApproveJoin {\n"
            "                        request_id,\n"
            "                        remember_for_future,\n"
            "                    } => self.approve_join(operation_id, request_id, remember_for_future),\n"
        ),
    )


if __name__ == "__main__":
    main()
