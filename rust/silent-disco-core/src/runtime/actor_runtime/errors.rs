use crate::domain::OperationId;
use crate::error::{CoreError, CoreErrorCode, ErrorSeverity};

pub(super) fn core_error(
    code: CoreErrorCode,
    message: impl Into<String>,
    severity: ErrorSeverity,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    CoreError {
        code,
        message: message.into(),
        subsystem: code.subsystem(),
        severity,
        retryable,
        operation_id,
        context: Vec::new(),
    }
}

pub(super) fn invalid_argument(
    message: impl Into<String>,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(
        CoreErrorCode::InvalidArgument,
        message,
        ErrorSeverity::Error,
        false,
        operation_id,
    )
}

pub(super) fn invalid_state(
    message: impl Into<String>,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(
        CoreErrorCode::InvalidStateTransition,
        message,
        ErrorSeverity::Error,
        false,
        operation_id,
    )
}

pub(super) fn resource_limit(
    message: impl Into<String>,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(
        CoreErrorCode::ResourceLimitExceeded,
        message,
        ErrorSeverity::Fatal,
        false,
        operation_id,
    )
}

pub(super) fn transport_delivery_failed(
    message: impl Into<String>,
    operation_id: OperationId,
) -> CoreError {
    core_error(
        CoreErrorCode::TransportDeliveryFailed,
        message,
        ErrorSeverity::Error,
        true,
        Some(operation_id),
    )
}

pub(super) fn worker_stopped(operation_id: Option<OperationId>) -> CoreError {
    core_error(
        CoreErrorCode::WorkerStopped,
        "authoritative actor worker is not available",
        ErrorSeverity::Fatal,
        false,
        operation_id,
    )
}

pub(super) fn shared_state_error(message: impl Into<String>) -> CoreError {
    core_error(
        CoreErrorCode::WorkerStopped,
        message,
        ErrorSeverity::Fatal,
        false,
        None,
    )
}
