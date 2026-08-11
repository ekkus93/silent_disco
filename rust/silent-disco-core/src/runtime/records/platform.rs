use super::errors::{RuntimeContractError, validate_token};
use super::{MAX_CAPABILITY_REQUESTS, MAX_EXPORT_ID_BYTES};
use crate::domain::{OperationId, SessionId, TuningSettings};
use crate::error::CoreError;
use crate::runtime::{
    AudioSourceDescriptor, CapabilitySnapshot, NetworkEndpoint, SessionAdvertisement,
};

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
///
/// `endpoint` is `None` when the platform must discover it as part of
/// establishment (e.g. Wi-Fi Direct, where the IP is only known after
/// `WifiP2pManager` connects) rather than it being known up front (e.g. a
/// manually entered host endpoint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEstablishmentRequest {
    pub session_id: SessionId,
    pub endpoint: Option<NetworkEndpoint>,
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
    pub const fn new(sample_rate_hz: u32, channels: u16) -> Result<Self, RuntimeContractError> {
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
        validate_token(&backend_name, 64, RuntimeContractError::AudioBackendName)?;
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
    SessionExpired {
        session_id: SessionId,
    },
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
