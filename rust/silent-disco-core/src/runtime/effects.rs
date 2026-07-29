use crate::domain::{DeviceId, OperationId, RequestId, SessionId};
use crate::protocol::{MAX_DISPLAY_NAME_BYTES, MAX_REASON_BYTES};
use core::fmt;
use std::error::Error;

/// One transport operation requested by the authoritative actor.
///
/// The desktop or mobile transport adapter executes this work and returns a
/// correlated [`super::TransportEvent::DeliveryCompleted`] fact. Request
/// removal and listener-state changes remain owned by the actor.
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
}

impl TransportEffectRequest {
    /// Validates bounded transport-control fields before an effect is emitted.
    ///
    /// # Errors
    ///
    /// Rejects a blank, whitespace-containing, control-containing, or oversized
    /// reason code.
    pub fn validate(&self) -> Result<(), EffectContractError> {
        match self {
            Self::DeliverJoinRejection { reason_code, .. }
            | Self::DisconnectListener { reason_code, .. } => validate_token(
                reason_code,
                MAX_REASON_BYTES,
                EffectContractError::ControlReason,
            ),
            Self::DeliverJoinApproval { .. } => Ok(()),
        }
    }
}

/// Correlated transport work emitted by the authoritative actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEffect {
    pub operation_id: OperationId,
    pub request: TransportEffectRequest,
}

impl TransportEffect {
    /// Creates a validated transport effect.
    ///
    /// # Errors
    ///
    /// Returns a bounded contract error before notification delivery.
    pub fn new(
        operation_id: OperationId,
        request: TransportEffectRequest,
    ) -> Result<Self, EffectContractError> {
        request.validate()?;
        Ok(Self {
            operation_id,
            request,
        })
    }
}

/// One durable-storage operation requested by the authoritative actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageEffectRequest {
    PersistTrustedDevice {
        device_id: DeviceId,
        display_name: String,
    },
}

impl StorageEffectRequest {
    /// Validates bounded storage-effect fields before an effect is emitted.
    ///
    /// # Errors
    ///
    /// Rejects an invalid listener display name.
    pub fn validate(&self) -> Result<(), EffectContractError> {
        match self {
            Self::PersistTrustedDevice { display_name, .. } => validate_human_text(
                display_name,
                MAX_DISPLAY_NAME_BYTES,
                EffectContractError::DeviceDisplayName,
            ),
        }
    }
}

/// Correlated storage work emitted by the authoritative actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEffect {
    pub operation_id: OperationId,
    pub request: StorageEffectRequest,
}

impl StorageEffect {
    /// Creates a validated storage effect.
    ///
    /// # Errors
    ///
    /// Returns a bounded contract error before notification delivery.
    pub fn new(
        operation_id: OperationId,
        request: StorageEffectRequest,
    ) -> Result<Self, EffectContractError> {
        request.validate()?;
        Ok(Self {
            operation_id,
            request,
        })
    }
}

/// Stable failure while validating actor-emitted effect records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectContractError {
    ControlReason,
    DeviceDisplayName,
}

impl fmt::Display for EffectContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ControlReason => "transport control reason is invalid",
            Self::DeviceDisplayName => "device display name is invalid",
        })
    }
}

impl Error for EffectContractError {}

fn validate_token(
    value: &str,
    maximum_bytes: usize,
    error: EffectContractError,
) -> Result<(), EffectContractError> {
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

fn validate_human_text(
    value: &str,
    maximum_bytes: usize,
    error: EffectContractError,
) -> Result<(), EffectContractError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > maximum_bytes
        || value.chars().any(char::is_control)
    {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        EffectContractError, StorageEffect, StorageEffectRequest, TransportEffect,
        TransportEffectRequest,
    };
    use crate::domain::{DeviceId, OperationId, RequestId, SessionId};

    #[test]
    fn effects_validate_at_construction() {
        let operation_id = OperationId::new("effect-1").expect("valid operation ID");
        let device_id = DeviceId::new("listener-1").expect("valid device ID");
        TransportEffect::new(
            operation_id.clone(),
            TransportEffectRequest::DeliverJoinApproval {
                request_id: RequestId::new("request-1").expect("valid request ID"),
                session_id: SessionId::new("session-1").expect("valid session ID"),
                listener_id: device_id.clone(),
                trusted_for_future: false,
            },
        )
        .expect("valid transport effect");
        StorageEffect::new(
            operation_id,
            StorageEffectRequest::PersistTrustedDevice {
                device_id,
                display_name: "Listener One".to_owned(),
            },
        )
        .expect("valid storage effect");
    }

    #[test]
    fn reason_codes_reject_internal_whitespace() {
        let result = TransportEffect::new(
            OperationId::new("effect-2").expect("valid operation ID"),
            TransportEffectRequest::DeliverJoinRejection {
                request_id: RequestId::new("request-2").expect("valid request ID"),
                session_id: SessionId::new("session-2").expect("valid session ID"),
                listener_id: DeviceId::new("listener-2").expect("valid device ID"),
                reason_code: "bad reason".to_owned(),
            },
        );
        assert_eq!(result, Err(EffectContractError::ControlReason));
    }
}
