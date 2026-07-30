use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};
use serde::{Deserialize, Serialize};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    CapabilitySnapshot, CommandReceipt, CoreDiagnostic, CoreNotification, CoreSnapshot, HostDraft,
    PlatformEffect, PlatformEffectRequest, RuntimeRecordValidationError, StorageEffect,
    StorageEffectRequest, TransportEffect, TransportEffectRequest,
};
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

/// Revision-bearing request used by commands without an additional payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RevisionCommandRequest {
    pub expected_revision: String,
}

/// Typed host-draft update submitted by the desktop setup screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct UpdateHostDraftRequest {
    pub expected_revision: String,
    pub session_name: String,
    pub approval_mode: String,
    pub invite_code: Option<String>,
    pub remember_approved_devices: bool,
}

/// Queue-admission evidence for one authoritative command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CommandReceiptDto {
    pub operation_id: String,
    pub accepted_at_revision: String,
}

impl From<CommandReceipt> for CommandReceiptDto {
    fn from(value: CommandReceipt) -> Self {
        Self {
            operation_id: value.operation_id.into_string(),
            accepted_at_revision: value.accepted_at_revision.get().to_string(),
        }
    }
}

/// Redacted selected-source summary. Native paths never cross IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AudioSourceSummaryDto {
    pub source_id: String,
    pub display_name: String,
    pub byte_length: Option<String>,
    pub duration_ms: Option<String>,
}

/// Authoritative Rust-owned host draft rendered by React.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostDraftDto {
    pub session_name: String,
    pub approval_mode: String,
    pub invite_code: Option<String>,
    pub audio_source: Option<AudioSourceSummaryDto>,
    pub remember_approved_devices: bool,
}

impl From<&HostDraft> for HostDraftDto {
    fn from(value: &HostDraft) -> Self {
        Self {
            session_name: value.session_name.clone(),
            approval_mode: value.approval_mode.wire_name().to_owned(),
            invite_code: value.invite_code.clone(),
            audio_source: value
                .audio_source
                .as_ref()
                .map(|source| AudioSourceSummaryDto {
                    source_id: source.source_id.clone(),
                    display_name: source.display_name.clone(),
                    byte_length: source.byte_length.map(|length| length.to_string()),
                    duration_ms: source.duration_ms.map(|duration| duration.to_string()),
                }),
            remember_approved_devices: value.remember_approved_devices,
        }
    }
}

/// Semantic platform capabilities relevant to host setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
/// Stable IPC capability bitmap; booleans map directly to generated frontend flags.
#[allow(clippy::struct_excessive_bools)]
pub struct CapabilitySnapshotDto {
    pub local_network_available: bool,
    pub audio_source_selection_available: bool,
    pub audio_output_available: bool,
    pub secure_store_available: bool,
}

impl From<CapabilitySnapshot> for CapabilitySnapshotDto {
    fn from(value: CapabilitySnapshot) -> Self {
        Self {
            local_network_available: value.local_network_available,
            audio_source_selection_available: value.audio_source_selection_available,
            audio_output_available: value.audio_output_available,
            secure_store_available: value.secure_store_available,
        }
    }
}

/// One authoritative host-draft validation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostDraftValidationDto {
    pub field: String,
    pub code: String,
    pub message: String,
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
    pub capabilities: CapabilitySnapshotDto,
    pub host_draft: HostDraftDto,
    pub host_draft_validation: Vec<HostDraftValidationDto>,
    pub can_create_host_session: bool,
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
        let host_draft_validation = host_draft_validation(&value.host_draft);
        let can_create_host_session = value.selected_role
            == Some(silent_disco_core::domain::AppRole::Host)
            && matches!(
                value.host_lifecycle,
                silent_disco_core::domain::HostLifecycle::Idle
                    | silent_disco_core::domain::HostLifecycle::Error
            )
            && host_draft_validation.is_empty();
        Self {
            revision: value.revision.get().to_string(),
            selected_role: value.selected_role.map(|role| role.wire_name().to_owned()),
            capabilities: CapabilitySnapshotDto::from(value.capabilities),
            host_draft: HostDraftDto::from(&value.host_draft),
            host_draft_validation,
            can_create_host_session,
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

/// Identifies the one active desktop notification subscription.
///
/// The identifier is encoded as a decimal string because JavaScript numbers cannot
/// represent every `u64` value exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct AttachNotificationResponse {
    pub subscription_id: String,
}

/// Redacted frontend-visible core effect. Native handles and payload details stay in Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PlatformEffectDto {
    pub operation_id: String,
    pub effect_kind: String,
}

impl From<PlatformEffect> for PlatformEffectDto {
    fn from(value: PlatformEffect) -> Self {
        Self {
            operation_id: value.operation_id.into_string(),
            effect_kind: platform_effect_name(&value.request).to_owned(),
        }
    }
}

impl From<TransportEffect> for PlatformEffectDto {
    fn from(value: TransportEffect) -> Self {
        Self {
            operation_id: value.operation_id.into_string(),
            effect_kind: transport_effect_name(&value.request).to_owned(),
        }
    }
}

impl From<StorageEffect> for PlatformEffectDto {
    fn from(value: StorageEffect) -> Self {
        Self {
            operation_id: value.operation_id.into_string(),
            effect_kind: storage_effect_name(&value.request).to_owned(),
        }
    }
}

/// One bounded non-secret diagnostic field emitted to the desktop frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DiagnosticFieldDto {
    pub key: String,
    pub value: String,
}

/// One bounded non-secret diagnostic event emitted to the desktop frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CoreDiagnosticDto {
    pub name: String,
    pub fields: Vec<DiagnosticFieldDto>,
}

impl From<CoreDiagnostic> for CoreDiagnosticDto {
    fn from(value: CoreDiagnostic) -> Self {
        Self {
            name: value.name,
            fields: value
                .fields
                .into_iter()
                .map(|field| DiagnosticFieldDto {
                    key: field.key,
                    value: field.value,
                })
                .collect(),
        }
    }
}

/// Authoritative, revisioned notification sent over the Tauri channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "details",
    rename_all = "camelCase"
)]
#[ts(tag = "kind", content = "details", rename_all = "camelCase")]
pub enum CoreNotificationDto {
    Snapshot(Box<CoreSnapshotDto>),
    Effect(PlatformEffectDto),
    Error(DesktopErrorDto),
    Diagnostic(CoreDiagnosticDto),
}

impl From<CoreNotification> for CoreNotificationDto {
    fn from(value: CoreNotification) -> Self {
        match value {
            CoreNotification::Snapshot(snapshot) => {
                Self::Snapshot(Box::new(CoreSnapshotDto::from(snapshot)))
            }
            CoreNotification::Effect(effect) => Self::Effect(PlatformEffectDto::from(effect)),
            CoreNotification::TransportEffect(effect) => {
                Self::Effect(PlatformEffectDto::from(effect))
            }
            CoreNotification::StorageEffect(effect) => {
                Self::Effect(PlatformEffectDto::from(effect))
            }
            CoreNotification::Error(error) => Self::Error(DesktopErrorDto::from(error)),
            CoreNotification::Diagnostic(diagnostic) => {
                Self::Diagnostic(CoreDiagnosticDto::from(diagnostic))
            }
        }
    }
}

fn platform_effect_name(request: &PlatformEffectRequest) -> &'static str {
    match request {
        PlatformEffectRequest::RequestCapabilities(_) => "request_capabilities",
        PlatformEffectRequest::StartAdvertising(_) => "start_advertising",
        PlatformEffectRequest::StopAdvertising => "stop_advertising",
        PlatformEffectRequest::StartDiscovery(_) => "start_discovery",
        PlatformEffectRequest::StopDiscovery => "stop_discovery",
        PlatformEffectRequest::EstablishNetwork(_) => "establish_network",
        PlatformEffectRequest::ReleaseNetwork => "release_network",
        PlatformEffectRequest::PrepareAudioSource(_) => "prepare_audio_source",
        PlatformEffectRequest::StartAudioOutput(_) => "start_audio_output",
        PlatformEffectRequest::StopAudioOutput => "stop_audio_output",
        PlatformEffectRequest::ShareDiagnostics { .. } => "share_diagnostics",
    }
}

fn transport_effect_name(request: &TransportEffectRequest) -> &'static str {
    match request {
        TransportEffectRequest::DeliverJoinApproval { .. } => "deliver_join_approval",
        TransportEffectRequest::DeliverJoinRejection { .. } => "deliver_join_rejection",
        TransportEffectRequest::DisconnectListener { .. } => "disconnect_listener",
    }
}

fn storage_effect_name(request: &StorageEffectRequest) -> &'static str {
    match request {
        StorageEffectRequest::PersistSettings { .. } => "persist_settings",
        StorageEffectRequest::PersistTrustedDevice { .. } => "persist_trusted_device",
    }
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

fn host_draft_validation(draft: &HostDraft) -> Vec<HostDraftValidationDto> {
    draft
        .validate_for_creation()
        .err()
        .map_or_else(Vec::new, |error| {
            let (field, code) = match error {
                RuntimeRecordValidationError::SessionName => ("sessionName", "session_name"),
                RuntimeRecordValidationError::AudioSourceId
                | RuntimeRecordValidationError::AudioSourceDisplayName
                | RuntimeRecordValidationError::AudioSourceLength
                | RuntimeRecordValidationError::AudioSourceRequired => {
                    ("audioSource", "audio_source")
                }
                RuntimeRecordValidationError::InviteCode
                | RuntimeRecordValidationError::InviteCodeRequired
                | RuntimeRecordValidationError::UnexpectedInviteCode => {
                    ("inviteCode", "invite_code")
                }
                RuntimeRecordValidationError::RevisionOverflow
                | RuntimeRecordValidationError::TuningSettings
                | RuntimeRecordValidationError::NetworkPort
                | RuntimeRecordValidationError::ProtocolVersion
                | RuntimeRecordValidationError::DeviceDisplayName
                | RuntimeRecordValidationError::DeliveryCount
                | RuntimeRecordValidationError::SynchronizationSummary
                | RuntimeRecordValidationError::DiagnosticName
                | RuntimeRecordValidationError::DiagnosticFieldKey
                | RuntimeRecordValidationError::DiagnosticFieldValue
                | RuntimeRecordValidationError::DiagnosticFieldLimit => {
                    ("form", "invalid_host_draft")
                }
            };
            vec![HostDraftValidationDto {
                field: field.to_owned(),
                code: code.to_owned(),
                message: error.to_string(),
            }]
        })
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
    use super::{AttachNotificationResponse, CoreNotificationDto, CoreSnapshotDto};
    use silent_disco_core::domain::{AppRole, HostLifecycle, OperationId, TransportState};
    use silent_disco_core::runtime::{
        CoreNotification, CoreSnapshot, PlatformEffect, PlatformEffectRequest, SnapshotRevision,
    };

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
        assert_eq!(dto.host_draft.approval_mode, "manual");
        assert_eq!(dto.host_draft_validation[0].field, "sessionName");
        assert!(!dto.can_create_host_session);
    }

    #[test]
    fn effect_conversion_redacts_native_payload_details() {
        let effect = PlatformEffect::new(
            OperationId::new("operation-1").expect("operation ID"),
            PlatformEffectRequest::StopAdvertising,
        )
        .expect("platform effect");

        let CoreNotificationDto::Effect(dto) =
            CoreNotificationDto::from(CoreNotification::Effect(effect))
        else {
            panic!("effect notification must remain an effect");
        };
        assert_eq!(dto.operation_id, "operation-1");
        assert_eq!(dto.effect_kind, "stop_advertising");
    }

    #[test]
    fn subscription_identifier_is_transport_safe() {
        let response = AttachNotificationResponse {
            subscription_id: u64::MAX.to_string(),
        };
        assert_eq!(response.subscription_id, "18446744073709551615");
    }
}
