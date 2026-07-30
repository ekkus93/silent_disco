use silent_disco_core::domain::OperationId;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};

/// Internal failure produced by a desktop-owned platform adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DesktopPlatformFailure {
    code: CoreErrorCode,
    message: &'static str,
    severity: ErrorSeverity,
    retryable: bool,
}

impl DesktopPlatformFailure {
    #[must_use]
    pub(super) const fn new(
        code: CoreErrorCode,
        message: &'static str,
        severity: ErrorSeverity,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message,
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
    message: &'static str,
    severity: ErrorSeverity,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    match CoreError::new(code, message, severity, retryable, operation_id) {
        Ok(error) => error,
        Err(_) => unreachable!("static desktop platform error definition must be valid"),
    }
}
