use super::{
    AudioSourceDescriptor, CapabilitySnapshot, CoreDiagnostic, DeliveryReport, HostDraft,
    HostDraftPatch, JoinRequestSummary, ListenerSummary, NetworkEndpoint, SessionAdvertisement,
    SnapshotRevision, SynchronizationSummary, TuningPatch,
};
use crate::domain::{
    AppRole, DeviceId, HostLifecycle, ListenerLifecycle, OperationId, PlaybackState, RequestId,
    SessionId, StreamId, TransportState, TuningSettings,
};
use crate::error::CoreError;
use crate::protocol::{MAX_DISPLAY_NAME_BYTES, MAX_INVITE_CODE_BYTES, MAX_REASON_BYTES, PROTOCOL_VERSION};
use crate::storage::{StoredSettings, TrustedDevice};
use core::fmt;
use std::error::Error;

pub const MAX_DISCOVERED_SESSIONS: usize = 128;
pub const MAX_PENDING_JOIN_REQUESTS: usize = 128;
pub const MAX_CONNECTED_LISTENERS: usize = 256;
pub const MAX_CAPABILITY_REQUESTS: usize = 16;
pub const MAX_EXPORT_ID_BYTES: usize = 128;
pub const MAX_STORAGE_TRUSTED_DEVICES: usize = 1_024;

#[derive(Debug, Clone, PartialEq)]
pub enum CoreCommand {
    SelectRole { role: AppRole },
    UpdateHostDraft(HostDraftPatch),
    CreateHostSession,
    EndHostSession,
    StartDiscovery,
    StopDiscovery,
    SelectSession { session_id: SessionId },
    SubmitJoin { invite_code: Option<String> },
    CancelJoin,
    ApproveJoin { request_id: RequestId },
    RejectJoin { request_id: RequestId },
    RemoveListener { listener_id: DeviceId },
    StartPlayback { source: AudioSourceDescriptor },
    PausePlayback,
    ResumePlayback,
    StopPlayback,
    SetLocalVolume { linear_gain: f32 },
    RequestResync,
    RetryRecoverableFailure,
    UpdateTuning(TuningPatch),
    ExportDiagnostics,
    Shutdown,
}

impl CoreCommand {
    pub fn validate_shape(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::SubmitJoin { invite_code: Some(code) } => {
                validate_token(code, MAX_INVITE_CODE_BYTES, RuntimeContractError::InviteCode)
            }
            Self::SetLocalVolume { linear_gain }
                if !linear_gain.is_finite() || !(0.0..=1.0).contains(linear_gain) =>
            {
                Err(RuntimeContractError::LinearGain)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreCommandRequest {
    pub expected_revision: SnapshotRevision,
    pub command: CoreCommand,
}

impl CoreCommandRequest {
    pub fn new(
        expected_revision: SnapshotRevision,
        command: CoreCommand,
    ) -> Result<Self, RuntimeContractError> {
        command.validate_shape()?;
        Ok(Self { expected_revision, command })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceipt {
    pub operation_id: OperationId,
    pub accepted_at_revision: SnapshotRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionCapability {
    NearbyDiscovery,
    NearbyAdvertising,
    LocalNetwork,
    AudioSourceSelection,
    AudioOutput,
    SecureStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryRequest {
    pub scan_window_ms: u64,
}

impl DiscoveryRequest {
    #[must_use]
    pub const fn from_tuning(tuning: &TuningSettings) -> Self {
        Self { scan_window_ms: tuning.scan_window_ms }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEstablishmentRequest {
    pub session_id: SessionId,
    pub endpoint: NetworkEndpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputRequest {
    pub sample_rate_hz: u32,
    pub channels: u16,
}

impl AudioOutputRequest {
    pub const fn new(sample_rate_hz: u32, channels: u16) -> Result<Self, RuntimeContractError> {
        if sample_rate_hz == 0 || channels == 0 {
            return Err(RuntimeContractError::AudioOutputFormat);
        }
        Ok(Self { sample_rate_hz, channels })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOutputInfo {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub backend_name: String,
}

impl AudioOutputInfo {
    pub fn new(
        sample_rate_hz: u32,
        channels: u16,
        backend_name: impl Into<String>,
    ) -> Result<Self, RuntimeContractError> {
        AudioOutputRequest::new(sample_rate_hz, channels)?;
        let backend_name = backend_name.into();
        validate_token(&backend_name, 64, RuntimeContractError::AudioBackendName)?;
        Ok(Self { sample_rate_hz, channels, backend_name })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformEffectRequest {
    RequestCapabilities(Vec<PermissionCapability>),
    StartAdvertising(SessionAdvertisement),
    StopAdvertising,
    StartDiscovery(DiscoveryRequest),
    StopDiscovery,
    EstablishNetwork(NetworkEstablishmentRequest),
    ReleaseNetwork,
    PrepareAudioSource(AudioSourceDescriptor),
    StartAudioOutput(AudioOutputRequest),
    StopAudioOutput,
    ShareDiagnostics { export_id: String },
}

impl PlatformEffectRequest {
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::RequestCapabilities(capabilities) => {
                if capabilities.is_empty() || capabilities.len() > MAX_CAPABILITY_REQUESTS {
                    return Err(RuntimeContractError::CapabilityList);
                }
                for (index, capability) in capabilities.iter().enumerate() {
                    if capabilities[..index].contains(capability) {
                        return Err(RuntimeContractError::CapabilityList);
                    }
                }
                Ok(())
            }
            Self::ShareDiagnostics { export_id } => {
                validate_token(export_id, MAX_EXPORT_ID_BYTES, RuntimeContractError::ExportId)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlatformEffect {
    pub operation_id: OperationId,
    pub request: PlatformEffectRequest,
}

impl PlatformEffect {
    pub fn new(
        operation_id: OperationId,
        request: PlatformEffectRequest,
    ) -> Result<Self, RuntimeContractError> {
        request.validate()?;
        Ok(Self { operation_id, request })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportEffectRequest {
    DeliverJoinApproval {
        request_id: RequestId,
        session_id: SessionId,
        listener_id: DeviceId,
        trusted_for_future: bool,
    },
    DeliverJoinRejection {
        request_id: RequestId,
        session_id: SessionId,
        listener_id: DeviceId,
        reason_code: String,
    },
    DisconnectListener {
        session_id: SessionId,
        listener_id: DeviceId,
        reason_code: String,
    },
    StartHostPlayback {
        session_id: SessionId,
        source: AudioSourceDescriptor,
    },
    PauseHostPlayback { session_id: SessionId },
    ResumeHostPlayback { session_id: SessionId },
    StopHostPlayback { session_id: SessionId },
}

impl TransportEffectRequest {
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::DeliverJoinRejection { reason_code, .. }
            | Self::DisconnectListener { reason_code, .. } => {
                validate_token(reason_code, MAX_REASON_BYTES, RuntimeContractError::ControlReason)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEffect {
    pub operation_id: OperationId,
    pub request: TransportEffectRequest,
}

impl TransportEffect {
    pub fn new(
        operation_id: OperationId,
        request: TransportEffectRequest,
    ) -> Result<Self, RuntimeContractError> {
        request.validate()?;
        Ok(Self { operation_id, request })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageEffectRequest {
    PersistTrustedDevice { device_id: DeviceId, display_name: String },
}

impl StorageEffectRequest {
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::PersistTrustedDevice { display_name, .. } => validate_human_text(
                display_name,
                MAX_DISPLAY_NAME_BYTES,
                RuntimeContractError::DeviceDisplayName,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEffect {
    pub operation_id: OperationId,
    pub request: StorageEffectRequest,
}

impl StorageEffect {
    pub fn new(
        operation_id: OperationId,
        request: StorageEffectRequest,
    ) -> Result<Self, RuntimeContractError> {
        request.validate()?;
        Ok(Self { operation_id, request })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformOperationCompletion {
    CapabilitiesResolved(CapabilitySnapshot),
    AdvertisingStarted,
    AdvertisingStopped,
    DiscoveryStarted,
    DiscoveryStopped,
    NetworkEndpointReady(NetworkEndpoint),
    NetworkReleased,
    AudioSourcePrepared(AudioSourceDescriptor),
    AudioOutputStarted(AudioOutputInfo),
    AudioOutputStopped,
    DiagnosticsShared { export_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformEvent {
    OperationSucceeded { operation_id: OperationId, completion: PlatformOperationCompletion },
    OperationFailed { operation_id: OperationId, error: CoreError },
    SessionDiscovered(SessionAdvertisement),
    SessionExpired { session_id: SessionId },
    CapabilityStateChanged(CapabilitySnapshot),
    AppEnteredForeground,
    AppEnteredBackground,
}

impl PlatformEvent {
    #[must_use]
    pub const fn operation_id(&self) -> Option<&OperationId> {
        match self {
            Self::OperationSucceeded { operation_id, .. }
            | Self::OperationFailed { operation_id, .. } => Some(operation_id),
            Self::SessionDiscovered(_)
            | Self::SessionExpired { .. }
            | Self::CapabilityStateChanged(_)
            | Self::AppEnteredForeground
            | Self::AppEnteredBackground => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    StateChanged(TransportState),
    JoinRequested(JoinRequestSummary),
    ListenerConnected(ListenerSummary),
    ListenerDisconnected { device_id: DeviceId, error: Option<CoreError> },
    DeliveryCompleted { operation_id: OperationId, report: DeliveryReport },
    SessionEnded { session_id: SessionId },
    Failed(CoreError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    PlaybackStateChanged(PlaybackState),
    PositionAdvanced { stream_id: StreamId, position_ms: u64 },
    SynchronizationUpdated { device_id: DeviceId, summary: SynchronizationSummary },
    EndOfStream { stream_id: StreamId },
    Underrun { missing_frames: u32 },
    Failed(CoreError),
}

#[derive(Debug, Clone, PartialEq)]
pub enum StorageCompletion {
    SettingsLoaded(Option<StoredSettings>),
    SettingsSaved,
    TrustedDevicesLoaded(Vec<TrustedDevice>),
    TrustedDeviceUpdated { device_id: DeviceId },
    DiagnosticsExportReady { export_id: String },
}

impl StorageCompletion {
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::TrustedDevicesLoaded(devices) if devices.len() > MAX_STORAGE_TRUSTED_DEVICES => {
                Err(RuntimeContractError::StoredTrustedDeviceLimit)
            }
            Self::DiagnosticsExportReady { export_id } => {
                validate_token(export_id, MAX_EXPORT_ID_BYTES, RuntimeContractError::ExportId)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StorageEvent {
    OperationSucceeded { operation_id: OperationId, completion: StorageCompletion },
    OperationFailed { operation_id: OperationId, error: CoreError },
}

impl StorageEvent {
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        match self {
            Self::OperationSucceeded { operation_id, .. }
            | Self::OperationFailed { operation_id, .. } => operation_id,
        }
    }

    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::OperationSucceeded { completion, .. } => completion.validate(),
            Self::OperationFailed { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverableAction {
    Retry,
    Rescan,
    Reconnect,
    Resynchronize,
    ReselectAudioSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreSnapshot {
    pub revision: SnapshotRevision,
    pub selected_role: Option<AppRole>,
    pub capabilities: CapabilitySnapshot,
    pub host_draft: HostDraft,
    pub host_lifecycle: HostLifecycle,
    pub listener_lifecycle: ListenerLifecycle,
    pub transport_state: TransportState,
    pub discovery_active: bool,
    pub discovered_sessions: Vec<SessionAdvertisement>,
    pub selected_session: Option<SessionId>,
    pub pending_join_requests: Vec<JoinRequestSummary>,
    pub listeners: Vec<ListenerSummary>,
    pub playback_state: PlaybackState,
    pub playback_position_ms: u64,
    pub synchronization: Option<SynchronizationSummary>,
    pub tuning: TuningSettings,
    pub last_delivery: Option<DeliveryReport>,
    pub recoverable_action: Option<RecoverableAction>,
    pub last_error: Option<CoreError>,
    pub shutting_down: bool,
}

impl Default for CoreSnapshot {
    fn default() -> Self {
        Self {
            revision: SnapshotRevision::default(),
            selected_role: None,
            capabilities: CapabilitySnapshot::default(),
            host_draft: HostDraft::default(),
            host_lifecycle: HostLifecycle::Idle,
            listener_lifecycle: ListenerLifecycle::Idle,
            transport_state: TransportState::Idle,
            discovery_active: false,
            discovered_sessions: Vec::new(),
            selected_session: None,
            pending_join_requests: Vec::new(),
            listeners: Vec::new(),
            playback_state: PlaybackState::Stopped,
            playback_position_ms: 0,
            synchronization: None,
            tuning: TuningSettings::default(),
            last_delivery: None,
            recoverable_action: None,
            last_error: None,
            shutting_down: false,
        }
    }
}

impl CoreSnapshot {
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        if self.discovered_sessions.len() > MAX_DISCOVERED_SESSIONS {
            return Err(RuntimeContractError::DiscoveredSessionLimit);
        }
        if self.pending_join_requests.len() > MAX_PENDING_JOIN_REQUESTS {
            return Err(RuntimeContractError::PendingRequestLimit);
        }
        if self.listeners.len() > MAX_CONNECTED_LISTENERS {
            return Err(RuntimeContractError::ListenerLimit);
        }
        validate_unique_sessions(&self.discovered_sessions)?;
        validate_unique_requests(&self.pending_join_requests)?;
        validate_unique_listeners(&self.listeners)?;
        if self.discovery_active != (self.transport_state == TransportState::Discovering) {
            return Err(RuntimeContractError::DiscoveryStateMismatch);
        }
        if let Some(selected_session) = &self.selected_session
            && !self.discovered_sessions.iter().any(|item| &item.session_id == selected_session)
        {
            return Err(RuntimeContractError::UnknownSelectedSession);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreNotification {
    Snapshot(CoreSnapshot),
    Effect(PlatformEffect),
    TransportEffect(TransportEffect),
    StorageEffect(StorageEffect),
    Error(CoreError),
    Diagnostic(CoreDiagnostic),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreActorInput {
    Command { operation_id: OperationId, request: CoreCommandRequest },
    Platform(PlatformEvent),
    Transport(TransportEvent),
    Audio(AudioEvent),
    Storage(StorageEvent),
}

impl CoreActorInput {
    #[must_use]
    pub const fn operation_id(&self) -> Option<&OperationId> {
        match self {
            Self::Command { operation_id, .. } => Some(operation_id),
            Self::Platform(event) => event.operation_id(),
            Self::Storage(event) => Some(event.operation_id()),
            Self::Transport(TransportEvent::DeliveryCompleted { operation_id, .. }) => Some(operation_id),
            Self::Transport(
                TransportEvent::StateChanged(_)
                | TransportEvent::JoinRequested(_)
                | TransportEvent::ListenerConnected(_)
                | TransportEvent::ListenerDisconnected { .. }
                | TransportEvent::SessionEnded { .. }
                | TransportEvent::Failed(_),
            )
            | Self::Audio(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeContractError {
    InviteCode,
    LinearGain,
    AudioOutputFormat,
    AudioBackendName,
    CapabilityList,
    ExportId,
    ControlReason,
    DeviceDisplayName,
    StoredTrustedDeviceLimit,
    DiscoveredSessionLimit,
    PendingRequestLimit,
    ListenerLimit,
    DuplicateSession,
    DuplicateRequest,
    DuplicateListener,
    DiscoveryStateMismatch,
    UnknownSelectedSession,
}

impl fmt::Display for RuntimeContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InviteCode => "join invite code is invalid",
            Self::LinearGain => "linear gain must be finite and between zero and one",
            Self::AudioOutputFormat => "audio-output format fields must be nonzero",
            Self::AudioBackendName => "audio backend name is invalid",
            Self::CapabilityList => "capability request list is invalid",
            Self::ExportId => "diagnostics export identifier is invalid",
            Self::ControlReason => "transport control reason is invalid",
            Self::DeviceDisplayName => "device display name is invalid",
            Self::StoredTrustedDeviceLimit => "stored trusted-device limit exceeded",
            Self::DiscoveredSessionLimit => "discovered-session limit exceeded",
            Self::PendingRequestLimit => "pending join-request limit exceeded",
            Self::ListenerLimit => "listener limit exceeded",
            Self::DuplicateSession => "duplicate discovered session identifier",
            Self::DuplicateRequest => "duplicate pending request identifier",
            Self::DuplicateListener => "duplicate listener identifier",
            Self::DiscoveryStateMismatch => "discovery flag and transport state disagree",
            Self::UnknownSelectedSession => "selected session is not present in discovery results",
        })
    }
}
impl Error for RuntimeContractError {}

fn validate_unique_sessions(items: &[SessionAdvertisement]) -> Result<(), RuntimeContractError> {
    for (index, item) in items.iter().enumerate() {
        if items[..index].iter().any(|earlier| earlier.session_id == item.session_id) {
            return Err(RuntimeContractError::DuplicateSession);
        }
    }
    Ok(())
}
fn validate_unique_requests(items: &[JoinRequestSummary]) -> Result<(), RuntimeContractError> {
    for (index, item) in items.iter().enumerate() {
        if items[..index].iter().any(|earlier| earlier.request_id == item.request_id) {
            return Err(RuntimeContractError::DuplicateRequest);
        }
    }
    Ok(())
}
fn validate_unique_listeners(items: &[ListenerSummary]) -> Result<(), RuntimeContractError> {
    for (index, item) in items.iter().enumerate() {
        if items[..index].iter().any(|earlier| earlier.device_id == item.device_id) {
            return Err(RuntimeContractError::DuplicateListener);
        }
    }
    Ok(())
}
fn validate_token(
    value: &str,
    maximum_bytes: usize,
    error: RuntimeContractError,
) -> Result<(), RuntimeContractError> {
    if value.is_empty()
        || value.len() > maximum_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(error);
    }
    Ok(())
}
fn validate_human_text(
    value: &str,
    maximum_bytes: usize,
    error: RuntimeContractError,
) -> Result<(), RuntimeContractError> {
    if value.trim().is_empty()
        || value.len() > maximum_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(error);
    }
    Ok(())
}

#[must_use]
pub const fn current_protocol_version() -> u16 { PROTOCOL_VERSION }

#[cfg(test)]
mod tests {
    use super::{
        CoreNotification, RuntimeContractError, StorageEffect, StorageEffectRequest,
        TransportEffect, TransportEffectRequest,
    };
    use crate::domain::{DeviceId, OperationId, RequestId, SessionId};
    use crate::runtime::AudioSourceDescriptor;

    #[test]
    fn transport_and_storage_effects_validate_at_construction() {
        let operation_id = OperationId::new("effect-1").expect("valid operation ID");
        let request_id = RequestId::new("request-1").expect("valid request ID");
        let session_id = SessionId::new("session-1").expect("valid session ID");
        let device_id = DeviceId::new("listener-1").expect("valid device ID");
        let transport = TransportEffect::new(
            operation_id.clone(),
            TransportEffectRequest::DeliverJoinApproval {
                request_id,
                session_id,
                listener_id: device_id.clone(),
                trusted_for_future: false,
            },
        )
        .expect("valid transport effect");
        assert!(matches!(CoreNotification::TransportEffect(transport), CoreNotification::TransportEffect(_)));
        let storage = StorageEffect::new(
            operation_id,
            StorageEffectRequest::PersistTrustedDevice {
                device_id,
                display_name: "Listener One".to_owned(),
            },
        )
        .expect("valid storage effect");
        assert!(matches!(CoreNotification::StorageEffect(storage), CoreNotification::StorageEffect(_)));
    }

    #[test]
    fn rejects_invalid_control_reason() {
        let result = TransportEffect::new(
            OperationId::new("effect-2").expect("valid operation ID"),
            TransportEffectRequest::DeliverJoinRejection {
                request_id: RequestId::new("request-2").expect("valid request ID"),
                session_id: SessionId::new("session-2").expect("valid session ID"),
                listener_id: DeviceId::new("listener-2").expect("valid device ID"),
                reason_code: " bad ".to_owned(),
            },
        );
        assert_eq!(result, Err(RuntimeContractError::ControlReason));
    }

    #[test]
    fn start_playback_effect_keeps_native_paths_out_of_contract() {
        let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4), Some(20))
            .expect("valid source");
        let effect = TransportEffect::new(
            OperationId::new("effect-3").expect("valid operation ID"),
            TransportEffectRequest::StartHostPlayback {
                session_id: SessionId::new("session-3").expect("valid session ID"),
                source,
            },
        )
        .expect("valid playback effect");
        assert!(matches!(effect.request, TransportEffectRequest::StartHostPlayback { .. }));
    }
}
