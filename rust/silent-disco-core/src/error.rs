use core::fmt;
use std::error::Error;

use crate::domain::{IdValidationError, IdValidationReason, OperationId};

pub const MAX_ERROR_MESSAGE_BYTES: usize = 512;
pub const MAX_ERROR_CONTEXT_ENTRIES: usize = 16;
pub const MAX_ERROR_CONTEXT_KEY_BYTES: usize = 64;
pub const MAX_ERROR_CONTEXT_VALUE_BYTES: usize = 256;

/// Stable subsystem classification for every core failure.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreSubsystem {
    Validation = 1,
    Protocol = 2,
    Transport = 3,
    Synchronization = 4,
    Audio = 5,
    Storage = 6,
    Platform = 7,
    Ffi = 8,
    Runtime = 9,
}

impl CoreSubsystem {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Protocol => "protocol",
            Self::Transport => "transport",
            Self::Synchronization => "synchronization",
            Self::Audio => "audio",
            Self::Storage => "storage",
            Self::Platform => "platform",
            Self::Ffi => "ffi",
            Self::Runtime => "runtime",
        }
    }
}

impl fmt::Display for CoreSubsystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

/// Severity determines visibility and whether the core may continue operating.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorSeverity {
    Warning = 1,
    Error = 2,
    Fatal = 3,
}

impl ErrorSeverity {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }
}

impl fmt::Display for ErrorSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

/// Stable error codes grouped by subsystem. Numeric values are part of the
/// binding and diagnostic contract and must not be renumbered.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreErrorCode {
    InvalidIdentifier = 1000,
    InvalidArgument = 1001,
    InvalidStateTransition = 1002,
    ResourceLimitExceeded = 1003,

    UnsupportedProtocolVersion = 2000,
    MalformedProtocolFrame = 2001,
    ProtocolFrameTooLarge = 2002,
    UnsupportedMessageKind = 2003,
    IntegrityCheckFailed = 2004,

    TransportUnavailable = 3000,
    TransportConnectionFailed = 3001,
    TransportConnectionLost = 3002,
    TransportDeliveryFailed = 3003,
    TransportTimeout = 3004,

    InvalidSyncSample = 4000,
    SynchronizationTimeout = 4001,
    SynchronizationConfidenceLost = 4002,

    InvalidAudioConfiguration = 5000,
    AudioOutputStartFailed = 5001,
    AudioOutputFailed = 5002,
    AudioUnderrun = 5003,
    AudioEngineUnavailable = 5004,

    StorageOpenFailed = 6000,
    StorageMigrationFailed = 6001,
    StorageIntegrityFailed = 6002,
    StorageReadFailed = 6003,
    StorageWriteFailed = 6004,
    StoragePragmaFailed = 6005,
    StorageTransactionFailed = 6006,
    StorageConstraintViolation = 6007,
    StorageBusy = 6008,
    StorageCorrupt = 6009,
    StorageCloseFailed = 6010,
    StorageQueryFailed = 6011,

    PlatformOperationFailed = 7000,
    PermissionDenied = 7001,
    CapabilityUnavailable = 7002,

    FfiInvalidArgument = 8000,
    FfiIncompatibleVersion = 8001,
    FfiPanicContained = 8002,
    FfiCallbackFailed = 8003,

    QueueOverflow = 9000,
    ShutdownInProgress = 9001,
    ShutdownFailed = 9002,
    WorkerStopped = 9003,
}

impl CoreErrorCode {
    #[must_use]
    pub const fn numeric_code(self) -> u16 {
        self as u16
    }

    #[must_use]
    pub const fn subsystem(self) -> CoreSubsystem {
        match self {
            Self::InvalidIdentifier
            | Self::InvalidArgument
            | Self::InvalidStateTransition
            | Self::ResourceLimitExceeded => CoreSubsystem::Validation,
            Self::UnsupportedProtocolVersion
            | Self::MalformedProtocolFrame
            | Self::ProtocolFrameTooLarge
            | Self::UnsupportedMessageKind
            | Self::IntegrityCheckFailed => CoreSubsystem::Protocol,
            Self::TransportUnavailable
            | Self::TransportConnectionFailed
            | Self::TransportConnectionLost
            | Self::TransportDeliveryFailed
            | Self::TransportTimeout => CoreSubsystem::Transport,
            Self::InvalidSyncSample
            | Self::SynchronizationTimeout
            | Self::SynchronizationConfidenceLost => CoreSubsystem::Synchronization,
            Self::InvalidAudioConfiguration
            | Self::AudioOutputStartFailed
            | Self::AudioOutputFailed
            | Self::AudioUnderrun
            | Self::AudioEngineUnavailable => CoreSubsystem::Audio,
            Self::StorageOpenFailed
            | Self::StorageMigrationFailed
            | Self::StorageIntegrityFailed
            | Self::StorageReadFailed
            | Self::StorageWriteFailed
            | Self::StoragePragmaFailed
            | Self::StorageTransactionFailed
            | Self::StorageConstraintViolation
            | Self::StorageBusy
            | Self::StorageCorrupt
            | Self::StorageCloseFailed
            | Self::StorageQueryFailed => CoreSubsystem::Storage,
            Self::PlatformOperationFailed
            | Self::PermissionDenied
            | Self::CapabilityUnavailable => CoreSubsystem::Platform,
            Self::FfiInvalidArgument
            | Self::FfiIncompatibleVersion
            | Self::FfiPanicContained
            | Self::FfiCallbackFailed => CoreSubsystem::Ffi,
            Self::QueueOverflow
            | Self::ShutdownInProgress
            | Self::ShutdownFailed
            | Self::WorkerStopped => CoreSubsystem::Runtime,
        }
    }

    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::InvalidIdentifier => "invalid_identifier",
            Self::InvalidArgument => "invalid_argument",
            Self::InvalidStateTransition => "invalid_state_transition",
            Self::ResourceLimitExceeded => "resource_limit_exceeded",
            Self::UnsupportedProtocolVersion => "unsupported_protocol_version",
            Self::MalformedProtocolFrame => "malformed_protocol_frame",
            Self::ProtocolFrameTooLarge => "protocol_frame_too_large",
            Self::UnsupportedMessageKind => "unsupported_message_kind",
            Self::IntegrityCheckFailed => "integrity_check_failed",
            Self::TransportUnavailable => "transport_unavailable",
            Self::TransportConnectionFailed => "transport_connection_failed",
            Self::TransportConnectionLost => "transport_connection_lost",
            Self::TransportDeliveryFailed => "transport_delivery_failed",
            Self::TransportTimeout => "transport_timeout",
            Self::InvalidSyncSample => "invalid_sync_sample",
            Self::SynchronizationTimeout => "synchronization_timeout",
            Self::SynchronizationConfidenceLost => "synchronization_confidence_lost",
            Self::InvalidAudioConfiguration => "invalid_audio_configuration",
            Self::AudioOutputStartFailed => "audio_output_start_failed",
            Self::AudioOutputFailed => "audio_output_failed",
            Self::AudioUnderrun => "audio_underrun",
            Self::AudioEngineUnavailable => "audio_engine_unavailable",
            Self::StorageOpenFailed => "storage_open_failed",
            Self::StorageMigrationFailed => "storage_migration_failed",
            Self::StorageIntegrityFailed => "storage_integrity_failed",
            Self::StorageReadFailed => "storage_read_failed",
            Self::StorageWriteFailed => "storage_write_failed",
            Self::StoragePragmaFailed => "storage_pragma_failed",
            Self::StorageTransactionFailed => "storage_transaction_failed",
            Self::StorageConstraintViolation => "storage_constraint_violation",
            Self::StorageBusy => "storage_busy",
            Self::StorageCorrupt => "storage_corrupt",
            Self::StorageCloseFailed => "storage_close_failed",
            Self::StorageQueryFailed => "storage_query_failed",
            Self::PlatformOperationFailed => "platform_operation_failed",
            Self::PermissionDenied => "permission_denied",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::FfiInvalidArgument => "ffi_invalid_argument",
            Self::FfiIncompatibleVersion => "ffi_incompatible_version",
            Self::FfiPanicContained => "ffi_panic_contained",
            Self::FfiCallbackFailed => "ffi_callback_failed",
            Self::QueueOverflow => "queue_overflow",
            Self::ShutdownInProgress => "shutdown_in_progress",
            Self::ShutdownFailed => "shutdown_failed",
            Self::WorkerStopped => "worker_stopped",
        }
    }
}

impl fmt::Display for CoreErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

/// One bounded, non-secret diagnostic context field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorContextEntry {
    pub key: String,
    pub value: String,
}

impl ErrorContextEntry {
    /// Creates a validated context entry.
    ///
    /// # Errors
    ///
    /// Returns [`CoreErrorBuildError`] when the key or value is blank, exceeds
    /// its byte bound, or contains disallowed whitespace/control characters.
    pub fn new(
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, CoreErrorBuildError> {
        let key = key.into();
        let value = value.into();
        validate_context_key(&key)?;
        validate_context_value(&value)?;
        Ok(Self { key, value })
    }
}

/// Stable transfer record for future `UniFFI` and persistence conversions.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreErrorRecord {
    pub code: CoreErrorCode,
    pub message: String,
    pub subsystem: CoreSubsystem,
    pub severity: ErrorSeverity,
    pub retryable: bool,
    pub operation_id: Option<OperationId>,
    pub context: Vec<ErrorContextEntry>,
}

/// Validated structured failure surfaced by the Rust core.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub code: CoreErrorCode,
    pub message: String,
    pub subsystem: CoreSubsystem,
    pub severity: ErrorSeverity,
    pub retryable: bool,
    pub operation_id: Option<OperationId>,
    pub context: Vec<ErrorContextEntry>,
}

impl CoreError {
    /// Creates a validated structured error with no context fields.
    ///
    /// # Errors
    ///
    /// Returns [`CoreErrorBuildError`] when the message is invalid.
    pub fn new(
        code: CoreErrorCode,
        message: impl Into<String>,
        severity: ErrorSeverity,
        retryable: bool,
        operation_id: Option<OperationId>,
    ) -> Result<Self, CoreErrorBuildError> {
        Self::try_from(CoreErrorRecord {
            code,
            message: message.into(),
            subsystem: code.subsystem(),
            severity,
            retryable,
            operation_id,
            context: Vec::new(),
        })
    }

    /// Appends one validated context field.
    ///
    /// # Errors
    ///
    /// Returns [`CoreErrorBuildError::ContextLimitExceeded`] after the bounded
    /// context capacity is reached.
    pub fn with_context(mut self, entry: ErrorContextEntry) -> Result<Self, CoreErrorBuildError> {
        if self.context.len() >= MAX_ERROR_CONTEXT_ENTRIES {
            return Err(CoreErrorBuildError::ContextLimitExceeded {
                maximum_entries: MAX_ERROR_CONTEXT_ENTRIES,
            });
        }
        self.context.push(entry);
        Ok(self)
    }

    /// Converts identifier validation into a structured validation failure.
    /// The rejected identifier value is never copied into the error.
    #[must_use]
    pub fn from_identifier_validation(
        error: &IdValidationError,
        operation_id: Option<OperationId>,
    ) -> Self {
        let reason = match error.reason() {
            IdValidationReason::Empty => "empty",
            IdValidationReason::TooLong { .. } => "too_long",
            IdValidationReason::SurroundingWhitespace => "surrounding_whitespace",
            IdValidationReason::ControlCharacter { .. } => "control_character",
        };
        Self {
            code: CoreErrorCode::InvalidIdentifier,
            message: "invalid domain identifier".into(),
            subsystem: CoreSubsystem::Validation,
            severity: ErrorSeverity::Error,
            retryable: false,
            operation_id,
            context: vec![
                ErrorContextEntry {
                    key: "identifier_kind".into(),
                    value: error.identifier_kind().stable_name().into(),
                },
                ErrorContextEntry {
                    key: "reason".into(),
                    value: reason.into(),
                },
            ],
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}: {}",
            self.subsystem, self.code, self.message
        )
    }
}

impl Error for CoreError {}

impl From<&CoreError> for CoreErrorRecord {
    fn from(error: &CoreError) -> Self {
        Self {
            code: error.code,
            message: error.message.clone(),
            subsystem: error.subsystem,
            severity: error.severity,
            retryable: error.retryable,
            operation_id: error.operation_id.clone(),
            context: error.context.clone(),
        }
    }
}

impl TryFrom<CoreErrorRecord> for CoreError {
    type Error = CoreErrorBuildError;

    fn try_from(record: CoreErrorRecord) -> Result<Self, Self::Error> {
        validate_error_message(&record.message)?;
        if record.subsystem != record.code.subsystem() {
            return Err(CoreErrorBuildError::SubsystemMismatch {
                code: record.code,
                expected: record.code.subsystem(),
                actual: record.subsystem,
            });
        }
        if record.context.len() > MAX_ERROR_CONTEXT_ENTRIES {
            return Err(CoreErrorBuildError::ContextLimitExceeded {
                maximum_entries: MAX_ERROR_CONTEXT_ENTRIES,
            });
        }
        for entry in &record.context {
            validate_context_key(&entry.key)?;
            validate_context_value(&entry.value)?;
        }
        Ok(Self {
            code: record.code,
            message: record.message,
            subsystem: record.subsystem,
            severity: record.severity,
            retryable: record.retryable,
            operation_id: record.operation_id,
            context: record.context,
        })
    }
}

/// Failure while constructing or converting a structured core error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreErrorBuildError {
    EmptyMessage,
    MessageTooLong {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    MessageContainsControlCharacter,
    EmptyContextKey,
    ContextKeyTooLong {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    ContextKeyHasSurroundingWhitespace,
    ContextKeyContainsControlCharacter,
    EmptyContextValue,
    ContextValueTooLong {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    ContextValueContainsControlCharacter,
    ContextLimitExceeded {
        maximum_entries: usize,
    },
    SubsystemMismatch {
        code: CoreErrorCode,
        expected: CoreSubsystem,
        actual: CoreSubsystem,
    },
}

impl fmt::Display for CoreErrorBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMessage => formatter.write_str("core error message is blank"),
            Self::MessageTooLong {
                actual_bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "core error message is {actual_bytes} bytes; maximum is {maximum_bytes}"
            ),
            Self::MessageContainsControlCharacter => {
                formatter.write_str("core error message contains a control character")
            }
            Self::EmptyContextKey => formatter.write_str("error context key is blank"),
            Self::ContextKeyTooLong {
                actual_bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "error context key is {actual_bytes} bytes; maximum is {maximum_bytes}"
            ),
            Self::ContextKeyHasSurroundingWhitespace => {
                formatter.write_str("error context key has surrounding whitespace")
            }
            Self::ContextKeyContainsControlCharacter => {
                formatter.write_str("error context key contains a control character")
            }
            Self::EmptyContextValue => formatter.write_str("error context value is blank"),
            Self::ContextValueTooLong {
                actual_bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "error context value is {actual_bytes} bytes; maximum is {maximum_bytes}"
            ),
            Self::ContextValueContainsControlCharacter => {
                formatter.write_str("error context value contains a control character")
            }
            Self::ContextLimitExceeded { maximum_entries } => write!(
                formatter,
                "error context exceeds the maximum of {maximum_entries} entries"
            ),
            Self::SubsystemMismatch {
                code,
                expected,
                actual,
            } => write!(
                formatter,
                "error code {code} belongs to {expected}, not {actual}"
            ),
        }
    }
}

impl Error for CoreErrorBuildError {}

fn validate_error_message(message: &str) -> Result<(), CoreErrorBuildError> {
    if message.trim().is_empty() {
        return Err(CoreErrorBuildError::EmptyMessage);
    }
    if message.len() > MAX_ERROR_MESSAGE_BYTES {
        return Err(CoreErrorBuildError::MessageTooLong {
            actual_bytes: message.len(),
            maximum_bytes: MAX_ERROR_MESSAGE_BYTES,
        });
    }
    if message.chars().any(char::is_control) {
        return Err(CoreErrorBuildError::MessageContainsControlCharacter);
    }
    Ok(())
}

fn validate_context_key(key: &str) -> Result<(), CoreErrorBuildError> {
    if key.trim().is_empty() {
        return Err(CoreErrorBuildError::EmptyContextKey);
    }
    if key.len() > MAX_ERROR_CONTEXT_KEY_BYTES {
        return Err(CoreErrorBuildError::ContextKeyTooLong {
            actual_bytes: key.len(),
            maximum_bytes: MAX_ERROR_CONTEXT_KEY_BYTES,
        });
    }
    if key.trim() != key {
        return Err(CoreErrorBuildError::ContextKeyHasSurroundingWhitespace);
    }
    if key.chars().any(char::is_control) {
        return Err(CoreErrorBuildError::ContextKeyContainsControlCharacter);
    }
    Ok(())
}

fn validate_context_value(value: &str) -> Result<(), CoreErrorBuildError> {
    if value.trim().is_empty() {
        return Err(CoreErrorBuildError::EmptyContextValue);
    }
    if value.len() > MAX_ERROR_CONTEXT_VALUE_BYTES {
        return Err(CoreErrorBuildError::ContextValueTooLong {
            actual_bytes: value.len(),
            maximum_bytes: MAX_ERROR_CONTEXT_VALUE_BYTES,
        });
    }
    if value.chars().any(char::is_control) {
        return Err(CoreErrorBuildError::ContextValueContainsControlCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CoreError, CoreErrorBuildError, CoreErrorCode, CoreErrorRecord, CoreSubsystem,
        ErrorContextEntry, ErrorSeverity, MAX_ERROR_CONTEXT_ENTRIES, MAX_ERROR_CONTEXT_KEY_BYTES,
        MAX_ERROR_CONTEXT_VALUE_BYTES, MAX_ERROR_MESSAGE_BYTES,
    };
    use crate::domain::{OperationId, SessionId};

    #[test]
    fn stable_codes_cover_every_required_subsystem_without_unknown() {
        let representatives = [
            (
                CoreErrorCode::InvalidArgument,
                CoreSubsystem::Validation,
                1001,
            ),
            (
                CoreErrorCode::MalformedProtocolFrame,
                CoreSubsystem::Protocol,
                2001,
            ),
            (
                CoreErrorCode::TransportConnectionFailed,
                CoreSubsystem::Transport,
                3001,
            ),
            (
                CoreErrorCode::SynchronizationTimeout,
                CoreSubsystem::Synchronization,
                4001,
            ),
            (CoreErrorCode::AudioOutputFailed, CoreSubsystem::Audio, 5002),
            (
                CoreErrorCode::StorageMigrationFailed,
                CoreSubsystem::Storage,
                6001,
            ),
            (
                CoreErrorCode::PlatformOperationFailed,
                CoreSubsystem::Platform,
                7000,
            ),
            (CoreErrorCode::FfiPanicContained, CoreSubsystem::Ffi, 8002),
            (CoreErrorCode::QueueOverflow, CoreSubsystem::Runtime, 9000),
            (CoreErrorCode::ShutdownFailed, CoreSubsystem::Runtime, 9002),
        ];

        for (code, subsystem, numeric) in representatives {
            assert_eq!(code.subsystem(), subsystem);
            assert_eq!(code.numeric_code(), numeric);
            assert_ne!(code.stable_name(), "unknown");
        }
    }

    #[test]
    fn validates_messages_context_and_subsystem_consistency() {
        assert_eq!(
            CoreError::new(
                CoreErrorCode::InvalidArgument,
                " ",
                ErrorSeverity::Error,
                false,
                None,
            ),
            Err(CoreErrorBuildError::EmptyMessage)
        );

        let oversized_message = "m".repeat(MAX_ERROR_MESSAGE_BYTES + 1);
        assert!(matches!(
            CoreError::new(
                CoreErrorCode::InvalidArgument,
                oversized_message,
                ErrorSeverity::Error,
                false,
                None,
            ),
            Err(CoreErrorBuildError::MessageTooLong { .. })
        ));

        assert!(matches!(
            ErrorContextEntry::new("k".repeat(MAX_ERROR_CONTEXT_KEY_BYTES + 1), "value"),
            Err(CoreErrorBuildError::ContextKeyTooLong { .. })
        ));
        assert!(matches!(
            ErrorContextEntry::new("key", "v".repeat(MAX_ERROR_CONTEXT_VALUE_BYTES + 1)),
            Err(CoreErrorBuildError::ContextValueTooLong { .. })
        ));

        let record = CoreErrorRecord {
            code: CoreErrorCode::AudioOutputFailed,
            message: "audio output failed".into(),
            subsystem: CoreSubsystem::Transport,
            severity: ErrorSeverity::Fatal,
            retryable: false,
            operation_id: None,
            context: Vec::new(),
        };
        assert!(matches!(
            CoreError::try_from(record),
            Err(CoreErrorBuildError::SubsystemMismatch { .. })
        ));
    }

    #[test]
    fn context_count_is_bounded() {
        let mut error = CoreError::new(
            CoreErrorCode::TransportDeliveryFailed,
            "delivery failed",
            ErrorSeverity::Error,
            true,
            None,
        )
        .expect("valid initial error");

        for index in 0..MAX_ERROR_CONTEXT_ENTRIES {
            error = error
                .with_context(
                    ErrorContextEntry::new(format!("key_{index}"), format!("value_{index}"))
                        .expect("bounded context entry"),
                )
                .expect("context capacity not reached");
        }

        let overflow = error.with_context(
            ErrorContextEntry::new("overflow", "value").expect("valid overflow entry"),
        );
        assert_eq!(
            overflow,
            Err(CoreErrorBuildError::ContextLimitExceeded {
                maximum_entries: MAX_ERROR_CONTEXT_ENTRIES,
            })
        );
    }

    #[test]
    fn record_conversion_preserves_operation_and_subsystem() {
        let operation_id = OperationId::new("operation-42").expect("valid operation identifier");
        let error = CoreError::new(
            CoreErrorCode::StorageWriteFailed,
            "failed to persist settings",
            ErrorSeverity::Error,
            true,
            Some(operation_id.clone()),
        )
        .expect("valid storage error")
        .with_context(ErrorContextEntry::new("repository", "settings").expect("valid context"))
        .expect("context fits");

        let record = CoreErrorRecord::from(&error);
        let restored = CoreError::try_from(record).expect("record must remain valid");
        assert_eq!(restored.operation_id, Some(operation_id));
        assert_eq!(restored.subsystem, CoreSubsystem::Storage);
        assert_eq!(restored, error);
    }

    #[test]
    fn identifier_conversion_does_not_leak_rejected_value() {
        let identifier_error =
            SessionId::new(" secret-session ").expect_err("surrounding whitespace must fail");
        let operation_id =
            OperationId::new("validate-session").expect("valid operation identifier");
        let error =
            CoreError::from_identifier_validation(&identifier_error, Some(operation_id.clone()));

        assert_eq!(error.code, CoreErrorCode::InvalidIdentifier);
        assert_eq!(error.subsystem, CoreSubsystem::Validation);
        assert_eq!(error.operation_id, Some(operation_id));
        assert!(!error.to_string().contains("secret-session"));
        assert!(
            error
                .context
                .iter()
                .all(|entry| !entry.value.contains("secret-session"))
        );
    }
}
