use silent_disco_core::domain::OperationId;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity, MAX_ERROR_MESSAGE_BYTES};

/// Internal failure produced by a desktop-owned platform adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DesktopPlatformFailure {
    code: CoreErrorCode,
    message: String,
    severity: ErrorSeverity,
    retryable: bool,
}

impl DesktopPlatformFailure {
    #[must_use]
    pub(super) fn new(
        code: CoreErrorCode,
        message: impl Into<String>,
        severity: ErrorSeverity,
        retryable: bool,
    ) -> Self {
        let message = message.into();
        Self {
            code,
            message: bounded_message(&message),
            severity,
            retryable,
        }
    }

    #[must_use]
    pub(super) fn into_core_error(self, operation_id: OperationId) -> CoreError {
        core_error(
            self.code,
            self.message,
            self.severity,
            self.retryable,
            Some(operation_id),
        )
    }
}

#[must_use]
pub(super) fn core_error(
    code: CoreErrorCode,
    message: impl Into<String>,
    severity: ErrorSeverity,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    let message = message.into();
    CoreError::new(
        code,
        bounded_message(&message),
        severity,
        retryable,
        operation_id,
    )
    .unwrap_or_else(|_| {
        CoreError::new(
            CoreErrorCode::PlatformOperationFailed,
            "desktop platform operation failed",
            ErrorSeverity::Error,
            false,
            None,
        )
        .expect("static fallback desktop platform error is valid")
    })
}

fn bounded_message(message: &str) -> String {
    let mut output = String::new();
    for character in message.chars() {
        if output.len().saturating_add(character.len_utf8()) > MAX_ERROR_MESSAGE_BYTES {
            break;
        }
        output.push(character);
    }
    if output.is_empty() {
        "desktop platform operation failed".to_owned()
    } else {
        output
    }
}
