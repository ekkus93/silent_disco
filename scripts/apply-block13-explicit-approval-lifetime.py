#!/usr/bin/env python3
"""Move per-request approval lifetime into the Rust command contract."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8")


def replace_once(path: str, label: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path} [{label}]: expected one match, found {count}")
    write(path, content.replace(old, new))


def replace_count(path: str, label: str, old: str, new: str, expected: int) -> None:
    content = read(path)
    count = content.count(old)
    if count != expected:
        raise SystemExit(f"{path} [{label}]: expected {expected} matches, found {count}")
    write(path, content.replace(old, new))


def update_command_records() -> None:
    old = "    ApproveJoin { request_id: RequestId },\n"
    new = (
        "    ApproveJoin {\n"
        "        request_id: RequestId,\n"
        "        remember_for_future: bool,\n"
        "    },\n"
    )
    for path in (
        "rust/silent-disco-core/src/runtime/records.rs",
        "rust/silent-disco-core/src/runtime/records_v2.rs",
    ):
        replace_once(path, "approve-command-payload", old, new)


def update_actor_dispatch() -> None:
    path = "rust/silent-disco-core/src/runtime/actor_runtime/state/commands.rs"
    compact = (
        "            CoreCommand::ApproveJoin { request_id } => "
        "self.approve_join(operation_id, request_id),\n"
    )
    expanded = (
        "            CoreCommand::ApproveJoin { request_id } => {\n"
        "                self.approve_join(operation_id, request_id)\n"
        "            }\n"
    )
    content = read(path)
    if compact in content:
        old = compact
    elif expanded in content:
        old = expanded
    else:
        raise SystemExit(f"{path} [approve-dispatch]: no supported fixture matched")
    new = (
        "            CoreCommand::ApproveJoin {\n"
        "                request_id,\n"
        "                remember_for_future,\n"
        "            } => self.approve_join(operation_id, request_id, remember_for_future),\n"
    )
    write(path, content.replace(old, new, 1))


def update_admission_policy() -> None:
    path = "rust/silent-disco-core/src/runtime/actor_runtime/state/admission.rs"
    replace_once(
        path,
        "approval-import",
        "    classify_delivery, classify_join_request, prepare_approval,\n",
        "    classify_delivery, classify_join_request, prepare_explicit_approval,\n",
    )
    replace_once(
        path,
        "approve-signature",
        (
            "    pub(super) fn approve_join(\n"
            "        &mut self,\n"
            "        operation_id: OperationId,\n"
            "        request_id: RequestId,\n"
            "    ) -> Result<ApplyOutcome, CoreError> {\n"
        ),
        (
            "    pub(super) fn approve_join(\n"
            "        &mut self,\n"
            "        operation_id: OperationId,\n"
            "        request_id: RequestId,\n"
            "        remember_for_future: bool,\n"
            "    ) -> Result<ApplyOutcome, CoreError> {\n"
        ),
    )
    replace_once(
        path,
        "explicit-approval-preparation",
        "        match prepare_approval(&self.snapshot.host_draft, &request) {\n",
        "        match prepare_explicit_approval(&request, remember_for_future) {\n",
    )

    policy = "rust/silent-disco-core/src/runtime/host_admission.rs"
    old = (
        "/// Plans an explicit approval without mutating the pending request.\n"
        "///\n"
        "/// Durable trust is written before approval delivery whenever the host requested\n"
        "/// \"remember approved devices\" and the listener is not already trusted.\n"
        "#[must_use]\n"
        "pub fn prepare_approval(draft: &HostDraft, request: &JoinRequestSummary) -> ApprovalPreparation {\n"
        "    if draft.remember_approved_devices && request.trust_state != TrustState::Trusted {\n"
        "        ApprovalPreparation::PersistTrustFirst(TrustPersistenceRequest {\n"
        "            request_id: request.request_id.clone(),\n"
        "            device_id: request.device_id.clone(),\n"
        "            display_name: request.display_name.clone(),\n"
        "        })\n"
        "    } else {\n"
        "        ApprovalPreparation::Deliver(ApprovalDelivery {\n"
        "            request_id: request.request_id.clone(),\n"
        "            device_id: request.device_id.clone(),\n"
        "            trusted_for_future: request.trust_state == TrustState::Trusted,\n"
        "            persistence_failed: false,\n"
        "        })\n"
        "    }\n"
        "}\n"
    )
    new = (
        "/// Plans an approval using the host draft's default lifetime policy.\n"
        "#[must_use]\n"
        "pub fn prepare_approval(draft: &HostDraft, request: &JoinRequestSummary) -> ApprovalPreparation {\n"
        "    prepare_explicit_approval(request, draft.remember_approved_devices)\n"
        "}\n\n"
        "/// Plans one explicit approval without mutating the pending request.\n"
        "///\n"
        "/// Durable trust is written before approval delivery only when this specific command\n"
        "/// requests future trust and the listener is not already trusted.\n"
        "#[must_use]\n"
        "pub fn prepare_explicit_approval(\n"
        "    request: &JoinRequestSummary,\n"
        "    remember_for_future: bool,\n"
        ") -> ApprovalPreparation {\n"
        "    if remember_for_future && request.trust_state != TrustState::Trusted {\n"
        "        ApprovalPreparation::PersistTrustFirst(TrustPersistenceRequest {\n"
        "            request_id: request.request_id.clone(),\n"
        "            device_id: request.device_id.clone(),\n"
        "            display_name: request.display_name.clone(),\n"
        "        })\n"
        "    } else {\n"
        "        ApprovalPreparation::Deliver(ApprovalDelivery {\n"
        "            request_id: request.request_id.clone(),\n"
        "            device_id: request.device_id.clone(),\n"
        "            trusted_for_future: request.trust_state == TrustState::Trusted,\n"
        "            persistence_failed: false,\n"
        "        })\n"
        "    }\n"
        "}\n"
    )
    replace_once(policy, "explicit-approval-policy", old, new)


def update_ffi() -> None:
    path = "rust/silent-disco-ffi/src/host_control/handle.rs"
    replace_once(
        path,
        "ffi-approve-signature",
        (
            "    pub fn approve_join(\n"
            "        &self,\n"
            "        expected_revision: u64,\n"
            "        request_id: String,\n"
            "    ) -> Result<FfiCommandReceipt, FfiBridgeError> {\n"
        ),
        (
            "    pub fn approve_join(\n"
            "        &self,\n"
            "        expected_revision: u64,\n"
            "        request_id: String,\n"
            "        remember_for_future: bool,\n"
            "    ) -> Result<FfiCommandReceipt, FfiBridgeError> {\n"
        ),
    )
    replace_once(
        path,
        "ffi-approve-command",
        (
            "            CoreCommand::ApproveJoin {\n"
            "                request_id: request_id_from_string(request_id)?,\n"
            "            },\n"
        ),
        (
            "            CoreCommand::ApproveJoin {\n"
            "                request_id: request_id_from_string(request_id)?,\n"
            "                remember_for_future,\n"
            "            },\n"
        ),
    )


def update_tests() -> None:
    actor = "rust/silent-disco-core/tests/host_block12_actor_admission.rs"
    replace_count(
        actor,
        "approve-once-fixtures",
        (
            "        CoreCommand::ApproveJoin {\n"
            "            request_id: request.request_id.clone(),\n"
            "        },\n"
        ),
        (
            "        CoreCommand::ApproveJoin {\n"
            "            request_id: request.request_id.clone(),\n"
            "            remember_for_future: false,\n"
            "        },\n"
        ),
        2,
    )
    replace_once(
        actor,
        "durable-approval-fixture",
        (
            "        CoreCommand::ApproveJoin {\n"
            "            request_id: request.request_id,\n"
            "        },\n"
        ),
        (
            "        CoreCommand::ApproveJoin {\n"
            "            request_id: request.request_id,\n"
            "            remember_for_future: true,\n"
            "        },\n"
        ),
    )
    replace_once(
        "rust/silent-disco-ffi/tests/host_admission.rs",
        "ffi-approve-call",
        ".approve_join(pending.revision, \"request-ffi-1\".to_owned())\n",
        ".approve_join(pending.revision, \"request-ffi-1\".to_owned(), false)\n",
    )


def main() -> None:
    update_command_records()
    update_actor_dispatch()
    update_admission_policy()
    update_ffi()
    update_tests()


if __name__ == "__main__":
    main()
