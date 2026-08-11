use super::errors::{RuntimeContractError, validate_token};
use crate::domain::{AppRole, DeviceId, OperationId, RequestId, SessionId};
use crate::protocol::MAX_INVITE_CODE_BYTES;
use crate::runtime::{AudioSourceDescriptor, HostDraftPatch, SnapshotRevision, TuningPatch};

/// Presentation intent submitted to the authoritative actor.
#[derive(Debug, Clone, PartialEq)]
pub enum CoreCommand {
    SelectRole {
        role: AppRole,
    },
    UpdateHostDraft(HostDraftPatch),
    CreateHostSession,
    EndHostSession,
    StartDiscovery,
    StopDiscovery,
    SelectSession {
        session_id: SessionId,
    },
    SubmitJoin {
        invite_code: Option<String>,
    },
    CancelJoin,
    ApproveJoin {
        request_id: RequestId,
        remember_for_future: bool,
    },
    RejectJoin {
        request_id: RequestId,
    },
    RemoveListener {
        listener_id: DeviceId,
    },
    StartPlayback {
        source: AudioSourceDescriptor,
    },
    PausePlayback,
    ResumePlayback,
    StopPlayback,
    SetLocalVolume {
        linear_gain: f32,
    },
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
            } => validate_token(
                code,
                MAX_INVITE_CODE_BYTES,
                RuntimeContractError::InviteCode,
            ),
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
