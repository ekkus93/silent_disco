use crate::dto::DesktopErrorDto;
use crate::platform::host_transport::ActiveHostSessionSnapshot;
use serde::{Deserialize, Serialize};
use silent_disco_core::domain::ApprovalMode;
use silent_disco_core::runtime::{CoreSnapshot, JoinRequestSummary, ListenerSummary};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostConnectionDto {
    pub host_address: String,
    pub control_port: u16,
    pub sync_port: u16,
    pub audio_port: u16,
    pub session_id: String,
    pub protocol_version: u16,
    pub invite_code_required: bool,
    pub expires_at_ms: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PendingJoinRequestDto {
    pub request_id: String,
    pub device_id: String,
    pub display_name: String,
    pub trust_state: String,
    pub invite_code_valid: bool,
    pub received_at_ms: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ConnectedListenerDto {
    pub device_id: String,
    pub display_name: String,
    pub trust_state: String,
    pub transport_state: String,
    pub last_contact_ms: String,
    pub estimated_offset_ms: Option<String>,
    pub round_trip_time_ms: Option<String>,
    pub last_error: Option<DesktopErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostSessionSnapshotDto {
    pub revision: String,
    pub host_lifecycle: String,
    pub transport_state: String,
    pub playback_state: String,
    pub session_name: String,
    pub connection: Option<HostConnectionDto>,
    pub pending_join_requests: Vec<PendingJoinRequestDto>,
    pub connected_listeners: Vec<ConnectedListenerDto>,
    pub playback_controls_enabled: bool,
    pub transport_worker_running: bool,
    pub transport_error: Option<String>,
    pub last_error: Option<DesktopErrorDto>,
}

impl HostSessionSnapshotDto {
    #[must_use]
    pub(crate) fn from_parts(
        snapshot: &CoreSnapshot,
        active: Option<&ActiveHostSessionSnapshot>,
    ) -> Self {
        let connection = active.map(|active| HostConnectionDto {
            host_address: active.endpoint.address.to_string(),
            control_port: active.endpoint.control_port,
            sync_port: active.endpoint.sync_port,
            audio_port: active.endpoint.audio_port,
            session_id: active.advertisement.session_id.as_str().to_owned(),
            protocol_version: active.advertisement.protocol_version,
            invite_code_required: active.advertisement.approval_mode == ApprovalMode::InviteCode,
            expires_at_ms: None,
        });
        Self {
            revision: snapshot.revision.get().to_string(),
            host_lifecycle: snapshot.host_lifecycle.wire_name().to_owned(),
            transport_state: snapshot.transport_state.wire_name().to_owned(),
            playback_state: snapshot.playback_state.wire_name().to_owned(),
            session_name: active.map_or_else(
                || snapshot.host_draft.session_name.clone(),
                |active| active.advertisement.session_name.clone(),
            ),
            connection,
            pending_join_requests: snapshot
                .pending_join_requests
                .iter()
                .map(PendingJoinRequestDto::from)
                .collect(),
            connected_listeners: snapshot
                .listeners
                .iter()
                .map(ConnectedListenerDto::from)
                .collect(),
            playback_controls_enabled: false,
            transport_worker_running: active.is_some_and(|active| active.worker_running),
            transport_error: active.and_then(|active| active.last_transport_error.clone()),
            last_error: snapshot.last_error.as_ref().map(core_error_dto),
        }
    }
}

impl From<&JoinRequestSummary> for PendingJoinRequestDto {
    fn from(value: &JoinRequestSummary) -> Self {
        Self {
            request_id: value.request_id.as_str().to_owned(),
            device_id: value.device_id.as_str().to_owned(),
            display_name: value.display_name.clone(),
            trust_state: value.trust_state.wire_name().to_owned(),
            invite_code_valid: value.invite_code_valid,
            received_at_ms: value.received_at.get().to_string(),
        }
    }
}

impl From<&ListenerSummary> for ConnectedListenerDto {
    fn from(value: &ListenerSummary) -> Self {
        Self {
            device_id: value.device_id.as_str().to_owned(),
            display_name: value.display_name.clone(),
            trust_state: value.trust_state.wire_name().to_owned(),
            transport_state: value.transport_state.wire_name().to_owned(),
            last_contact_ms: value.last_contact.get().to_string(),
            estimated_offset_ms: value
                .estimated_offset_ms
                .map(|offset| offset.get().to_string()),
            round_trip_time_ms: value
                .round_trip_time_ms
                .map(|duration| duration.get().to_string()),
            last_error: value.last_error.as_ref().map(core_error_dto),
        }
    }
}

fn core_error_dto(error: &silent_disco_core::error::CoreError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        error.code.stable_name(),
        error.code.subsystem().stable_name(),
        error.severity.stable_name(),
        error.retryable,
        &error.message,
    )
}
