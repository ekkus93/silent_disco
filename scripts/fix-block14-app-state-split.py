from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    target = ROOT / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match in {path}, found {count}: {old[:120]!r}")
    write(path, content.replace(old, new, 1))


replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """use crate::runtime_dto::{
    AttachNotificationResponse, CommandReceiptDto, CoreNotificationDto, CoreSnapshotDto,
    OpenProfileRequest, OpenProfileResponse, RevisionCommandRequest, UpdateHostDraftRequest,
};
""",
    """use crate::runtime_dto::{
    AttachNotificationResponse, CommandReceiptDto, CoreNotificationDto, CoreSnapshotDto,
    OpenProfileRequest, OpenProfileResponse,
};
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """use silent_disco_core::domain::{AppRole, ApprovalMode};
use silent_disco_core::runtime::{
    AudioSourcePatch, CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreObserver, HostDraftPatch, InviteCodePatch, SnapshotRevision,
};
""",
    """use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand, CoreCommandRequest,
    CoreObserver, SnapshotRevision,
};
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    "    fn submit_core_command(\n",
    "    pub(crate) fn submit_core_command(\n",
)

app_state = read("desktop/src-tauri/src/app_state.rs")
command_start = app_state.index(
    "/// Selects the host role through the revision-aware authoritative actor.\n"
)
attach_marker = "/// Attaches or replaces the frontend notification channel for the ready profile.\n"
command_end = app_state.index(attach_marker, command_start)
write(
    "desktop/src-tauri/src/app_state.rs",
    app_state[:command_start] + attach_marker + app_state[command_end + len(attach_marker) :],
)

write(
    "desktop/src-tauri/src/host_commands.rs",
    '''use crate::app_state::DesktopAppState;
use crate::dto::DesktopErrorDto;
use crate::runtime_dto::{CommandReceiptDto, RevisionCommandRequest, UpdateHostDraftRequest};
use silent_disco_core::domain::{AppRole, ApprovalMode};
use silent_disco_core::runtime::{
    AudioSourcePatch, CoreCommand, HostDraftPatch, InviteCodePatch, SnapshotRevision,
};
use tauri::{AppHandle, Manager};

/// Selects the host role through the revision-aware authoritative actor.
#[tauri::command]
pub fn select_host_role(
    app: AppHandle,
    request: RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    app.state::<DesktopAppState>().submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::SelectRole { role: AppRole::Host },
    )
}

/// Applies one typed host-draft patch without allowing native paths through IPC.
#[tauri::command]
pub fn update_host_draft(
    app: AppHandle,
    request: UpdateHostDraftRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let approval_mode = ApprovalMode::from_wire_name(&request.approval_mode).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.host.invalid_approval_mode",
            "validation",
            "error",
            false,
            &error.to_string(),
        )
    })?;
    let invite_code = match request.invite_code {
        Some(code) => InviteCodePatch::Set(code),
        None => InviteCodePatch::Clear,
    };
    app.state::<DesktopAppState>().submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some(request.session_name),
            approval_mode: Some(approval_mode),
            invite_code,
            audio_source: AudioSourcePatch::Unchanged,
            remember_approved_devices: Some(request.remember_approved_devices),
        }),
    )
}

/// Requests host-session creation. Queue admission is not reported as session success.
#[tauri::command]
pub fn create_host_session(
    app: AppHandle,
    request: RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    app.state::<DesktopAppState>().submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::CreateHostSession,
    )
}

fn parse_snapshot_revision(value: &str) -> Result<SnapshotRevision, DesktopErrorDto> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(DesktopErrorDto::new(
            "desktop.command.invalid_revision",
            "validation",
            "error",
            false,
            "expected revision must be a canonical unsigned decimal string",
        ));
    }
    value.parse::<u64>().map(SnapshotRevision::new).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.command.invalid_revision",
            "validation",
            "error",
            false,
            &format!("expected revision is outside the supported range: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::parse_snapshot_revision;

    #[test]
    fn revision_parser_accepts_only_canonical_decimal() {
        assert_eq!(parse_snapshot_revision("0").expect("zero").get(), 0);
        assert_eq!(parse_snapshot_revision("42").expect("revision").get(), 42);
        assert!(parse_snapshot_revision("").is_err());
        assert!(parse_snapshot_revision("01").is_err());
        assert!(parse_snapshot_revision("-1").is_err());
    }
}
''',
)

replace_once(
    "desktop/src-tauri/src/lib.rs",
    "mod app_state;\n",
    "mod app_state;\nmod host_commands;\n",
)
replace_once(
    "desktop/src-tauri/src/lib.rs",
    """            app_state::select_host_role,
            app_state::update_host_draft,
            app_state::create_host_session,
""",
    """            host_commands::select_host_role,
            host_commands::update_host_draft,
            host_commands::create_host_session,
""",
)

(ROOT / "scripts/fix-block14-app-state-split.py").unlink()
