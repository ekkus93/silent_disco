#!/usr/bin/env python3
"""Move invite-code validation from Android into the Rust UniFFI facade."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TYPES = ROOT / "rust/silent-disco-ffi/src/host_control/types.rs"
HANDLE = ROOT / "rust/silent-disco-ffi/src/host_control/handle.rs"
LIB = ROOT / "rust/silent-disco-ffi/src/lib.rs"
TEST = ROOT / "rust/silent-disco-ffi/tests/host_admission.rs"


def replace_once(path: Path, label: str, old: str, new: str) -> None:
    content = path.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path} [{label}]: expected one match, found {count}")
    path.write_text(content.replace(old, new), encoding="utf-8")


def add_input_record() -> None:
    marker = """#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiSynchronizationSummary {
"""
    input_record = """#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiJoinRequestInput {
    pub request_id: String,
    pub device_id: String,
    pub display_name: String,
    pub trust_state: FfiTrustState,
    pub invite_code: Option<String>,
    pub received_at_ms: u64,
}

"""
    replace_once(TYPES, "join-input-record", marker, input_record + marker)


def export_input_record() -> None:
    replace_once(
        LIB,
        "join-input-export",
        "FfiCoreSnapshot, FfiDeliveryReport, FfiHostDraft, FfiHostLifecycle, FfiJoinRequest,",
        "FfiCoreSnapshot, FfiDeliveryReport, FfiHostDraft, FfiHostLifecycle, FfiJoinRequest,\n    FfiJoinRequestInput,",
    )


def update_submission_policy() -> None:
    replace_once(
        HANDLE,
        "join-input-import",
        "    FfiDeliveryReport, FfiHostDraft, FfiJoinRequest, FfiListenerSummary, FfiPlatformCompletion,",
        "    FfiDeliveryReport, FfiHostDraft, FfiJoinRequestInput, FfiListenerSummary,\n    FfiPlatformCompletion,",
    )
    replace_once(
        HANDLE,
        "approval-mode-import",
        "    DeviceId, MonotonicMillis, OperationId, RequestId, SyncConfidence, TransportState, TrustState,",
        "    ApprovalMode, DeviceId, MonotonicMillis, OperationId, RequestId, SyncConfidence,\n    TransportState, TrustState,",
    )
    replace_once(
        HANDLE,
        "join-input-argument",
        "pub fn submit_join_request(&self, request: FfiJoinRequest)",
        "pub fn submit_join_request(&self, request: FfiJoinRequestInput)",
    )
    old = """        self.ensure_open()?;
        let request = JoinRequestSummary::new(
            request_id_from_string(request.request_id)?,
            device_id_from_string(request.device_id)?,
            request.display_name,
            TrustState::try_from(request.trust_state)?,
            request.invite_code_valid,
            MonotonicMillis::new(request.received_at_ms),
        )
"""
    new = """        self.ensure_open()?;
        let snapshot = self.handle.current_snapshot()?;
        let invite_code_valid = match snapshot.host_draft.approval_mode {
            ApprovalMode::InviteCode => {
                snapshot.host_draft.invite_code.as_deref() == request.invite_code.as_deref()
            }
            ApprovalMode::Manual | ApprovalMode::TrustedDevices => true,
        };
        let request = JoinRequestSummary::new(
            request_id_from_string(request.request_id)?,
            device_id_from_string(request.device_id)?,
            request.display_name,
            TrustState::try_from(request.trust_state)?,
            invite_code_valid,
            MonotonicMillis::new(request.received_at_ms),
        )
"""
    replace_once(HANDLE, "authoritative-invite-comparison", old, new)


def update_test() -> None:
    replace_once(
        TEST,
        "join-input-test-import",
        "FfiCoreNotification, FfiCoreObserver, FfiDeliveryReport, FfiHostDraft, FfiJoinRequest,",
        "FfiCoreNotification, FfiCoreObserver, FfiDeliveryReport, FfiHostDraft,\n    FfiJoinRequestInput,",
    )
    replace_once(
        TEST,
        "join-input-test-constructor",
        ".submit_join_request(FfiJoinRequest {",
        ".submit_join_request(FfiJoinRequestInput {",
    )
    replace_once(
        TEST,
        "raw-invite-test-field",
        "invite_code_valid: true,",
        "invite_code: None,",
    )


def main() -> None:
    add_input_record()
    export_input_record()
    update_submission_policy()
    update_test()


if __name__ == "__main__":
    main()
