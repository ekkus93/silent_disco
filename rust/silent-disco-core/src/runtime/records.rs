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
use crate::protocol::{MAX_INVITE_CODE_BYTES, PROTOCOL_VERSION};
use crate::storage::{StoredSettings, TrustedDevice};
use core::fmt;
use std::error::Error;

pub const MAX_DISCOVERED_SESSIONS: usize = 128;
pub const MAX_PENDING_JOIN_REQUESTS: usize = 128;
pub const MAX_CONNECTED_LISTENERS: usize = 256;
pub const MAX_CAPABILITY_REQUESTS: usize = 16;
pub const MAX_EXPORT_ID_BYTES: usize = 128;
pub const MAX_STORAGE_TRUSTED_DEVICES: usize = 1_024;

/// Presentation intent submitted to the authoritative actor.
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
    /// Validates command-local shape before queue admission.
    ///
    /// State-dependent legality remains the actor's responsibility.
    ///
    /// # Errors
    ///
    /// Rejects malformed invite codes and non-finite or out-of-range volume.
    pub fn validate_shape(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::SubmitJoin {
                invite_code: Some(code),
            } => validate_token(code, MAX_INVITE_CODE_BYTES, RuntimeContractError::InviteCode),
            Self::SetLocalVolume { linear_gain }
                if !linear_gain.is_finite() || !(0.0..=1.0).contains(linear_gain) =>
            {
                Err(RuntimeContractError::LinearGain)
            }
            _ => Ok(()),
        }
    }
}

/// Revision-aware command request. The actor assigns the operation ID.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreCommandRequest {
    pub expected_revision: SnapshotRevision,
    pub command: CoreCommand,
}

impl CoreCommandRequest {
    /// Creates a shape-validated request.
    ///
    /// # Errors
    ///
    /// Returns the command-local validation failure without queueing it.
    pub fn new(
        expected_revision: SnapshotRevision,
        command: CoreCommand,
    ) -> Result<Self, RuntimeContractError> {
        command.validate_shape()?;
        Ok(Self {
            expected_revision,
            command,
        })
    }
}

/// Receipt proving only that a command entered the actor queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceipt {
    pub operation_id: OperationId,
    pub accepted_at_revision: SnapshotRevision,
}

/// Semantic platform capabilities. Native permission names stay in platform code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionCapability {
    NearbyDiscovery,
    NearbyAdvertising,
    LocalNetwork,
    AudioSourceSelection,
    AudioOutput,
    SecureStore,
}

/// Request to start a bounded discovery scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryRequest {
    pub scan_window_ms: u64,
}

impl DiscoveryRequest {
    /// Creates a request from already validated shared tuning.
    #[must_use]
    pub const fn from_tuning(tuning: &TuningSettings) -> Self {
        Self {
            scan_window_ms: tuning.scan_window_ms,
        }
    }
}

/// Request to establish transport for a selected session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEstablishmentRequest {
    pub session_id: SessionId,
    pub endpoint: NetworkEndpoint,
}

/// Shared audio-output format request. It contains no native device handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputRequest {
    pub sample_rate_hz: u32,
    pub channels: u16,
}

impl AudioOutputRequest {
    /// Creates a nonzero audio-output format request.
    ///
    /// # Errors
    ///
    /// Rejects zero sample rate or channel count.
    pub const fn new(
        sample_rate_hz: u32,
        channels: u16,
    ) -> Result<Self, RuntimeContractError> {
        if sample_rate_hz == 0 || channels == 0 {
            return Err(RuntimeContractError::AudioOutputFormat);
        }
        Ok(Self {
            sample_rate_hz,
            channels,
        })
    }
}

/// Actual platform audio-output format reported after startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioOutputInfo {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub backend_name: String,
}

impl AudioOutputInfo {
    /// Creates a bounded actual-format record.
    ///
    /// # Errors
    ///
    /// Rejects zero format fields or an invalid backend token.
    pub fn new(
        sample_rate_hz: u32,
        channels: u16,
        backend_name: impl Into<String>,
    ) -> Result<Self, RuntimeContractError> {
        AudioOutputRequest::new(sample_rate_hz, channels)?;
        let backend_name = backend_name.into();
        validate_token(
            &backend_name,
            64,
            RuntimeContractError::AudioBackendName,
        )?;
        Ok(Self {
            sample_rate_hz,
            channels,
            backend_name,
        })
    }
}

/// Platform work requested by the actor. The wrapper operation ID is mandatory.
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
    /// Validates bounded platform-effect payloads.
    ///
    /// # Errors
    ///
    /// Rejects excessive capability lists, duplicates, and invalid export IDs.
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
            Self::ShareDiagnostics { export_id } => validate_token(
                export_id,
                MAX_EXPORT_ID_BYTES,
                RuntimeContractError::ExportId,
            ),
            _ => Ok(()),
        }
    }
}

/// Correlated request emitted to a platform adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformEffect {
    pub operation_id: OperationId,
    pub request: PlatformEffectRequest,
}

impl PlatformEffect {
    /// Creates a validated correlated effect.
    ///
    /// # Errors
    ///
    /// Returns an invalid payload failure before notification delivery.
    pub fn new(
        operation_id: OperationId,
        request: PlatformEffectRequest,
    ) -> Result<Self, RuntimeContractError> {
        request.validate()?;
        Ok(Self {
            operation_id,
            request,
        })
    }
}

/// Successful fact returned by a platform adapter.
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

/// Platform fact entering the actor. Completion events are always correlated.
#[derive(Debug, Clone, PartialEq)]
pub enum PlatformEvent {
    OperationSucceeded {
        operation_id: OperationId,
        completion: PlatformOperationCompletion,
    },
    OperationFailed {
        operation_id: OperationId,
        error: CoreError,
    },
    SessionDiscovered(SessionAdvertisement),
    SessionExpired { session_id: SessionId },
    CapabilityStateChanged(CapabilitySnapshot),
    AppEnteredForeground,
    AppEnteredBackground,
}

impl PlatformEvent {
    /// Returns the operation ID for a correlated completion.
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

/// Fact emitted by the shared transport runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    StateChanged(TransportState),
    JoinRequested(JoinRequestSummary),
    ListenerConnected(ListenerSummary),
    ListenerDisconnected {
        device_id: DeviceId,
        error: Option<CoreError>,
    },
    DeliveryCompleted {
        operation_id: OperationId,
        report: DeliveryReport,
    },
    SessionEnded { session_id: SessionId },
    Failed(CoreError),
}

/// Non-real-time fact emitted by decoder, scheduler, or output telemetry.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    PlaybackStateChanged(PlaybackState),
    PositionAdvanced {
        stream_id: StreamId,
        position_ms: u64,
    },
    SynchronizationUpdated {
        device_id: DeviceId,
        summary: SynchronizationSummary,
    },
    EndOfStream { stream_id: StreamId },
    Underrun { missing_frames: u32 },
    Failed(CoreError),
}

/// Successful result of one asynchronous storage request.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageCompletion {
    SettingsLoaded(Option<StoredSettings>),
    SettingsSaved,
    TrustedDevicesLoaded(Vec<TrustedDevice>),
    TrustedDeviceUpdated { device_id: DeviceId },
    DiagnosticsExportReady { export_id: String },
}

impl StorageCompletion {
    /// Validates bounded completion payloads before actor submission.
    ///
    /// # Errors
    ///
    /// Rejects excessive trusted-device collections or an invalid export ID.
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::TrustedDevicesLoaded(devices) if devices.len() > MAX_STORAGE_TRUSTED_DEVICES => {
                Err(RuntimeContractError::StoredTrustedDeviceLimit)
            }
            Self::DiagnosticsExportReady { export_id } => validate_token(
                export_id,
                MAX_EXPORT_ID_BYTES,
                RuntimeContractError::ExportId,
            ),
            _ => Ok(()),
        }
    }
}

/// Correlated database fact entering the actor.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageEvent {
    OperationSucceeded {
        operation_id: OperationId,
        completion: StorageCompletion,
    },
    OperationFailed {
        operation_id: OperationId,
        error: CoreError,
    },
}

impl StorageEvent {
    /// Returns the mandatory storage operation ID.
    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        match self {
            Self::OperationSucceeded { operation_id, .. }
            | Self::OperationFailed { operation_id, .. } => operation_id,
        }
    }

    /// Validates any bounded success payload.
    ///
    /// # Errors
    ///
    /// Returns a completion payload validation failure.
    pub fn validate(&self) -> Result<(), RuntimeContractError> {
        match self {
            Self::OperationSucceeded { completion, .. } => completion.validate(),
            Self::OperationFailed { .. } => Ok(()),
        }
    }
}

/// Presentation action that remains available after a recoverable failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverableAction {
    Retry,
    Rescan,
    Reconnect,
    Resynchronize,
    ReselectAudioSource,
}

/// Immutable authoritative presentation state.
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
    /// Validates bounded collections and cross-field state invariants.
    ///
    /// # Errors
    ///
    /// Rejects excessive collections, duplicate identifiers, discovery flag/state
    /// disagreement, or an unknown selected session.
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
            && !self
                .discovered_sessions
                .iter()
                .any(|advertisement| &advertisement.session_id == selected_session)
        {
            return Err(RuntimeContractError::UnknownSelectedSession);
        }
        Ok(())
    }
}

/// Output delivered by the notification dispatcher.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreNotification {
    Snapshot(CoreSnapshot),
    Effect(PlatformEffect),
    Error(CoreError),
    Diagnostic(CoreDiagnostic),
}

/// Source-ordered input accepted by the serialized actor.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreActorInput {
    Command {
        operation_id: OperationId,
        request: CoreCommandRequest,
    },
    Platform(PlatformEvent),
    Transport(TransportEvent),
    Audio(AudioEvent),
    Storage(StorageEvent),
}

impl CoreActorInput {
    /// Returns the correlation ID when the input is command- or operation-derived.
    #[must_use]
    pub const fn operation_id(&self) -> Option<&OperationId> {
        match self {
            Self::Command { operation_id, .. } => Some(operation_id),
            Self::Platform(event) => event.operation_id(),
            Self::Storage(event) => Some(event.operation_id()),
            Self::Transport(TransportEvent::DeliveryCompleted { operation_id, .. }) => {
                Some(operation_id)
            }
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

/// Stable failure while validating the actor's public contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeContractError {
    InviteCode,
    LinearGain,
    AudioOutputFormat,
    AudioBackendName,
    CapabilityList,
    ExportId,
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

fn validate_unique_sessions(
    sessions: &[SessionAdvertisement],
) -> Result<(), RuntimeContractError> {
    for (index, session) in sessions.iter().enumerate() {
        if sessions[..index]
            .iter()
            .any(|earlier| earlier.session_id == session.session_id)
        {
            return Err(RuntimeContractError::DuplicateSession);
        }
    }
    Ok(())
}

fn validate_unique_requests(requests: &[JoinRequestSummary]) -> Result<(), RuntimeContractError> {
    for (index, request) in requests.iter().enumerate() {
        if requests[..index]
            .iter()
            .any(|earlier| earlier.request_id == request.request_id)
        {
            return Err(RuntimeContractError::DuplicateRequest);
        }
    }
    Ok(())
}

fn validate_unique_listeners(listeners: &[ListenerSummary]) -> Result<(), RuntimeContractError> {
    for (index, listener) in listeners.iter().enumerate() {
        if listeners[..index]
            .iter()
            .any(|earlier| earlier.device_id == listener.device_id)
        {
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
        || value.trim() != value
        || value.len() > maximum_bytes
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(error);
    }
    Ok(())
}

/// Returns the shared wire-protocol version used in session advertisements.
#[must_use]
pub const fn current_protocol_version() -> u16 {
    PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::{
        AudioOutputRequest, CommandReceipt, CoreActorInput, CoreCommand, CoreCommandRequest,
        CoreSnapshot, PermissionCapability, PlatformEffect, PlatformEffectRequest,
        PlatformEvent, PlatformOperationCompletion, RuntimeContractError,
    };
    use crate::domain::{OperationId, TransportState};
    use crate::runtime::SnapshotRevision;

    #[test]
    fn command_receipt_represents_queue_admission_only() {
        let receipt = CommandReceipt {
            operation_id: OperationId::new("operation-1").expect("valid operation ID"),
            accepted_at_revision: SnapshotRevision::new(7),
        };
        assert_eq!(receipt.accepted_at_revision.get(), 7);
    }

    #[test]
    fn command_shape_rejects_invalid_volume_and_invite_code() {
        assert_eq!(
            CoreCommandRequest::new(
                SnapshotRevision::new(0),
                CoreCommand::SetLocalVolume {
                    linear_gain: f32::NAN,
                },
            ),
            Err(RuntimeContractError::LinearGain)
        );
        assert_eq!(
            CoreCommandRequest::new(
                SnapshotRevision::new(0),
                CoreCommand::SubmitJoin {
                    invite_code: Some("bad code".to_owned()),
                },
            ),
            Err(RuntimeContractError::InviteCode)
        );
    }

    #[test]
    fn effect_requires_valid_unique_capabilities_and_preserves_correlation() {
        let operation_id = OperationId::new("operation-2").expect("valid operation ID");
        let effect = PlatformEffect::new(
            operation_id.clone(),
            PlatformEffectRequest::RequestCapabilities(vec![
                PermissionCapability::LocalNetwork,
                PermissionCapability::AudioSourceSelection,
            ]),
        )
        .expect("valid effect");
        assert_eq!(effect.operation_id, operation_id);

        assert_eq!(
            PlatformEffect::new(
                OperationId::new("operation-3").expect("valid operation ID"),
                PlatformEffectRequest::RequestCapabilities(vec![
                    PermissionCapability::LocalNetwork,
                    PermissionCapability::LocalNetwork,
                ]),
            ),
            Err(RuntimeContractError::CapabilityList)
        );
    }

    #[test]
    fn completion_and_actor_input_return_same_operation_id() {
        let operation_id = OperationId::new("operation-4").expect("valid operation ID");
        let event = PlatformEvent::OperationSucceeded {
            operation_id: operation_id.clone(),
            completion: PlatformOperationCompletion::DiscoveryStarted,
        };
        let input = CoreActorInput::Platform(event);
        assert_eq!(input.operation_id(), Some(&operation_id));
    }

    #[test]
    fn snapshot_rejects_discovery_state_disagreement() {
        let snapshot = CoreSnapshot {
            discovery_active: true,
            transport_state: TransportState::Idle,
            ..CoreSnapshot::default()
        };
        assert_eq!(
            snapshot.validate(),
            Err(RuntimeContractError::DiscoveryStateMismatch)
        );
    }

    #[test]
    fn audio_output_request_rejects_zero_format() {
        assert_eq!(
            AudioOutputRequest::new(0, 2),
            Err(RuntimeContractError::AudioOutputFormat)
        );
        assert!(AudioOutputRequest::new(48_000, 2).is_ok());
    }
}
