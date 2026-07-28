use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};
use serde::{Deserialize, Serialize};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::CoreSnapshot;
use ts_rs::TS;

/// Validated transport shape for opening a profile.
///
/// The backend parses the bounded `ProfileId`; the frontend never supplies paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OpenProfileRequest {
    pub profile_id: String,
}

/// Bounded authoritative snapshot summary used during the initial desktop bridge.
///
/// Additional domain collections are added as their UI surfaces are implemented;
/// lifecycle values and the revision always come directly from `CoreSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CoreSnapshotDto {
    pub revision: String,
    pub selected_role: Option<String>,
    pub host_lifecycle: String,
    pub listener_lifecycle: String,
    pub transport_state: String,
    pub discovery_active: bool,
    pub discovered_session_count: u32,
    pub pending_join_request_count: u32,
    pub listener_count: u32,
    pub playback_state: String,
    pub playback_position_ms: String,
    pub recoverable_action: Option<String>,
    pub last_error: Option<DesktopErrorDto>,
    pub shutting_down: bool,
}

impl From<CoreSnapshot> for CoreSnapshotDto {
    fn from(value: CoreSnapshot) -> Self {
        Self {
            revision: value.revision.get().to_string(),
            selected_role: value.selected_role.map(|role| role.wire_name().to_owned()),
            host_lifecycle: value.host_lifecycle.wire_name().to_owned(),
            listener_lifecycle: value.listener_lifecycle.wire_name().to_owned(),
            transport_state: value.transport_state.wire_name().to_owned(),
            discovery_active: value.discovery_active,
            discovered_session_count: saturating_u32(value.discovered_sessions.len()),
            pending_join_request_count: saturating_u32(value.pending_join_requests.len()),
            listener_count: saturating_u32(value.listeners.len()),
            playback_state: value.playback_state.wire_name().to_owned(),
            playback_position_ms: value.playback_position_ms.to_string(),
            recoverable_action: value.recoverable_action.map(recoverable_action_name),
            last_error: value.last_error.map(DesktopErrorDto::from),
            shutting_down: value.shutting_down,
        }
    }
}

/// Result returned only after profile lock, secure identity, storage, actor, and
/// the bounded notification observer are all ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct OpenProfileResponse {
    pub lifecycle: BridgeLifecycleDto,
    pub core_version: CoreVersionDto,
    pub snapshot: CoreSnapshotDto,
}

impl From<CoreError> for DesktopErrorDto {
    fn from(value: CoreError) -> Self {
        Self::new(
            &format!("core.{}", value.code.stable_name()),
            value.subsystem.stable_name(),
            value.severity.stable_name(),
            value.retryable,
            &value.message,
        )
    }
}

fn saturating_u32(value: usize) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(error) => panic!("validated core collection count exceeded u32: {error}"),
    }
}

fn recoverable_action_name(action: silent_disco_core::runtime::RecoverableAction) -> String {
    match action {
        silent_disco_core::runtime::RecoverableAction::Retry => "retry",
        silent_disco_core::runtime::RecoverableAction::Rescan => "rescan",
        silent_disco_core::runtime::RecoverableAction::Reconnect => "reconnect",
        silent_disco_core::runtime::RecoverableAction::Resynchronize => "resynchronize",
        silent_disco_core::runtime::RecoverableAction::ReselectAudioSource => {
            "reselect_audio_source"
        }
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::CoreSnapshotDto;
    use silent_disco_core::domain::{AppRole, HostLifecycle, TransportState};
    use silent_disco_core::runtime::{CoreSnapshot, SnapshotRevision};

    #[test]
    fn snapshot_conversion_preserves_authoritative_revision_and_lifecycle() {
        let snapshot = CoreSnapshot {
            revision: SnapshotRevision::new(u64::MAX),
            selected_role: Some(AppRole::Host),
            host_lifecycle: HostLifecycle::WaitingForListeners,
            transport_state: TransportState::Advertising,
            ..CoreSnapshot::default()
        };

        let dto = CoreSnapshotDto::from(snapshot);
        assert_eq!(dto.revision, u64::MAX.to_string());
        assert_eq!(dto.selected_role.as_deref(), Some("host"));
        assert_eq!(dto.host_lifecycle, "waiting_for_listeners");
        assert_eq!(dto.transport_state, "advertising");
    }
}
