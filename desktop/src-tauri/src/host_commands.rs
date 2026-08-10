use crate::app_state::DesktopAppState;
use crate::diagnostics_dto::DesktopDiagnosticsDto;
use crate::dto::DesktopErrorDto;
use crate::host_session_dto::{HostInvitationDto, HostSessionSnapshotDto};
use crate::platform::file_picker::{SelectedSourceRegistry, pick_and_inspect};
use crate::platform::network_dto::{NetworkInterfaceSnapshotDto, SetNetworkBindPreferenceRequest};
use crate::platform::source_staging::stage_audio_source;
use crate::platform::source_staging_control::{
    SourceStagingControl, TauriSourceStagingProgressSink,
};
use crate::runtime_dto::{
    ApproveJoinRequest, CommandReceiptDto, JoinRequestCommandRequest, ListenerCommandRequest,
    RevisionCommandRequest, UpdateHostDraftRequest,
};
use silent_disco_core::domain::{AppRole, ApprovalMode, DeviceId, RequestId};
use silent_disco_core::runtime::{
    AudioSourcePatch, CoreCommand, HostDraftPatch, InviteCodePatch, SnapshotRevision,
};
use tauri::{AppHandle, Manager, State};

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

/// Opens one backend-owned file dialog, stages one source, and registers its opaque descriptor.
///
/// Dialog cancellation returns `Ok(None)`. Staging copies into the active profile through an owned
/// temporary file, verifies content, and atomically publishes before the actor sees the descriptor.
///
/// # Errors
///
/// Returns a structured validation, dialog, filesystem, staging, registry, actor, cancellation, or
/// blocking-worker failure. Native paths never enter the command response or core command.
#[tauri::command]
pub async fn select_audio_source(
    app: AppHandle,
    request: RevisionCommandRequest,
) -> Result<Option<CommandReceiptDto>, DesktopErrorDto> {
    let revision = parse_snapshot_revision(&request.expected_revision)?;
    let picker_app = app.clone();
    let inspected = tauri::async_runtime::spawn_blocking(move || pick_and_inspect(picker_app))
        .await
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.audio_source.worker_failed",
                "audio_source",
                "error",
                true,
                &format!("audio source selection worker failed: {error}"),
            )
        })??;
    let Some(source) = inspected else {
        return Ok(None);
    };

    let operation = app.state::<SourceStagingControl>().begin()?;
    let sources_directory = app.state::<DesktopAppState>().source_staging_directory()?;
    let staging_app = app.clone();
    let staged = tauri::async_runtime::spawn_blocking(move || {
        let progress = TauriSourceStagingProgressSink::new(staging_app);
        stage_audio_source(&source, &sources_directory, &operation, &progress)
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.audio_source.staging_worker_failed",
            "audio_source",
            "error",
            true,
            &format!("audio source staging worker failed: {error}"),
        )
    })??;
    let source = staged.source;
    let descriptor = source.descriptor().clone();
    let source_id = descriptor.source_id.clone();
    let registry = app.state::<SelectedSourceRegistry>();
    let previous = registry.replace(source)?;
    let result = app.state::<DesktopAppState>().submit_core_command(
        revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: None,
        }),
    );
    match result {
        Ok(receipt) => Ok(Some(receipt)),
        Err(primary) => {
            if let Err(rollback) = registry.restore_if_current(&source_id, previous) {
                return Err(append_registration_rollback(&primary, &rollback));
            }
            Err(primary)
        }
    }
}

/// Requests cancellation of the active bounded source-staging operation.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn cancel_audio_source_staging(state: State<'_, SourceStagingControl>) -> bool {
    state.cancel()
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

/// Returns the authoritative active host workflow projection.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn get_host_session_state(
    state: State<'_, DesktopAppState>,
) -> Result<HostSessionSnapshotDto, DesktopErrorDto> {
    state.host_session_snapshot()
}

/// Requests delivery-first approval of one pending listener.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn approve_join_request(
    state: State<'_, DesktopAppState>,
    request: ApproveJoinRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::ApproveJoin {
            request_id: parse_request_id(&request.request_id)?,
            remember_for_future: request.remember_for_future,
        },
    )
}

/// Requests delivery-first rejection of one pending listener.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn reject_join_request(
    state: State<'_, DesktopAppState>,
    request: JoinRequestCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::RejectJoin {
            request_id: parse_request_id(&request.request_id)?,
        },
    )
}

/// Requests delivery-first removal of one connected listener.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn remove_listener(
    state: State<'_, DesktopAppState>,
    request: ListenerCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::RemoveListener {
            listener_id: parse_device_id(&request.listener_id)?,
        },
    )
}

/// Requests a revision-aware host-session shutdown.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn end_host_session(
    state: State<'_, DesktopAppState>,
    request: RevisionCommandRequest,
) -> Result<CommandReceiptDto, DesktopErrorDto> {
    state.submit_core_command(
        parse_snapshot_revision(&request.expected_revision)?,
        CoreCommand::EndHostSession,
    )
}

/// Starts real-time playback of the host draft's selected audio source:
/// opens the shared decoder, spawns the shared packetizer, and begins
/// broadcasting real audio/sync frames to connected listeners.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn start_host_playback(
    state: State<'_, DesktopAppState>,
    sources: State<'_, SelectedSourceRegistry>,
) -> Result<(), DesktopErrorDto> {
    state.start_host_playback(&sources)
}

/// Pauses the active playback stream after a validated actor transition.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn pause_host_playback(state: State<'_, DesktopAppState>) -> Result<(), DesktopErrorDto> {
    state.pause_host_playback()
}

/// Resumes the active, paused playback stream after a validated actor transition.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn resume_host_playback(state: State<'_, DesktopAppState>) -> Result<(), DesktopErrorDto> {
    state.resume_host_playback()
}

/// Stops the active playback stream and blocks until its pump thread exits.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn stop_host_playback(state: State<'_, DesktopAppState>) -> Result<(), DesktopErrorDto> {
    state.stop_host_playback()
}

/// Sets the desktop host's local-monitor preference (Block 34.2). Disabling
/// takes effect immediately; enabling takes effect on the next stream
/// start. The resulting status (active/failed) is surfaced through
/// `HostSessionSnapshotDto.monitor` on the next `get_host_session_state`
/// poll, not from this call's own result.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn set_host_monitor_enabled(
    state: State<'_, DesktopAppState>,
    enabled: bool,
) -> Result<(), DesktopErrorDto> {
    state.set_monitor_enabled(enabled)
}

/// Returns the bounded, redacted diagnostics snapshot (Block 35.1/35.2) --
/// safe to display or copy as-is.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn get_host_diagnostics(
    state: State<'_, DesktopAppState>,
) -> Result<DesktopDiagnosticsDto, DesktopErrorDto> {
    state.host_diagnostics()
}

/// Opens a native save dialog and writes the current diagnostics snapshot
/// there (Block 35.3). The snapshot is captured before the dialog opens, so
/// what gets written reflects the moment the export was requested, not
/// whatever the state happens to be when the user finishes picking a
/// destination. Dialog cancellation returns `Cancelled`, not an error --
/// see [`DiagnosticsExportOutcome`](crate::platform::diagnostics_export::DiagnosticsExportOutcome).
///
/// # Errors
///
/// Returns a structured error if the current diagnostics cannot be
/// gathered, the dialog worker fails, or the write/install fails. Never
/// reports success before the file is actually committed to disk.
#[tauri::command]
pub async fn export_host_diagnostics(
    app: AppHandle,
) -> Result<crate::platform::diagnostics_export::DiagnosticsExportOutcome, DesktopErrorDto> {
    let diagnostics = app.state::<DesktopAppState>().host_diagnostics()?;
    let now_ms = crate::platform::invitation::current_wall_clock_ms();
    tauri::async_runtime::spawn_blocking(move || {
        crate::platform::diagnostics_export::export_host_diagnostics(app, &diagnostics, now_ms)
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.diagnostics.export_worker_failed",
            "diagnostics",
            "error",
            true,
            &format!("diagnostics export worker failed: {error}"),
        )
    })?
}

/// Builds and signs a fresh QR invitation for the active host session
/// (Block 31.1). Always regenerates -- there is no cached invitation to
/// silently reuse, so the frontend's refresh action is genuinely explicit.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn create_host_invitation(
    state: State<'_, DesktopAppState>,
) -> Result<HostInvitationDto, DesktopErrorDto> {
    state.create_host_invitation()
}

/// Lists classified network addresses and the current bind policy for the ready profile.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn get_host_network_state(
    state: State<'_, DesktopAppState>,
) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
    state.host_network_snapshot()
}

/// Replaces the automatic or explicit host bind preference after current-snapshot validation.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn set_host_network_preference(
    state: State<'_, DesktopAppState>,
    request: SetNetworkBindPreferenceRequest,
) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
    state.set_host_network_preference(&request)
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

fn parse_request_id(value: &str) -> Result<RequestId, DesktopErrorDto> {
    RequestId::new(value.to_owned()).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.command.invalid_request_id",
            "validation",
            "error",
            false,
            &error.to_string(),
        )
    })
}

fn parse_device_id(value: &str) -> Result<DeviceId, DesktopErrorDto> {
    DeviceId::new(value.to_owned()).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.command.invalid_listener_id",
            "validation",
            "error",
            false,
            &error.to_string(),
        )
    })
}

fn append_registration_rollback(
    primary: &DesktopErrorDto,
    rollback: &DesktopErrorDto,
) -> DesktopErrorDto {
    DesktopErrorDto::new(
        &primary.code,
        &primary.subsystem,
        &primary.severity,
        primary.retryable,
        &format!(
            "{}; selected-source registration rollback also failed: {}",
            primary.message, rollback.message
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_device_id, parse_request_id, parse_snapshot_revision};

    #[test]
    fn revision_parser_accepts_only_canonical_decimal() {
        assert_eq!(parse_snapshot_revision("0").expect("zero").get(), 0);
        assert_eq!(parse_snapshot_revision("42").expect("revision").get(), 42);
        assert!(parse_snapshot_revision("").is_err());
        assert!(parse_snapshot_revision("01").is_err());
        assert!(parse_snapshot_revision("-1").is_err());
    }

    #[test]
    fn admission_identifiers_are_validated_before_actor_submission() {
        assert!(parse_request_id("request-1").is_ok());
        assert!(parse_request_id(" bad ").is_err());
        assert!(parse_device_id("listener-1").is_ok());
        assert!(parse_device_id("").is_err());
    }
}
