use core::fmt;
use std::error::Error;

use crate::runtime::{JoinRequestSummary, ListenerSummary, SessionAdvertisement};

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

pub(super) fn validate_unique_sessions(
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

pub(super) fn validate_unique_requests(
    requests: &[JoinRequestSummary],
) -> Result<(), RuntimeContractError> {
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

pub(super) fn validate_unique_listeners(
    listeners: &[ListenerSummary],
) -> Result<(), RuntimeContractError> {
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

pub(super) fn validate_token(
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
