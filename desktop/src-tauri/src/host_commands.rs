use crate::app_state::DesktopAppState;
use crate::dto::DesktopErrorDto;
use crate::runtime_dto::{CommandReceiptDto, RevisionCommandRequest, UpdateHostDraftRequest};
use silent_disco_core::domain::{AppRole, ApprovalMode};
use silent_disco_core::runtime::{
    AudioSourcePatch, CoreCommand, HostDraftPatch, InviteCodePatch, SnapshotRevision,
};
use tauri::State;

/// Selects the host role through the revision-aware authoritative actor.
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction requires State by value.
#[tauri::command]
pub fn select_host_role(
    state: State<'_, DesktopAppState>,
    request: RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let RevisionCommandRequest { expected_revision } = request;
    state.submit_core_command(
        parse_snapshot_revision(&expected_revision)?,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    )
}

/// Applies one typed host-draft patch without allowing native paths through IPC.
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction requires State by value.
#[tauri::command]
pub fn update_host_draft(
    state: State<'_, DesktopAppState>,
    request: UpdateHostDraftRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let UpdateHostDraftRequest {
        expected_revision,
        session_name,
        approval_mode,
        invite_code,
        remember_approved_devices,
    } = request;
    let approval_mode = ApprovalMode::from_wire_name(&approval_mode).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.host.invalid_approval_mode",
            "validation",
            "error",
            false,
            &error.to_string(),
        )
    })?;
    let invite_code = match invite_code {
        Some(code) => InviteCodePatch::Set(code),
        None => InviteCodePatch::Clear,
    };
    state.submit_core_command(
        parse_snapshot_revision(&expected_revision)?,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some(session_name),
            approval_mode: Some(approval_mode),
            invite_code,
            audio_source: AudioSourcePatch::Unchanged,
            remember_approved_devices: Some(remember_approved_devices),
        }),
    )
}

/// Requests host-session creation. Queue admission is not reported as session success.
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction requires State by value.
#[tauri::command]
pub fn create_host_session(
    state: State<'_, DesktopAppState>,
    request: RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    let RevisionCommandRequest { expected_revision } = request;
    state.submit_core_command(
        parse_snapshot_revision(&expected_revision)?,
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
    value
        .parse::<u64>()
        .map(SnapshotRevision::new)
        .map_err(|error| {
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
