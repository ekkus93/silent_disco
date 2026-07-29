use super::types::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCommandReceipt,
    FfiCoreDiagnostic, FfiCoreError, FfiCoreNotification, FfiCoreSnapshot, FfiDeliveryReport,
    FfiDiagnosticField, FfiHostDraft, FfiHostLifecycle, FfiJoinRequest, FfiListenerSummary,
    FfiPlatformCompletion, FfiPlatformEffect, FfiPlaybackState, FfiStorageEffect,
    FfiSynchronizationSummary, FfiTransportEffect, FfiTransportState, FfiTrustState,
    FfiTuningPatch, FfiTuningSettings,
};
use silent_disco_core::domain::{
    AppRole, ApprovalMode, HostLifecycle, PlaybackState, TransportState, TrustState, TuningSettings,
};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    AudioOutputInfo, AudioSourceDescriptor, CommandReceipt, CoreDiagnostic, CoreNotification,
    CoreSnapshot, DeliveryReport, NetworkEndpoint, PermissionCapability, PlatformEffect,
    PlatformEffectRequest, PlatformOperationCompletion, RecoverableAction, StorageEffect,
    StorageEffectRequest, SynchronizationSummary, TransportEffect, TransportEffectRequest,
    TuningPatch,
};
use std::net::IpAddr;
use std::str::FromStr;

impl From<CoreError> for FfiBridgeError {
    fn from(error: CoreError) -> Self {
        Self::Core(format!("{}: {}", error.code.stable_name(), error.message))
    }
}

impl From<AppRole> for FfiAppRole {
    fn from(value: AppRole) -> Self {
        match value {
            AppRole::Host => Self::Host,
            AppRole::Listener => Self::Listener,
        }
    }
}

impl From<FfiAppRole> for AppRole {
    fn from(value: FfiAppRole) -> Self {
        match value {
            FfiAppRole::Host => Self::Host,
            FfiAppRole::Listener => Self::Listener,
        }
    }
}

impl From<ApprovalMode> for FfiApprovalMode {
    fn from(value: ApprovalMode) -> Self {
        match value {
            ApprovalMode::Manual => Self::Manual,
            ApprovalMode::TrustedDevices => Self::TrustedDevices,
            ApprovalMode::InviteCode => Self::InviteCode,
        }
    }
}

impl From<FfiApprovalMode> for ApprovalMode {
    fn from(value: FfiApprovalMode) -> Self {
        match value {
            FfiApprovalMode::Manual => Self::Manual,
            FfiApprovalMode::TrustedDevices => Self::TrustedDevices,
            FfiApprovalMode::InviteCode => Self::InviteCode,
        }
    }
}

impl From<TrustState> for FfiTrustState {
    fn from(value: TrustState) -> Self {
        match value {
            TrustState::SessionOnly => Self::SessionOnly,
            TrustState::Trusted => Self::Trusted,
        }
    }
}

impl TryFrom<FfiTrustState> for TrustState {
    type Error = FfiBridgeError;

    fn try_from(value: FfiTrustState) -> Result<Self, Self::Error> {
        match value {
            FfiTrustState::SessionOnly => Ok(Self::SessionOnly),
            FfiTrustState::Trusted => Ok(Self::Trusted),
            FfiTrustState::Unknown | FfiTrustState::Revoked => Err(FfiBridgeError::Core(
                "join requests cannot use unknown or revoked trust state".to_owned(),
            )),
        }
    }
}

impl From<HostLifecycle> for FfiHostLifecycle {
    fn from(value: HostLifecycle) -> Self {
        match value {
            HostLifecycle::Idle => Self::Idle,
            HostLifecycle::CreatingSession => Self::CreatingSession,
            HostLifecycle::Advertising => Self::Advertising,
            HostLifecycle::WaitingForListeners => Self::WaitingForListeners,
            HostLifecycle::Ready => Self::Ready,
            HostLifecycle::Streaming => Self::Streaming,
            HostLifecycle::Paused => Self::Paused,
            HostLifecycle::EndingSession => Self::EndingSession,
            HostLifecycle::Error => Self::Error,
        }
    }
}

impl From<TransportState> for FfiTransportState {
    fn from(value: TransportState) -> Self {
        match value {
            TransportState::Idle => Self::Idle,
            TransportState::Discovering => Self::Discovering,
            TransportState::Advertising => Self::Advertising,
            TransportState::Connecting => Self::Connecting,
            TransportState::Connected => Self::Connected,
            TransportState::Retrying => Self::Retrying,
            TransportState::Disconnected => Self::Disconnected,
            TransportState::Failed => Self::Failed,
        }
    }
}

impl From<FfiTransportState> for TransportState {
    fn from(value: FfiTransportState) -> Self {
        match value {
            FfiTransportState::Idle => Self::Idle,
            FfiTransportState::Discovering => Self::Discovering,
            FfiTransportState::Advertising => Self::Advertising,
            FfiTransportState::Connecting => Self::Connecting,
            FfiTransportState::Connected => Self::Connected,
            FfiTransportState::Retrying => Self::Retrying,
            FfiTransportState::Disconnected => Self::Disconnected,
            FfiTransportState::Failed => Self::Failed,
        }
    }
}

impl From<PlaybackState> for FfiPlaybackState {
    fn from(value: PlaybackState) -> Self {
        match value {
            PlaybackState::Stopped => Self::Stopped,
            PlaybackState::Buffering => Self::Buffering,
            PlaybackState::Ready => Self::Ready,
            PlaybackState::Playing => Self::Playing,
            PlaybackState::Paused => Self::Paused,
            PlaybackState::Underrun => Self::Underrun,
            PlaybackState::Error => Self::Error,
        }
    }
}

impl From<FfiPlaybackState> for PlaybackState {
    fn from(value: FfiPlaybackState) -> Self {
        match value {
            FfiPlaybackState::Stopped => Self::Stopped,
            FfiPlaybackState::Buffering => Self::Buffering,
            FfiPlaybackState::Ready => Self::Ready,
            FfiPlaybackState::Playing => Self::Playing,
            FfiPlaybackState::Paused => Self::Paused,
            FfiPlaybackState::Underrun => Self::Underrun,
            FfiPlaybackState::Error => Self::Error,
        }
    }
}

impl From<AudioSourceDescriptor> for FfiAudioSource {
    fn from(value: AudioSourceDescriptor) -> Self {
        Self {
            source_id: value.source_id,
            display_name: value.display_name,
            size_bytes: value.byte_length,
            duration_ms: value.duration_ms,
        }
    }
}

impl TryFrom<FfiAudioSource> for AudioSourceDescriptor {
    type Error = FfiBridgeError;

    fn try_from(value: FfiAudioSource) -> Result<Self, Self::Error> {
        AudioSourceDescriptor::new(
            value.source_id,
            value.display_name,
            value.size_bytes,
            value.duration_ms,
        )
        .map_err(|error| FfiBridgeError::Core(error.to_string()))
    }
}

impl From<TuningSettings> for FfiTuningSettings {
    fn from(value: TuningSettings) -> Self {
        Self {
            sync_sample_window: u32::from(value.sync_sample_window),
            sync_cadence_ms: value.sync_cadence_ms,
            startup_buffer_ms: value.startup_buffer_ms,
            late_packet_threshold_ms: value.late_packet_threshold_ms,
            hard_resync_threshold_ms: value.hard_resync_threshold_ms,
            sync_drift_threshold_ms: value.sync_drift_threshold_ms,
            scan_window_ms: value.scan_window_ms,
        }
    }
}

impl TryFrom<FfiTuningPatch> for TuningPatch {
    type Error = FfiBridgeError;

    fn try_from(value: FfiTuningPatch) -> Result<Self, Self::Error> {
        Ok(Self {
            sync_sample_window: value
                .sync_sample_window
                .map(u16::try_from)
                .transpose()
                .map_err(|_| FfiBridgeError::Core("sync sample window exceeds u16".to_owned()))?,
            sync_cadence_ms: value.sync_cadence_ms,
            startup_buffer_ms: value.startup_buffer_ms,
            late_packet_threshold_ms: value.late_packet_threshold_ms,
            hard_resync_threshold_ms: value.hard_resync_threshold_ms,
            sync_drift_threshold_ms: value.sync_drift_threshold_ms,
            scan_window_ms: value.scan_window_ms,
        })
    }
}

impl From<DeliveryReport> for FfiDeliveryReport {
    fn from(value: DeliveryReport) -> Self {
        Self {
            intended_peers: value.intended_peers,
            successful_peers: value.successful_peers,
            failed_peers: value.failed_peers,
        }
    }
}

impl TryFrom<FfiDeliveryReport> for DeliveryReport {
    type Error = FfiBridgeError;

    fn try_from(value: FfiDeliveryReport) -> Result<Self, Self::Error> {
        DeliveryReport::new(
            value.intended_peers,
            value.successful_peers,
            value.failed_peers,
        )
        .map_err(|error| FfiBridgeError::Core(error.to_string()))
    }
}

impl From<CoreError> for FfiCoreError {
    fn from(value: CoreError) -> Self {
        Self {
            code: value.code.stable_name().to_owned(),
            subsystem: value.subsystem.stable_name().to_owned(),
            severity: value.severity.stable_name().to_owned(),
            retryable: value.retryable,
            operation_id: value.operation_id.map(|value| value.into_string()),
            message: value.message,
        }
    }
}

impl From<CommandReceipt> for FfiCommandReceipt {
    fn from(value: CommandReceipt) -> Self {
        Self {
            operation_id: value.operation_id.into_string(),
            accepted_at_revision: value.accepted_at_revision.get(),
        }
    }
}

impl From<CoreDiagnostic> for FfiCoreDiagnostic {
    fn from(value: CoreDiagnostic) -> Self {
        Self {
            name: value.name,
            fields: value
                .fields
                .into_iter()
                .map(|field| FfiDiagnosticField {
                    key: field.key,
                    value: field.value,
                })
                .collect(),
        }
    }
}

impl From<SynchronizationSummary> for FfiSynchronizationSummary {
    fn from(value: SynchronizationSummary) -> Self {
        Self {
            confidence: value.confidence.wire_name().to_owned(),
            offset_ms: value.offset_ms,
            round_trip_ms: value.round_trip_ms,
            drift_ppm: value.drift_ppm,
        }
    }
}

impl From<silent_disco_core::runtime::JoinRequestSummary> for FfiJoinRequest {
    fn from(value: silent_disco_core::runtime::JoinRequestSummary) -> Self {
        Self {
            request_id: value.request_id.into_string(),
            device_id: value.device_id.into_string(),
            display_name: value.display_name,
            trust_state: value.trust_state.into(),
            invite_code_valid: value.invite_code_valid,
            received_at_ms: value.received_at.get(),
        }
    }
}

impl From<silent_disco_core::runtime::ListenerSummary> for FfiListenerSummary {
    fn from(value: silent_disco_core::runtime::ListenerSummary) -> Self {
        Self {
            device_id: value.device_id.into_string(),
            display_name: value.display_name,
            trust_state: value.trust_state.into(),
            transport_state: value.transport_state.into(),
            synchronization: value.synchronization.map(Into::into),
            last_contact_ms: value.last_contact.map(|value| value.get()),
            last_error: value.last_error.map(Into::into),
        }
    }
}

impl From<CoreSnapshot> for FfiCoreSnapshot {
    fn from(value: CoreSnapshot) -> Self {
        let tuning = value.tuning.into();
        let host_draft = FfiHostDraft {
            session_name: value.host_draft.session_name,
            approval_mode: value.host_draft.approval_mode.into(),
            invite_code: value.host_draft.invite_code,
            audio_source: value.host_draft.audio_source.map(Into::into),
            remember_approved_devices: value.host_draft.remember_approved_devices,
            tuning,
        };
        Self {
            revision: value.revision.get(),
            selected_role: value.selected_role.map(Into::into),
            host_draft,
            host_lifecycle: value.host_lifecycle.into(),
            listener_lifecycle: value.listener_lifecycle.wire_name().to_owned(),
            transport_state: value.transport_state.into(),
            discovery_active: value.discovery_active,
            pending_join_requests: value
                .pending_join_requests
                .into_iter()
                .map(Into::into)
                .collect(),
            listeners: value.listeners.into_iter().map(Into::into).collect(),
            playback_state: value.playback_state.into(),
            playback_position_ms: value.playback_position_ms,
            last_delivery: value.last_delivery.map(Into::into),
            recoverable_action: value.recoverable_action.map(recoverable_action_name),
            last_error: value.last_error.map(Into::into),
            shutting_down: value.shutting_down,
        }
    }
}

impl From<PlatformEffect> for FfiPlatformEffect {
    fn from(value: PlatformEffect) -> Self {
        let operation_id = value.operation_id.into_string();
        match value.request {
            PlatformEffectRequest::RequestCapabilities(capabilities) => Self::RequestCapabilities {
                operation_id,
                capabilities: capabilities
                    .into_iter()
                    .map(capability_name)
                    .map(str::to_owned)
                    .collect(),
            },
            PlatformEffectRequest::StartAdvertising(advertisement) => Self::StartAdvertising {
                operation_id,
                session_id: advertisement.session_id.into_string(),
                host_device_id: advertisement.host_device_id.into_string(),
                session_name: advertisement.session_name,
                approval_mode: advertisement.approval_mode.into(),
            },
            PlatformEffectRequest::StopAdvertising => Self::StopAdvertising { operation_id },
            PlatformEffectRequest::StartDiscovery(request) => Self::StartDiscovery {
                operation_id,
                scan_window_ms: request.scan_window_ms,
            },
            PlatformEffectRequest::StopDiscovery => Self::StopDiscovery { operation_id },
            PlatformEffectRequest::EstablishNetwork(request) => Self::EstablishNetwork {
                operation_id,
                session_id: request.session_id.into_string(),
                address: request.endpoint.address.to_string(),
                control_port: request.endpoint.control_port,
                sync_port: request.endpoint.sync_port,
                audio_port: request.endpoint.audio_port,
            },
            PlatformEffectRequest::ReleaseNetwork => Self::ReleaseNetwork { operation_id },
            PlatformEffectRequest::PrepareAudioSource(source) => Self::PrepareAudioSource {
                operation_id,
                source: source.into(),
            },
            PlatformEffectRequest::StartAudioOutput(request) => Self::StartAudioOutput {
                operation_id,
                sample_rate_hz: request.sample_rate_hz,
                channels: request.channels,
            },
            PlatformEffectRequest::StopAudioOutput => Self::StopAudioOutput { operation_id },
            PlatformEffectRequest::ShareDiagnostics { export_id } => Self::ShareDiagnostics {
                operation_id,
                export_id,
            },
        }
    }
}

impl From<TransportEffect> for FfiTransportEffect {
    fn from(value: TransportEffect) -> Self {
        let operation_id = value.operation_id.into_string();
        match value.request {
            TransportEffectRequest::DeliverJoinApproval {
                request_id,
                session_id,
                listener_id,
                trusted_for_future,
            } => Self::DeliverJoinApproval {
                operation_id,
                request_id: request_id.into_string(),
                session_id: session_id.into_string(),
                listener_id: listener_id.into_string(),
                trusted_for_future,
            },
            TransportEffectRequest::DeliverJoinRejection {
                request_id,
                session_id,
                listener_id,
                reason_code,
            } => Self::DeliverJoinRejection {
                operation_id,
                request_id: request_id.into_string(),
                session_id: session_id.into_string(),
                listener_id: listener_id.into_string(),
                reason_code,
            },
            TransportEffectRequest::DisconnectListener {
                session_id,
                listener_id,
                reason_code,
            } => Self::DisconnectListener {
                operation_id,
                session_id: session_id.into_string(),
                listener_id: listener_id.into_string(),
                reason_code,
            },
        }
    }
}

impl From<StorageEffect> for FfiStorageEffect {
    fn from(value: StorageEffect) -> Self {
        let operation_id = value.operation_id.into_string();
        match value.request {
            StorageEffectRequest::PersistSettings { settings } => Self::PersistSettings {
                operation_id,
                settings: settings.into(),
            },
            StorageEffectRequest::PersistTrustedDevice {
                device_id,
                display_name,
            } => Self::PersistTrustedDevice {
                operation_id,
                device_id: device_id.into_string(),
                display_name,
            },
        }
    }
}

impl From<CoreNotification> for FfiCoreNotification {
    fn from(value: CoreNotification) -> Self {
        match value {
            CoreNotification::Snapshot(snapshot) => Self::Snapshot {
                snapshot: snapshot.into(),
            },
            CoreNotification::Effect(effect) => Self::PlatformEffect {
                effect: effect.into(),
            },
            CoreNotification::TransportEffect(effect) => Self::TransportEffect {
                effect: effect.into(),
            },
            CoreNotification::StorageEffect(effect) => Self::StorageEffect {
                effect: effect.into(),
            },
            CoreNotification::Error(error) => Self::Error {
                error: error.into(),
            },
            CoreNotification::Diagnostic(diagnostic) => Self::Diagnostic {
                diagnostic: diagnostic.into(),
            },
        }
    }
}

impl TryFrom<FfiPlatformCompletion> for PlatformOperationCompletion {
    type Error = FfiBridgeError;

    fn try_from(value: FfiPlatformCompletion) -> Result<Self, Self::Error> {
        match value {
            FfiPlatformCompletion::AdvertisingStarted => Ok(Self::AdvertisingStarted),
            FfiPlatformCompletion::AdvertisingStopped => Ok(Self::AdvertisingStopped),
            FfiPlatformCompletion::DiscoveryStarted => Ok(Self::DiscoveryStarted),
            FfiPlatformCompletion::DiscoveryStopped => Ok(Self::DiscoveryStopped),
            FfiPlatformCompletion::NetworkEndpointReady {
                address,
                control_port,
                sync_port,
                audio_port,
            } => Ok(Self::NetworkEndpointReady(network_endpoint(
                &address,
                control_port,
                sync_port,
                audio_port,
            )?)),
            FfiPlatformCompletion::NetworkReleased => Ok(Self::NetworkReleased),
            FfiPlatformCompletion::AudioSourcePrepared { source } => {
                Ok(Self::AudioSourcePrepared(source.try_into()?))
            }
            FfiPlatformCompletion::AudioOutputStarted {
                sample_rate_hz,
                channels,
                backend_name,
            } => Ok(Self::AudioOutputStarted(
                AudioOutputInfo::new(sample_rate_hz, channels, backend_name)
                    .map_err(|error| FfiBridgeError::Core(error.to_string()))?,
            )),
            FfiPlatformCompletion::AudioOutputStopped => Ok(Self::AudioOutputStopped),
            FfiPlatformCompletion::DiagnosticsShared { export_id } => {
                Ok(Self::DiagnosticsShared { export_id })
            }
        }
    }
}

pub(super) fn network_endpoint(
    address: &str,
    control_port: u16,
    sync_port: u16,
    audio_port: u16,
) -> Result<NetworkEndpoint, FfiBridgeError> {
    let address = IpAddr::from_str(address)
        .map_err(|_| FfiBridgeError::Core("network address is invalid".to_owned()))?;
    NetworkEndpoint::new(address, control_port, sync_port, audio_port)
        .map_err(|error| FfiBridgeError::Core(error.to_string()))
}

fn recoverable_action_name(value: RecoverableAction) -> String {
    match value {
        RecoverableAction::Retry => "retry",
        RecoverableAction::Rescan => "rescan",
        RecoverableAction::Reconnect => "reconnect",
        RecoverableAction::Resynchronize => "resynchronize",
        RecoverableAction::ReselectAudioSource => "reselect_audio_source",
    }
    .to_owned()
}

fn capability_name(value: PermissionCapability) -> &'static str {
    match value {
        PermissionCapability::NearbyDiscovery => "nearby_discovery",
        PermissionCapability::NearbyAdvertising => "nearby_advertising",
        PermissionCapability::LocalNetwork => "local_network",
        PermissionCapability::AudioSourceSelection => "audio_source_selection",
        PermissionCapability::AudioOutput => "audio_output",
        PermissionCapability::SecureStore => "secure_store",
    }
}
