#!/usr/bin/env python3
"""Move per-request approval lifetime into the Rust command contract."""

from pathlib import Path
from textwrap import dedent

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


def update_command_records() -> None:
    for path in (
        "rust/silent-disco-core/src/runtime/records.rs",
        "rust/silent-disco-core/src/runtime/records_v2.rs",
    ):
        replace_once(
            path,
            "approve-command-payload",
            "    ApproveJoin { request_id: RequestId },\n",
            dedent(
                """
                    ApproveJoin {
                        request_id: RequestId,
                        remember_for_future: bool,
                    },
                """
            ),
        )


def update_actor_dispatch() -> None:
    replace_once(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/commands.rs",
        "approve-dispatch",
        dedent(
            """
                        CoreCommand::ApproveJoin { request_id } => {
                            self.approve_join(operation_id, request_id)
                        }
            """
        ),
        dedent(
            """
                        CoreCommand::ApproveJoin {
                            request_id,
                            remember_for_future,
                        } => self.approve_join(operation_id, request_id, remember_for_future),
            """
        ),
    )


def update_admission_policy() -> None:
    path = "rust/silent-disco-core/src/runtime/actor_runtime/state/admission.rs"
    replace_once(
        path,
        "approve-signature",
        dedent(
            """
                    pub(super) fn approve_join(
                        &mut self,
                        operation_id: OperationId,
                        request_id: RequestId,
                    ) -> Result<ApplyOutcome, CoreError> {
            """
        ),
        dedent(
            """
                    pub(super) fn approve_join(
                        &mut self,
                        operation_id: OperationId,
                        request_id: RequestId,
                        remember_for_future: bool,
                    ) -> Result<ApplyOutcome, CoreError> {
            """
        ),
    )
    replace_once(
        path,
        "explicit-approval-preparation",
        "        match prepare_approval(&self.snapshot.host_draft, &request) {\n",
        "        match prepare_approval(&request, remember_for_future) {\n",
    )

    policy = "rust/silent-disco-core/src/runtime/host_admission.rs"
    replace_once(
        policy,
        "approval-doc",
        dedent(
            '''
            /// Plans an explicit approval without mutating the pending request.
            ///
            /// Durable trust is written before approval delivery whenever the host requested
            /// "remember approved devices" and the listener is not already trusted.
            #[must_use]
            pub fn prepare_approval(draft: &HostDraft, request: &JoinRequestSummary) -> ApprovalPreparation {
                if draft.remember_approved_devices && request.trust_state != TrustState::Trusted {
            '''
        ),
        dedent(
            '''
            /// Plans an explicit approval without mutating the pending request.
            ///
            /// Durable trust is written before approval delivery only when this specific approval
            /// requests future trust and the listener is not already trusted.
            #[must_use]
            pub fn prepare_approval(
                request: &JoinRequestSummary,
                remember_for_future: bool,
            ) -> ApprovalPreparation {
                if remember_for_future && request.trust_state != TrustState::Trusted {
            '''
        ),
    )
    replace_once(
        policy,
        "policy-unit-tests",
        dedent(
            """
                #[test]
                fn remember_policy_requires_persistence_before_delivery() {
                    let draft = HostDraft {
                        remember_approved_devices: true,
                        ..HostDraft::default()
                    };
                    let request = request(TrustState::SessionOnly, true);

                    let ApprovalPreparation::PersistTrustFirst(persistence) =
                        prepare_approval(&draft, &request)
                    else {
                        panic!("approval must persist trust before delivery");
                    };
                    assert_eq!(persistence.request_id, request.request_id);
                    assert_eq!(persistence.device_id, request.device_id);

                    let delivery = approval_after_persistence(&persistence, TrustPersistenceOutcome::Committed);
                    assert!(delivery.trusted_for_future);
                    assert!(!delivery.persistence_failed);
                }

                #[test]
                fn failed_persistence_falls_back_visibly_to_session_only_delivery() {
                    let draft = HostDraft {
                        remember_approved_devices: true,
                        ..HostDraft::default()
                    };
                    let request = request(TrustState::SessionOnly, true);
                    let ApprovalPreparation::PersistTrustFirst(persistence) =
                        prepare_approval(&draft, &request)
                    else {
                        panic!("approval must persist trust before delivery");
                    };

                    let delivery = approval_after_persistence(&persistence, TrustPersistenceOutcome::Failed);
                    assert!(!delivery.trusted_for_future);
                    assert!(delivery.persistence_failed);
                }

                #[test]
                fn session_only_approval_does_not_request_persistence() {
                    let draft = HostDraft::default();
                    let request = request(TrustState::SessionOnly, true);

                    let ApprovalPreparation::Deliver(delivery) = prepare_approval(&draft, &request) else {
                        panic!("session-only approval must deliver directly");
                    };
                    assert!(!delivery.trusted_for_future);
                    assert!(!delivery.persistence_failed);
                }
            """
        ),
        dedent(
            """
                #[test]
                fn explicit_future_trust_requires_persistence_before_delivery() {
                    let request = request(TrustState::SessionOnly, true);

                    let ApprovalPreparation::PersistTrustFirst(persistence) =
                        prepare_approval(&request, true)
                    else {
                        panic!("approval must persist trust before delivery");
                    };
                    assert_eq!(persistence.request_id, request.request_id);
                    assert_eq!(persistence.device_id, request.device_id);

                    let delivery = approval_after_persistence(&persistence, TrustPersistenceOutcome::Committed);
                    assert!(delivery.trusted_for_future);
                    assert!(!delivery.persistence_failed);
                }

                #[test]
                fn failed_persistence_falls_back_visibly_to_session_only_delivery() {
                    let request = request(TrustState::SessionOnly, true);
                    let ApprovalPreparation::PersistTrustFirst(persistence) =
                        prepare_approval(&request, true)
                    else {
                        panic!("approval must persist trust before delivery");
                    };

                    let delivery = approval_after_persistence(&persistence, TrustPersistenceOutcome::Failed);
                    assert!(!delivery.trusted_for_future);
                    assert!(delivery.persistence_failed);
                }

                #[test]
                fn approve_once_does_not_request_persistence() {
                    let request = request(TrustState::SessionOnly, true);

                    let ApprovalPreparation::Deliver(delivery) = prepare_approval(&request, false) else {
                        panic!("approve-once must deliver directly");
                    };
                    assert!(!delivery.trusted_for_future);
                    assert!(!delivery.persistence_failed);
                }
            """
        ),
    )


def update_ffi() -> None:
    path = "rust/silent-disco-ffi/src/host_control/handle.rs"
    replace_once(
        path,
        "ffi-approve-signature",
        dedent(
            """
                pub fn approve_join(
                    &self,
                    expected_revision: u64,
                    request_id: String,
                ) -> Result<FfiCommandReceipt, FfiBridgeError> {
            """
        ),
        dedent(
            """
                pub fn approve_join(
                    &self,
                    expected_revision: u64,
                    request_id: String,
                    remember_for_future: bool,
                ) -> Result<FfiCommandReceipt, FfiBridgeError> {
            """
        ),
    )
    replace_once(
        path,
        "ffi-approve-command",
        dedent(
            """
                        CoreCommand::ApproveJoin {
                            request_id: request_id_from_string(request_id)?,
                        },
            """
        ),
        dedent(
            """
                        CoreCommand::ApproveJoin {
                            request_id: request_id_from_string(request_id)?,
                            remember_for_future,
                        },
            """
        ),
    )


def update_tests() -> None:
    actor = "rust/silent-disco-core/tests/host_block12_actor_admission.rs"
    content = read(actor)
    occurrences = content.count("CoreCommand::ApproveJoin {")
    if occurrences != 3:
        raise SystemExit(f"{actor}: expected 3 approval commands, found {occurrences}")
    first = dedent(
        """
                CoreCommand::ApproveJoin {
                    request_id: request.request_id.clone(),
                },
        """
    )
    if content.count(first) != 2:
        raise SystemExit(f"{actor}: expected two approve-once fixtures")
    content = content.replace(
        first,
        dedent(
            """
                    CoreCommand::ApproveJoin {
                        request_id: request.request_id.clone(),
                        remember_for_future: false,
                    },
            """
        ),
    )
    remembered = dedent(
        """
                CoreCommand::ApproveJoin {
                    request_id: request.request_id,
                },
        """
    )
    if content.count(remembered) != 1:
        raise SystemExit(f"{actor}: expected one durable approval fixture")
    content = content.replace(
        remembered,
        dedent(
            """
                    CoreCommand::ApproveJoin {
                        request_id: request.request_id,
                        remember_for_future: true,
                    },
            """
        ),
    )
    write(actor, content)

    replace_once(
        "rust/silent-disco-ffi/tests/host_admission.rs",
        "ffi-approve-call",
        ".approve_join(pending.revision, \"request-ffi-1\".to_owned())\n",
        ".approve_join(pending.revision, \"request-ffi-1\".to_owned(), false)\n",
    )


def update_automation_source() -> None:
    path = "scripts/apply-block13-rust-admission.py"
    content = read(path)
    content = content.replace(
        "CoreCommand::ApproveJoin { request_id } => {\n                        self.approve_join(operation_id, request_id)\n                    }",
        "CoreCommand::ApproveJoin {\n                        request_id,\n                        remember_for_future,\n                    } => self.approve_join(operation_id, request_id, remember_for_future),",
    )
    content = content.replace(
        ".approve_join(pending.revision, \"request-ffi-1\".to_owned())",
        ".approve_join(pending.revision, \"request-ffi-1\".to_owned(), false)",
    )
    write(path, content)


def main() -> None:
    update_command_records()
    update_actor_dispatch()
    update_admission_policy()
    update_ffi()
    update_tests()
    update_automation_source()


if __name__ == "__main__":
    main()
