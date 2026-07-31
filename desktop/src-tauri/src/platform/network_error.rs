use super::failure::DesktopPlatformFailure;
use crate::dto::DesktopErrorDto;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::transport::{TransportError, TransportErrorKind};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NetworkErrorKind {
    InvalidArgument,
    InvalidState,
    Unavailable,
    Ambiguous,
    ResourceLimit,
    Transport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DesktopNetworkError {
    pub(super) kind: NetworkErrorKind,
    pub(super) message: String,
    code: CoreErrorCode,
    retryable: bool,
}

impl DesktopNetworkError {
    pub(super) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::InvalidArgument,
            CoreErrorCode::InvalidArgument,
            message,
            false,
        )
    }

    pub(super) fn invalid_state(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::InvalidState,
            CoreErrorCode::InvalidStateTransition,
            message,
            false,
        )
    }

    pub(super) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::Unavailable,
            CoreErrorCode::TransportUnavailable,
            message,
            true,
        )
    }

    pub(super) fn ambiguous(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::Ambiguous,
            CoreErrorCode::InvalidArgument,
            message,
            false,
        )
    }

    pub(super) fn resource_limit(message: impl Into<String>) -> Self {
        Self::new(
            NetworkErrorKind::ResourceLimit,
            CoreErrorCode::ResourceLimitExceeded,
            message,
            false,
        )
    }

    pub(super) fn poisoned() -> Self {
        Self::new(
            NetworkErrorKind::InvalidState,
            CoreErrorCode::WorkerStopped,
            "desktop host network state mutex was poisoned",
            false,
        )
    }

    pub(super) fn transport(error: &TransportError) -> Self {
        let code = match error.kind {
            TransportErrorKind::Bind | TransportErrorKind::Listen => {
                CoreErrorCode::TransportUnavailable
            }
            TransportErrorKind::Timeout => CoreErrorCode::TransportTimeout,
            TransportErrorKind::ShuttingDown | TransportErrorKind::WorkerPanicked => {
                CoreErrorCode::ShutdownFailed
            }
            _ => CoreErrorCode::TransportConnectionFailed,
        };
        Self::new(NetworkErrorKind::Transport, code, error.to_string(), true)
    }

    pub(super) fn endpoint_mismatch(cleanup: Option<&TransportError>) -> Self {
        let message = cleanup.map_or_else(
            || "shared transport returned an endpoint for a different bind address".to_owned(),
            |cleanup| format!(
                "shared transport returned an endpoint for a different bind address; cleanup also failed: {cleanup}"
            ),
        );
        Self::new(
            NetworkErrorKind::Transport,
            CoreErrorCode::TransportUnavailable,
            message,
            false,
        )
    }

    fn new(
        kind: NetworkErrorKind,
        code: CoreErrorCode,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            retryable,
        }
    }

    pub(super) fn platform_failure(&self) -> DesktopPlatformFailure {
        DesktopPlatformFailure::new(
            self.code,
            self.message.clone(),
            ErrorSeverity::Error,
            self.retryable,
        )
    }

    pub(super) fn dto(self) -> DesktopErrorDto {
        DesktopErrorDto::new(
            &format!("desktop.network.{}", self.code.stable_name()),
            "transport",
            "error",
            self.retryable,
            &self.message,
        )
    }

    pub(super) fn core_error(
        self,
        operation_id: Option<silent_disco_core::domain::OperationId>,
    ) -> CoreError {
        let message = bounded_error_message(&self.message);
        CoreError::new(
            self.code,
            message,
            ErrorSeverity::Error,
            self.retryable,
            operation_id,
        )
        .expect("bounded desktop network error")
    }
}

fn bounded_error_message(message: &str) -> String {
    let mut output = String::new();
    for character in message.chars() {
        let next = character.len_utf8();
        if output.len().saturating_add(next) > silent_disco_core::error::MAX_ERROR_MESSAGE_BYTES {
            break;
        }
        output.push(character);
    }
    if output.is_empty() {
        "desktop network operation failed".to_owned()
    } else {
        output
    }
}

impl fmt::Display for DesktopNetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DesktopNetworkError {}
