#!/usr/bin/env python3
"""Move invite-code validation from Android into the Rust UniFFI facade."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TYPES = ROOT / "rust/silent-disco-ffi/src/host_control/types.rs"
HANDLE = ROOT / "rust/silent-disco-ffi/src/host_control/handle.rs"
TEST = ROOT / "rust/silent-disco-ffi/tests/host_admission.rs"


def replace_once(path: Path, label: str, old: str, new: str) -> None:
    content = path.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path} [{label}]: expected one match, found {count}")
    path.write_text(content.replace(old, new), encoding="utf-8")


def update_record() -> None:
    replace_once(
        TYPES,
        "raw-invite-field",
        "    pub invite_code_valid: bool,",
        "    pub invite_code: Option<String>,",
    )


def update_submission_policy() -> None:
    replace_once(
        HANDLE,
        "approval-mode-import",
        "    DeviceId, MonotonicMillis, OperationId, RequestId, SyncConfidence, TransportState, TrustState,",
        "    ApprovalMode, DeviceId, MonotonicMillis, OperationId, RequestId, SyncConfidence,\n    TransportState, TrustState,",
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
        "raw-invite-test-field",
        "                        invite_code_valid: true,",
        "                        invite_code: None,",
    )


def main() -> None:
    update_record()
    update_submission_policy()
    update_test()


if __name__ == "__main__":
    main()
