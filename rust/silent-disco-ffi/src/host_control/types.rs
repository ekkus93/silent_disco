use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAppRole {
    Host,
    Listener,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiApprovalMode {
    Manual,
    TrustedDevices,
    InviteCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTrustState {
    Unknown,
    SessionOnly,
    Trusted,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiHostLifecycle {
    Idle,
    CreatingSession,
    Advertising,
    WaitingForListeners,
    Ready,
    Streaming,
    Paused,
    EndingSession,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiListenerLifecycle {
    Idle,
    Scanning,
    SessionSelected,
    JoinRequested,
    AwaitingApproval,
    Approved,
    Connecting,
    SyncingClock,
    Buffering,
    Playing,
    Reconnecting,
    Desynced,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTransportState {
    Idle,
    Discovering,
    Advertising,
    Connecting,
    Connected,
    Retrying,
    Disconnected,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPlaybackState {
    Stopped,
    Buffering,
    Ready,
    Playing,
    Paused,
    Underrun,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiAudioSource {
    pub source_id: String,
    pub display_name: String,
    pub size_bytes: Option<u64>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiTuningSettings {
    pub sync_sample_window: u32,
    pub sync_cadence_ms: u64,
    pub startup_buffer_ms: u64,
    pub late_packet_threshold_ms: u64,
    pub hard_resync_threshold_ms: u64,
    pub sync_drift_threshold_ms: f64,
    pub scan_window_ms: u64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiTuningPatch {
    pub sync_sample_window: Option<u32>,
    pub sync_cadence_ms: Option<u64>,
    pub startup_buffer_ms: Option<u64>,
    pub late_packet_threshold_ms: Option<u64>,
    pub hard_resync_threshold_ms: Option<u64>,
    pub sync_drift_threshold_ms: Option<f64>,
    pub scan_window_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiHostDraft {
    pub session_name: String,
    pub approval_mode: FfiApprovalMode,
    pub invite_code: Option<String>,
    pub audio_source: Option<FfiAudioSource>,
    pub remember_approved_devices: bool,
    pub tuning: FfiTuningSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiJoinRequest {
    pub request_id: String,
    pub device_id: String,
    pub display_name: String,
    pub trust_state: FfiTrustState,
    pub invite_code_valid: bool,
    pub received_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiJoinRequestInput {
    pub request_id: String,
    pub device_id: String,
    pub display_name: String,
    pub trust_state: FfiTrustState,
    pub invite_code: Option<String>,
    pub received_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiSessionAdvertisement {
    pub session_id: String,
    pub host_device_id: String,
    pub session_name: String,
    pub approval_mode: FfiApprovalMode,
    pub protocol_version: u16,
    pub address: Option<String>,
    pub control_port: Option<u16>,
    pub sync_port: Option<u16>,
    pub audio_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiSynchronizationSummary {
    pub confidence: String,
    pub offset_ms: f64,
    pub round_trip_ms: f64,
    pub drift_ppm: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiListenerSummary {
    pub device_id: String,
    pub display_name: String,
    pub trust_state: FfiTrustState,
    pub transport_state: FfiTransportState,
    pub synchronization: Option<FfiSynchronizationSummary>,
    pub last_contact_ms: Option<u64>,
    pub last_error: Option<FfiCoreError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiDeliveryReport {
    pub intended_peers: u32,
    pub successful_peers: u32,
    pub failed_peers: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiCoreError {
    pub code: String,
    pub subsystem: String,
    pub severity: String,
    pub retryable: bool,
    pub operation_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiCommandReceipt {
    pub operation_id: String,
    pub accepted_at_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiDiagnosticField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiCoreDiagnostic {
    pub name: String,
    pub fields: Vec<FfiDiagnosticField>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiCoreSnapshot {
    pub revision: u64,
    pub selected_role: Option<FfiAppRole>,
    pub host_draft: FfiHostDraft,
    pub host_lifecycle: FfiHostLifecycle,
    pub listener_lifecycle: FfiListenerLifecycle,
    pub transport_state: FfiTransportState,
    pub discovery_active: bool,
    pub discovered_sessions: Vec<FfiSessionAdvertisement>,
    pub selected_session: Option<String>,
    pub pending_join_requests: Vec<FfiJoinRequest>,
    pub listeners: Vec<FfiListenerSummary>,
    pub playback_state: FfiPlaybackState,
    pub playback_position_ms: u64,
    pub last_delivery: Option<FfiDeliveryReport>,
    pub recoverable_action: Option<String>,
    pub last_error: Option<FfiCoreError>,
    pub shutting_down: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum FfiPlatformEffect {
    RequestCapabilities {
        operation_id: String,
        capabilities: Vec<String>,
    },
    StartAdvertising {
        operation_id: String,
        session_id: String,
        host_device_id: String,
        session_name: String,
        approval_mode: FfiApprovalMode,
    },
    StopAdvertising {
        operation_id: String,
    },
    StartDiscovery {
        operation_id: String,
        scan_window_ms: u64,
    },
    StopDiscovery {
        operation_id: String,
    },
    EstablishNetwork {
        operation_id: String,
        session_id: String,
        address: Option<String>,
        control_port: Option<u16>,
        sync_port: Option<u16>,
        audio_port: Option<u16>,
    },
    ReleaseNetwork {
        operation_id: String,
    },
    PrepareAudioSource {
        operation_id: String,
        source: FfiAudioSource,
    },
    StartAudioOutput {
        operation_id: String,
        sample_rate_hz: u32,
        channels: u16,
    },
    StopAudioOutput {
        operation_id: String,
    },
    ShareDiagnostics {
        operation_id: String,
        export_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTransportEffect {
    DeliverJoinApproval {
        operation_id: String,
        request_id: String,
        session_id: String,
        listener_id: String,
        trusted_for_future: bool,
    },
    DeliverJoinRejection {
        operation_id: String,
        request_id: String,
        session_id: String,
        listener_id: String,
        reason_code: String,
    },
    DisconnectListener {
        operation_id: String,
        session_id: String,
        listener_id: String,
        reason_code: String,
    },
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum FfiStorageEffect {
    PersistSettings {
        operation_id: String,
        settings: FfiTuningSettings,
    },
    PersistTrustedDevice {
        operation_id: String,
        device_id: String,
        display_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum FfiPlatformCompletion {
    AdvertisingStarted,
    AdvertisingStopped,
    DiscoveryStarted,
    DiscoveryStopped,
    NetworkEndpointReady {
        address: String,
        control_port: u16,
        sync_port: u16,
        audio_port: u16,
    },
    NetworkReleased,
    AudioSourcePrepared {
        source: FfiAudioSource,
    },
    AudioOutputStarted {
        sample_rate_hz: u32,
        channels: u16,
        backend_name: String,
    },
    AudioOutputStopped,
    DiagnosticsShared {
        export_id: String,
    },
}

#[allow(
    clippy::large_enum_variant,
    reason = "UniFFI rich enums require owned record payloads at the foreign boundary"
)]
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum FfiCoreNotification {
    Snapshot { snapshot: FfiCoreSnapshot },
    PlatformEffect { effect: FfiPlatformEffect },
    TransportEffect { effect: FfiTransportEffect },
    StorageEffect { effect: FfiStorageEffect },
    Error { error: FfiCoreError },
    Diagnostic { diagnostic: FfiCoreDiagnostic },
}

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiBridgeError {
    Core(String),
    Callback(String),
    Closed(String),
}

impl fmt::Display for FfiBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(message) | Self::Callback(message) | Self::Closed(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for FfiBridgeError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for FfiBridgeError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Callback(error.to_string())
    }
}

#[uniffi::export(with_foreign)]
pub trait FfiCoreObserver: Send + Sync {
    /// Accepts one serialized notification from the authoritative Rust actor.
    ///
    /// # Errors
    ///
    /// Returns [`FfiBridgeError`] when the foreign observer cannot accept the notification.
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError>;
}
