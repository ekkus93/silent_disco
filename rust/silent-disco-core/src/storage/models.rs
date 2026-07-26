use core::{fmt, str::FromStr};
use std::error::Error;

use crate::domain::{AppRole, DeviceId, DiagnosticRunId, SessionId, TrustState, TuningSettings};

pub const MAX_DISPLAY_NAME_BYTES: usize = 256;
pub const MAX_PRIVATE_KEY_REFERENCE_BYTES: usize = 512;
pub const MAX_PUBLIC_KEY_BYTES: usize = 4_096;
pub const MAX_SESSION_NAME_BYTES: usize = 256;
pub const MAX_FAILURE_CODE_BYTES: usize = 128;
pub const MAX_FAILURE_MESSAGE_BYTES: usize = 512;
pub const MAX_DIAGNOSTIC_SUMMARY_BYTES: usize = 262_144;
pub const MAX_DIAGNOSTIC_QUERY_LIMIT: u32 = 32;
pub const MAX_DIAGNOSTIC_EXPORT_LIMIT: u32 = 32;

/// Persisted tuning values and their wall-clock update timestamp.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSettings {
    pub tuning: TuningSettings,
    pub updated_at_ms: u64,
}

impl StoredSettings {
    /// Validates tuning values and the persisted timestamp.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for unsupported values.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        self.tuning
            .validate()
            .map_err(|_| StorageModelValidationError::TuningSettings)?;
        validate_sql_millis(self.updated_at_ms, StorageModelValidationError::Timestamp)
    }
}

/// Persisted trusted-device metadata. Private key bytes are never stored here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedDevice {
    pub device_id: DeviceId,
    pub display_name: String,
    pub public_key: Option<Vec<u8>>,
    pub private_key_ref: Option<String>,
    pub trust_state: TrustState,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub updated_at_ms: u64,
}

impl TrustedDevice {
    /// Validates bounded text, key material, and timestamp ordering.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for unsupported values.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_human_name(&self.display_name, MAX_DISPLAY_NAME_BYTES)
            .map_err(|()| StorageModelValidationError::DisplayName)?;
        if let Some(public_key) = &self.public_key
            && (public_key.is_empty() || public_key.len() > MAX_PUBLIC_KEY_BYTES)
        {
            return Err(StorageModelValidationError::PublicKey);
        }
        if let Some(reference) = &self.private_key_ref {
            validate_reference(reference, MAX_PRIVATE_KEY_REFERENCE_BYTES)
                .map_err(|()| StorageModelValidationError::PrivateKeyReference)?;
        }
        validate_sql_millis(self.first_seen_ms, StorageModelValidationError::Timestamp)?;
        validate_sql_millis(self.last_seen_ms, StorageModelValidationError::Timestamp)?;
        validate_sql_millis(self.updated_at_ms, StorageModelValidationError::Timestamp)?;
        if self.first_seen_ms > self.last_seen_ms || self.last_seen_ms > self.updated_at_ms {
            return Err(StorageModelValidationError::TimestampOrder);
        }
        Ok(())
    }
}

/// Stable persisted session outcome.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionOutcome {
    Active = 1,
    Completed = 2,
    Cancelled = 3,
    Failed = 4,
}

impl SessionOutcome {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// Decodes the stable database representation.
    ///
    /// # Errors
    ///
    /// Returns a model-validation failure for unknown values.
    pub fn from_wire_name(value: &str) -> Result<Self, StorageModelValidationError> {
        match value {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            _ => Err(StorageModelValidationError::SessionOutcome),
        }
    }
}

impl fmt::Display for SessionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.wire_name())
    }
}

impl FromStr for SessionOutcome {
    type Err = StorageModelValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_wire_name(value)
    }
}

/// Data required to begin one session-history record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStart {
    pub session_id: SessionId,
    pub role: AppRole,
    pub session_name: String,
    pub started_at_ms: u64,
}

impl SessionStart {
    /// Validates session name and timestamp.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for unsupported values.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_human_name(&self.session_name, MAX_SESSION_NAME_BYTES)
            .map_err(|()| StorageModelValidationError::SessionName)?;
        validate_sql_millis(self.started_at_ms, StorageModelValidationError::Timestamp)
    }
}

/// Mutable in-progress session counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUpdate {
    pub session_id: SessionId,
    pub listener_count: u32,
}

/// Data required to finish one active session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEnd {
    pub session_id: SessionId,
    pub ended_at_ms: u64,
    pub listener_count: u32,
    pub outcome: SessionOutcome,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
}

impl SessionEnd {
    /// Validates terminal outcome, timestamp, and optional failure details.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for unsupported values.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_sql_millis(self.ended_at_ms, StorageModelValidationError::Timestamp)?;
        if self.outcome == SessionOutcome::Active {
            return Err(StorageModelValidationError::SessionOutcome);
        }
        match self.outcome {
            SessionOutcome::Failed => {
                let code = self
                    .failure_code
                    .as_deref()
                    .ok_or(StorageModelValidationError::FailureCode)?;
                validate_reference(code, MAX_FAILURE_CODE_BYTES)
                    .map_err(|()| StorageModelValidationError::FailureCode)?;
                if let Some(message) = &self.failure_message {
                    validate_bounded_text(message, MAX_FAILURE_MESSAGE_BYTES)
                        .map_err(|()| StorageModelValidationError::FailureMessage)?;
                }
            }
            SessionOutcome::Completed | SessionOutcome::Cancelled => {
                if self.failure_code.is_some() || self.failure_message.is_some() {
                    return Err(StorageModelValidationError::UnexpectedFailureDetails);
                }
            }
            SessionOutcome::Active => {
                return Err(StorageModelValidationError::SessionOutcome);
            }
        }
        Ok(())
    }
}

/// Complete persisted session-history row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistory {
    pub session_id: SessionId,
    pub role: AppRole,
    pub session_name: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub listener_count: u32,
    pub outcome: SessionOutcome,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
}

/// One summarized diagnostic run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRunSummary {
    pub run_id: DiagnosticRunId,
    pub session_id: Option<SessionId>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub summary_json: String,
}

impl DiagnosticRunSummary {
    /// Validates timestamps and the bounded JSON payload shape.
    ///
    /// JSON syntax is additionally enforced by the `SQLite` schema.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for unsupported values.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_sql_millis(self.started_at_ms, StorageModelValidationError::Timestamp)?;
        if let Some(ended_at_ms) = self.ended_at_ms {
            validate_sql_millis(ended_at_ms, StorageModelValidationError::Timestamp)?;
            if ended_at_ms < self.started_at_ms {
                return Err(StorageModelValidationError::TimestampOrder);
            }
        }
        if self.summary_json.is_empty()
            || self.summary_json.len() > MAX_DIAGNOSTIC_SUMMARY_BYTES
            || self.summary_json.contains('\0')
        {
            return Err(StorageModelValidationError::DiagnosticSummary);
        }
        Ok(())
    }
}

/// Bounded diagnostic-run query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticQuery {
    pub session_id: Option<SessionId>,
    pub limit: u32,
}

impl DiagnosticQuery {
    /// Validates the query limit.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for a zero or excessive limit.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        if self.limit == 0 || self.limit > MAX_DIAGNOSTIC_QUERY_LIMIT {
            return Err(StorageModelValidationError::DiagnosticQueryLimit);
        }
        Ok(())
    }
}

/// Stable exclusive cursor for deterministic diagnostic export pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExportCursor {
    pub started_at_ms: u64,
    pub run_id: DiagnosticRunId,
}

impl DiagnosticExportCursor {
    /// Validates the cursor timestamp.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure when the timestamp cannot be stored.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        validate_sql_millis(self.started_at_ms, StorageModelValidationError::Timestamp)
    }
}

/// Bounded request for one deterministic diagnostic export page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExportRequest {
    pub session_id: Option<SessionId>,
    pub cursor: Option<DiagnosticExportCursor>,
    pub limit: u32,
}

impl DiagnosticExportRequest {
    /// Validates the page limit and optional cursor.
    ///
    /// # Errors
    ///
    /// Returns a stable model-validation failure for an invalid page request.
    pub fn validate(&self) -> Result<(), StorageModelValidationError> {
        if self.limit == 0 || self.limit > MAX_DIAGNOSTIC_EXPORT_LIMIT {
            return Err(StorageModelValidationError::DiagnosticExportLimit);
        }
        if let Some(cursor) = &self.cursor {
            cursor.validate()?;
        }
        Ok(())
    }
}

/// One bounded deterministic diagnostic export page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticExport {
    pub schema_version: u32,
    pub runs: Vec<DiagnosticRunSummary>,
    pub next_cursor: Option<DiagnosticExportCursor>,
}

/// Stable validation failures for persisted domain models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageModelValidationError {
    TuningSettings,
    DisplayName,
    PublicKey,
    PrivateKeyReference,
    SessionName,
    SessionOutcome,
    FailureCode,
    FailureMessage,
    UnexpectedFailureDetails,
    Timestamp,
    TimestampOrder,
    DiagnosticSummary,
    DiagnosticQueryLimit,
    DiagnosticExportLimit,
}

impl fmt::Display for StorageModelValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TuningSettings => "tuning settings are invalid",
            Self::DisplayName => "device display name is invalid",
            Self::PublicKey => "public key is empty or exceeds the supported size",
            Self::PrivateKeyReference => "private key reference is invalid",
            Self::SessionName => "session name is invalid",
            Self::SessionOutcome => "session outcome is invalid for this operation",
            Self::FailureCode => "failed sessions require a valid failure code",
            Self::FailureMessage => "failure message exceeds the supported size",
            Self::UnexpectedFailureDetails => {
                "non-failed sessions must not contain failure details"
            }
            Self::Timestamp => "timestamp exceeds the supported SQLite integer range",
            Self::TimestampOrder => "timestamps are not in chronological order",
            Self::DiagnosticSummary => "diagnostic summary is empty or exceeds the supported size",
            Self::DiagnosticQueryLimit => "diagnostic query limit is outside the supported range",
            Self::DiagnosticExportLimit => {
                "diagnostic export page limit is outside the supported range"
            }
        })
    }
}

impl Error for StorageModelValidationError {}

pub(crate) fn validate_sql_millis(
    value: u64,
    error: StorageModelValidationError,
) -> Result<(), StorageModelValidationError> {
    i64::try_from(value).map(|_| ()).map_err(|_| error)
}

fn validate_human_name(value: &str, maximum_bytes: usize) -> Result<(), ()> {
    if value.trim().is_empty() || value.len() > maximum_bytes || value.chars().any(char::is_control)
    {
        return Err(());
    }
    Ok(())
}

fn validate_reference(value: &str, maximum_bytes: usize) -> Result<(), ()> {
    if value.trim().is_empty()
        || value.trim() != value
        || value.len() > maximum_bytes
        || value.chars().any(char::is_control)
    {
        return Err(());
    }
    Ok(())
}

fn validate_bounded_text(value: &str, maximum_bytes: usize) -> Result<(), ()> {
    if value.len() > maximum_bytes || value.contains('\0') {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosticQuery, DiagnosticRunSummary, SessionEnd, SessionOutcome,
        StorageModelValidationError, StoredSettings, TrustedDevice,
    };
    use crate::domain::{DeviceId, DiagnosticRunId, TrustState, TuningSettings};

    #[test]
    fn preserves_unicode_names_and_binary_public_keys() {
        let device = TrustedDevice {
            device_id: DeviceId::new("listener-1").expect("valid identifier"),
            display_name: "Zoë 🎧 東京".into(),
            public_key: Some(vec![0, 1, 2, 0xff]),
            private_key_ref: Some("keystore:listener-1".into()),
            trust_state: TrustState::Trusted,
            first_seen_ms: 10,
            last_seen_ms: 20,
            updated_at_ms: 30,
        };
        assert_eq!(device.validate(), Ok(()));
    }

    #[test]
    fn rejects_invalid_terminal_session_details() {
        let active_end = SessionEnd {
            session_id: crate::domain::SessionId::new("session-1")
                .expect("valid session identifier"),
            ended_at_ms: 20,
            listener_count: 1,
            outcome: SessionOutcome::Active,
            failure_code: None,
            failure_message: None,
        };
        assert_eq!(
            active_end.validate(),
            Err(StorageModelValidationError::SessionOutcome)
        );
    }

    #[test]
    fn validates_settings_and_diagnostic_bounds() {
        let settings = StoredSettings {
            tuning: TuningSettings::default(),
            updated_at_ms: 1,
        };
        assert_eq!(settings.validate(), Ok(()));

        let query = DiagnosticQuery {
            session_id: None,
            limit: 0,
        };
        assert_eq!(
            query.validate(),
            Err(StorageModelValidationError::DiagnosticQueryLimit)
        );

        let diagnostic = DiagnosticRunSummary {
            run_id: DiagnosticRunId::new("run-1").expect("valid run identifier"),
            session_id: None,
            started_at_ms: 10,
            ended_at_ms: Some(9),
            summary_json: "{}".into(),
        };
        assert_eq!(
            diagnostic.validate(),
            Err(StorageModelValidationError::TimestampOrder)
        );
    }
}
