use super::errors::{RuntimeContractError, validate_token};
use super::{MAX_EXPORT_ID_BYTES, MAX_STORAGE_TRUSTED_DEVICES};
use crate::domain::{DeviceId, OperationId, PlaybackState, SessionId, StreamId, TransportState};
use crate::error::CoreError;
use crate::runtime::{DeliveryReport, JoinRequestSummary, ListenerSummary, SynchronizationSummary};
use crate::storage::{StoredSettings, TrustedDevice};

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
    SessionEnded {
        session_id: SessionId,
    },
    Failed(CoreError),
    /// The host requires manual approval and is waiting on a decision.
    ///
    /// Listener-role only; arrives after the network endpoint is
    /// established but before the host has decided on the join request.
    AwaitingApproval,
    /// The host approved this listener's join request.
    ///
    /// Listener-role only. `trusted_for_future` mirrors
    /// [`crate::runtime::ApprovalDelivery::trusted_for_future`] on the host
    /// side, but persisting it locally is not yet implemented.
    JoinApproved {
        trusted_for_future: bool,
    },
    /// The host rejected this listener's join request.
    ///
    /// Listener-role only. `reason` is the host's rejection reason,
    /// preserved verbatim for diagnostics.
    JoinRejected {
        reason: String,
    },
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
    EndOfStream {
        stream_id: StreamId,
    },
    Underrun {
        missing_frames: u32,
    },
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
