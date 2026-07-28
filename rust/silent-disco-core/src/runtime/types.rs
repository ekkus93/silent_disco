use crate::domain::{
    ApprovalMode, DeliverySeverity, DeviceId, MonotonicMillis, RequestId, SessionId, SyncConfidence,
    TransportState, TrustState, TuningSettings,
};
use crate::error::CoreError;
use crate::protocol::{MAX_DISPLAY_NAME_BYTES, MAX_INVITE_CODE_BYTES, MAX_SESSION_NAME_BYTES};
use core::fmt;
use std::error::Error;
use std::net::IpAddr;

pub const MAX_AUDIO_SOURCE_ID_BYTES: usize = 128;
pub const MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES: usize = 256;
pub const MAX_DIAGNOSTIC_EVENT_NAME_BYTES: usize = 96;
pub const MAX_DIAGNOSTIC_FIELDS: usize = 16;
pub const MAX_DIAGNOSTIC_FIELD_KEY_BYTES: usize = 64;
pub const MAX_DIAGNOSTIC_FIELD_VALUE_BYTES: usize = 256;

/// Monotonically increasing revision of an immutable [`super::CoreSnapshot`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SnapshotRevision(u64);

impl SnapshotRevision {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next revision without wrapping.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeRecordValidationError::RevisionOverflow`] at `u64::MAX`.
    pub const fn checked_next(self) -> Result<Self, RuntimeRecordValidationError> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(RuntimeRecordValidationError::RevisionOverflow),
        }
    }
}

impl fmt::Display for SnapshotRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque app-owned audio source reference. Native paths never enter this record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSourceDescriptor {
    pub source_id: String,
    pub display_name: String,
    pub byte_length: Option<u64>,
    pub duration_ms: Option<u64>,
}

impl AudioSourceDescriptor {
    /// Creates a bounded descriptor for one staged source.
    ///
    /// # Errors
    ///
    /// Rejects blank, oversized, whitespace-surrounded, or control-containing
    /// identifiers and display names, as well as an explicitly empty source.
    pub fn new(
        source_id: impl Into<String>,
        display_name: impl Into<String>,
        byte_length: Option<u64>,
        duration_ms: Option<u64>,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let source_id = source_id.into();
        let display_name = display_name.into();
        validate_token(
            &source_id,
            MAX_AUDIO_SOURCE_ID_BYTES,
            RuntimeRecordValidationError::AudioSourceId,
        )?;
        validate_human_text(
            &display_name,
            MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES,
            RuntimeRecordValidationError::AudioSourceDisplayName,
        )?;
        if byte_length == Some(0) {
            return Err(RuntimeRecordValidationError::AudioSourceLength);
        }
        Ok(Self {
            source_id,
            display_name,
            byte_length,
            duration_ms,
        })
    }
}

/// Explicit patch semantics for the optional host invite code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InviteCodePatch {
    #[default]
    Unchanged,
    Set(String),
    Clear,
}

/// Explicit patch semantics for the optional staged audio source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AudioSourcePatch {
    #[default]
    Unchanged,
    Set(AudioSourceDescriptor),
    Clear,
}

/// Presentation intent for changing the host draft. It contains no native paths.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostDraftPatch {
    pub session_name: Option<String>,
    pub approval_mode: Option<ApprovalMode>,
    pub invite_code: InviteCodePatch,
    pub audio_source: AudioSourcePatch,
    pub remember_approved_devices: Option<bool>,
}

/// Authoritative host-session draft owned by the Rust actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDraft {
    pub session_name: String,
    pub approval_mode: ApprovalMode,
    pub invite_code: Option<String>,
    pub audio_source: Option<AudioSourceDescriptor>,
    pub remember_approved_devices: bool,
}

impl Default for HostDraft {
    fn default() -> Self {
        Self {
            session_name: String::new(),
            approval_mode: ApprovalMode::Manual,
            invite_code: None,
            audio_source: None,
            remember_approved_devices: false,
        }
    }
}

impl HostDraft {
    /// Applies a patch and validates every modified field before returning.
    ///
    /// # Errors
    ///
    /// The original draft remains unchanged when validation fails.
    pub fn patched(&self, patch: &HostDraftPatch) -> Result<Self, RuntimeRecordValidationError> {
        let mut next = self.clone();
        if let Some(session_name) = &patch.session_name {
            validate_human_text(
                session_name,
                MAX_SESSION_NAME_BYTES,
                RuntimeRecordValidationError::SessionName,
            )?;
            next.session_name.clone_from(session_name);
        }
        if let Some(approval_mode) = patch.approval_mode {
            next.approval_mode = approval_mode;
        }
        match &patch.invite_code {
            InviteCodePatch::Unchanged => {}
            InviteCodePatch::Set(code) => {
                validate_token(
                    code,
                    MAX_INVITE_CODE_BYTES,
                    RuntimeRecordValidationError::InviteCode,
                )?;
                next.invite_code = Some(code.clone());
            }
            InviteCodePatch::Clear => next.invite_code = None,
        }
        match &patch.audio_source {
            AudioSourcePatch::Unchanged => {}
            AudioSourcePatch::Set(source) => next.audio_source = Some(source.clone()),
            AudioSourcePatch::Clear => next.audio_source = None,
        }
        if let Some(remember) = patch.remember_approved_devices {
            next.remember_approved_devices = remember;
        }
        next.validate_consistency()?;
        Ok(next)
    }

    /// Validates fields required before a host session may be created.
    ///
    /// # Errors
    ///
    /// Rejects missing session name/source or approval-mode inconsistencies.
    pub fn validate_for_creation(&self) -> Result<(), RuntimeRecordValidationError> {
        validate_human_text(
            &self.session_name,
            MAX_SESSION_NAME_BYTES,
            RuntimeRecordValidationError::SessionName,
        )?;
        if self.audio_source.is_none() {
            return Err(RuntimeRecordValidationError::AudioSourceRequired);
        }
        self.validate_consistency()
    }

    fn validate_consistency(&self) -> Result<(), RuntimeRecordValidationError> {
        match (self.approval_mode, self.invite_code.as_deref()) {
            (ApprovalMode::InviteCode, Some(code)) => validate_token(
                code,
                MAX_INVITE_CODE_BYTES,
                RuntimeRecordValidationError::InviteCode,
            ),
            (ApprovalMode::InviteCode, None) => Err(RuntimeRecordValidationError::InviteCodeRequired),
            (ApprovalMode::Manual | ApprovalMode::TrustedDevices, Some(_)) => {
                Err(RuntimeRecordValidationError::UnexpectedInviteCode)
            }
            (ApprovalMode::Manual | ApprovalMode::TrustedDevices, None) => Ok(()),
        }
    }
}

/// Partial update for the validated shared tuning settings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TuningPatch {
    pub sync_sample_window: Option<u16>,
    pub sync_cadence_ms: Option<u64>,
    pub startup_buffer_ms: Option<u64>,
    pub late_packet_threshold_ms: Option<u64>,
    pub hard_resync_threshold_ms: Option<u64>,
    pub sync_drift_threshold_ms: Option<f64>,
    pub scan_window_ms: Option<u64>,
}

impl TuningPatch {
    /// Applies this patch and validates the complete resulting settings record.
    ///
    /// # Errors
    ///
    /// Returns a tuning validation failure without mutating the input settings.
    pub fn apply_to(
        &self,
        current: &TuningSettings,
    ) -> Result<TuningSettings, RuntimeRecordValidationError> {
        let next = TuningSettings {
            sync_sample_window: self.sync_sample_window.unwrap_or(current.sync_sample_window),
            sync_cadence_ms: self.sync_cadence_ms.unwrap_or(current.sync_cadence_ms),
            startup_buffer_ms: self.startup_buffer_ms.unwrap_or(current.startup_buffer_ms),
            late_packet_threshold_ms: self
                .late_packet_threshold_ms
                .unwrap_or(current.late_packet_threshold_ms),
            hard_resync_threshold_ms: self
                .hard_resync_threshold_ms
                .unwrap_or(current.hard_resync_threshold_ms),
            sync_drift_threshold_ms: self
                .sync_drift_threshold_ms
                .unwrap_or(current.sync_drift_threshold_ms),
            scan_window_ms: self.scan_window_ms.unwrap_or(current.scan_window_ms),
        };
        next.validate()
            .map_err(|_| RuntimeRecordValidationError::TuningSettings)?;
        Ok(next)
    }
}

/// Validated standard-IP endpoint produced by a platform or transport adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkEndpoint {
    pub address: IpAddr,
    pub control_port: u16,
    pub sync_port: u16,
    pub audio_port: u16,
}

impl NetworkEndpoint {
    /// Creates an endpoint whose three service ports are all nonzero.
    ///
    /// # Errors
    ///
    /// Returns an invalid-port failure when any service port is zero.
    pub const fn new(
        address: IpAddr,
        control_port: u16,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<Self, RuntimeRecordValidationError> {
        if control_port == 0 || sync_port == 0 || audio_port == 0 {
            return Err(RuntimeRecordValidationError::NetworkPort);
        }
        Ok(Self {
            address,
            control_port,
            sync_port,
            audio_port,
        })
    }
}

/// Cross-platform semantic capability state. Platforms map this to native APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapabilitySnapshot {
    pub nearby_discovery_available: bool,
    pub nearby_advertising_available: bool,
    pub local_network_available: bool,
    pub audio_source_selection_available: bool,
    pub audio_output_available: bool,
    pub secure_store_available: bool,
}

/// Discovery advertisement containing only bounded semantic information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAdvertisement {
    pub session_id: SessionId,
    pub host_device_id: DeviceId,
    pub session_name: String,
    pub approval_mode: ApprovalMode,
    pub protocol_version: u16,
    pub endpoint: Option<NetworkEndpoint>,
}

impl SessionAdvertisement {
    /// Creates a validated session advertisement.
    ///
    /// # Errors
    ///
    /// Rejects an invalid session name or zero protocol version.
    pub fn new(
        session_id: SessionId,
        host_device_id: DeviceId,
        session_name: impl Into<String>,
        approval_mode: ApprovalMode,
        protocol_version: u16,
        endpoint: Option<NetworkEndpoint>,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let session_name = session_name.into();
        validate_human_text(
            &session_name,
            MAX_SESSION_NAME_BYTES,
            RuntimeRecordValidationError::SessionName,
        )?;
        if protocol_version == 0 {
            return Err(RuntimeRecordValidationError::ProtocolVersion);
        }
        Ok(Self {
            session_id,
            host_device_id,
            session_name,
            approval_mode,
            protocol_version,
            endpoint,
        })
    }
}

/// One pending listener admission request presented by the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequestSummary {
    pub request_id: RequestId,
    pub device_id: DeviceId,
    pub display_name: String,
    pub trust_state: TrustState,
    pub invite_code_valid: bool,
    pub received_at: MonotonicMillis,
}

impl JoinRequestSummary {
    /// Creates a bounded join-request summary.
    ///
    /// # Errors
    ///
    /// Rejects a blank or oversized display name.
    pub fn new(
        request_id: RequestId,
        device_id: DeviceId,
        display_name: impl Into<String>,
        trust_state: TrustState,
        invite_code_valid: bool,
        received_at: MonotonicMillis,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let display_name = display_name.into();
        validate_human_text(
            &display_name,
            MAX_DISPLAY_NAME_BYTES,
            RuntimeRecordValidationError::DeviceDisplayName,
        )?;
        Ok(Self {
            request_id,
            device_id,
            display_name,
            trust_state,
            invite_code_valid,
            received_at,
        })
    }
}

/// Bounded delivery accounting for one control or packet broadcast operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryReport {
    pub intended_peers: u32,
    pub successful_peers: u32,
    pub failed_peers: u32,
    pub severity: DeliverySeverity,
}

impl DeliveryReport {
    /// Validates counts and derives the stable delivery severity.
    ///
    /// # Errors
    ///
    /// Rejects count overflow or a successful/failed total that does not equal the
    /// intended peer count.
    pub const fn new(
        intended_peers: u32,
        successful_peers: u32,
        failed_peers: u32,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let Some(accounted) = successful_peers.checked_add(failed_peers) else {
            return Err(RuntimeRecordValidationError::DeliveryCount);
        };
        if accounted != intended_peers {
            return Err(RuntimeRecordValidationError::DeliveryCount);
        }
        let severity = if intended_peers == 0 {
            DeliverySeverity::ZeroPeers
        } else if failed_peers == 0 {
            DeliverySeverity::Ok
        } else {
            DeliverySeverity::PartialFailure
        };
        Ok(Self {
            intended_peers,
            successful_peers,
            failed_peers,
            severity,
        })
    }
}

/// Current synchronization estimate for one listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynchronizationSummary {
    pub confidence: SyncConfidence,
    pub offset_ms: f64,
    pub round_trip_ms: f64,
    pub drift_ppm: f64,
}

impl SynchronizationSummary {
    /// Creates a finite synchronization summary with nonnegative round-trip time.
    ///
    /// # Errors
    ///
    /// Rejects non-finite values or a negative RTT.
    pub fn new(
        confidence: SyncConfidence,
        offset_ms: f64,
        round_trip_ms: f64,
        drift_ppm: f64,
    ) -> Result<Self, RuntimeRecordValidationError> {
        if !offset_ms.is_finite()
            || !round_trip_ms.is_finite()
            || round_trip_ms < 0.0
            || !drift_ppm.is_finite()
        {
            return Err(RuntimeRecordValidationError::SynchronizationSummary);
        }
        Ok(Self {
            confidence,
            offset_ms,
            round_trip_ms,
            drift_ppm,
        })
    }
}

/// Current host-side summary for one known listener.
#[derive(Debug, Clone, PartialEq)]
pub struct ListenerSummary {
    pub device_id: DeviceId,
    pub display_name: String,
    pub trust_state: TrustState,
    pub transport_state: TransportState,
    pub synchronization: Option<SynchronizationSummary>,
    pub last_contact: Option<MonotonicMillis>,
    pub last_error: Option<CoreError>,
}

impl ListenerSummary {
    /// Creates a bounded listener summary.
    ///
    /// # Errors
    ///
    /// Rejects a blank or oversized display name.
    pub fn new(
        device_id: DeviceId,
        display_name: impl Into<String>,
        trust_state: TrustState,
        transport_state: TransportState,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let display_name = display_name.into();
        validate_human_text(
            &display_name,
            MAX_DISPLAY_NAME_BYTES,
            RuntimeRecordValidationError::DeviceDisplayName,
        )?;
        Ok(Self {
            device_id,
            display_name,
            trust_state,
            transport_state,
            synchronization: None,
            last_contact: None,
            last_error: None,
        })
    }
}

/// One bounded diagnostic field emitted by the core actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticField {
    pub key: String,
    pub value: String,
}

impl DiagnosticField {
    /// Creates a bounded non-secret diagnostic field.
    ///
    /// # Errors
    ///
    /// Rejects blank, oversized, whitespace-surrounded keys and oversized or
    /// control-containing values.
    pub fn new(
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let key = key.into();
        let value = value.into();
        validate_token(
            &key,
            MAX_DIAGNOSTIC_FIELD_KEY_BYTES,
            RuntimeRecordValidationError::DiagnosticFieldKey,
        )?;
        validate_bounded_text(
            &value,
            MAX_DIAGNOSTIC_FIELD_VALUE_BYTES,
            RuntimeRecordValidationError::DiagnosticFieldValue,
        )?;
        Ok(Self { key, value })
    }
}

/// Structured bounded informational event emitted by the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDiagnostic {
    pub name: String,
    pub fields: Vec<DiagnosticField>,
}

impl CoreDiagnostic {
    /// Creates a bounded diagnostic event.
    ///
    /// # Errors
    ///
    /// Rejects an invalid name or more than 16 fields.
    pub fn new(
        name: impl Into<String>,
        fields: Vec<DiagnosticField>,
    ) -> Result<Self, RuntimeRecordValidationError> {
        let name = name.into();
        validate_token(
            &name,
            MAX_DIAGNOSTIC_EVENT_NAME_BYTES,
            RuntimeRecordValidationError::DiagnosticName,
        )?;
        if fields.len() > MAX_DIAGNOSTIC_FIELDS {
            return Err(RuntimeRecordValidationError::DiagnosticFieldLimit);
        }
        Ok(Self { name, fields })
    }
}

/// Stable failures while constructing actor records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRecordValidationError {
    RevisionOverflow,
    AudioSourceId,
    AudioSourceDisplayName,
    AudioSourceLength,
    AudioSourceRequired,
    SessionName,
    InviteCode,
    InviteCodeRequired,
    UnexpectedInviteCode,
    TuningSettings,
    NetworkPort,
    ProtocolVersion,
    DeviceDisplayName,
    DeliveryCount,
    SynchronizationSummary,
    DiagnosticName,
    DiagnosticFieldKey,
    DiagnosticFieldValue,
    DiagnosticFieldLimit,
}

impl fmt::Display for RuntimeRecordValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RevisionOverflow => "snapshot revision overflow",
            Self::AudioSourceId => "audio source ID is invalid",
            Self::AudioSourceDisplayName => "audio source display name is invalid",
            Self::AudioSourceLength => "audio source byte length must be greater than zero",
            Self::AudioSourceRequired => "a staged audio source is required",
            Self::SessionName => "session name is invalid",
            Self::InviteCode => "invite code is invalid",
            Self::InviteCodeRequired => "invite-code approval requires an invite code",
            Self::UnexpectedInviteCode => "invite code is not allowed for this approval mode",
            Self::TuningSettings => "tuning patch produces invalid settings",
            Self::NetworkPort => "network service ports must be nonzero",
            Self::ProtocolVersion => "protocol version must be nonzero",
            Self::DeviceDisplayName => "device display name is invalid",
            Self::DeliveryCount => "delivery counts are inconsistent",
            Self::SynchronizationSummary => "synchronization summary is invalid",
            Self::DiagnosticName => "diagnostic event name is invalid",
            Self::DiagnosticFieldKey => "diagnostic field key is invalid",
            Self::DiagnosticFieldValue => "diagnostic field value is invalid",
            Self::DiagnosticFieldLimit => "diagnostic field count exceeds the supported limit",
        })
    }
}

impl Error for RuntimeRecordValidationError {}

fn validate_human_text(
    value: &str,
    maximum_bytes: usize,
    error: RuntimeRecordValidationError,
) -> Result<(), RuntimeRecordValidationError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > maximum_bytes
        || value.chars().any(char::is_control)
    {
        return Err(error);
    }
    Ok(())
}

fn validate_token(
    value: &str,
    maximum_bytes: usize,
    error: RuntimeRecordValidationError,
) -> Result<(), RuntimeRecordValidationError> {
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

fn validate_bounded_text(
    value: &str,
    maximum_bytes: usize,
    error: RuntimeRecordValidationError,
) -> Result<(), RuntimeRecordValidationError> {
    if value.is_empty()
        || value.len() > maximum_bytes
        || value.chars().any(|character| character == '\0')
    {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AudioSourceDescriptor, AudioSourcePatch, DeliveryReport, HostDraft, HostDraftPatch,
        InviteCodePatch, NetworkEndpoint, RuntimeRecordValidationError, SnapshotRevision,
        SynchronizationSummary, TuningPatch,
    };
    use crate::domain::{ApprovalMode, DeliverySeverity, SyncConfidence, TuningSettings};
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn revisions_advance_without_wrapping() {
        assert_eq!(SnapshotRevision::new(7).checked_next().expect("next").get(), 8);
        assert_eq!(
            SnapshotRevision::new(u64::MAX).checked_next(),
            Err(RuntimeRecordValidationError::RevisionOverflow)
        );
    }

    #[test]
    fn host_patch_is_atomic_and_enforces_invite_policy() {
        let source = AudioSourceDescriptor::new("source-1", "Track.wav", Some(44), Some(1_000))
            .expect("valid source");
        let draft = HostDraft::default();
        let valid = HostDraftPatch {
            session_name: Some("Oakland test".to_owned()),
            approval_mode: Some(ApprovalMode::InviteCode),
            invite_code: InviteCodePatch::Set("2468".to_owned()),
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(true),
        };
        let next = draft.patched(&valid).expect("valid patch");
        next.validate_for_creation().expect("creatable draft");

        let invalid = HostDraftPatch {
            approval_mode: Some(ApprovalMode::Manual),
            ..HostDraftPatch::default()
        };
        assert_eq!(
            next.patched(&invalid),
            Err(RuntimeRecordValidationError::UnexpectedInviteCode)
        );
        assert_eq!(next.approval_mode, ApprovalMode::InviteCode);
    }

    #[test]
    fn tuning_patch_validates_cross_field_constraints() {
        let patch = TuningPatch {
            late_packet_threshold_ms: Some(100),
            hard_resync_threshold_ms: Some(110),
            ..TuningPatch::default()
        };
        assert_eq!(
            patch.apply_to(&TuningSettings::default()),
            Err(RuntimeRecordValidationError::TuningSettings)
        );
    }

    #[test]
    fn delivery_severity_is_derived_from_complete_accounting() {
        assert_eq!(
            DeliveryReport::new(0, 0, 0).expect("zero peers").severity,
            DeliverySeverity::ZeroPeers
        );
        assert_eq!(
            DeliveryReport::new(3, 2, 1).expect("partial").severity,
            DeliverySeverity::PartialFailure
        );
        assert_eq!(
            DeliveryReport::new(3, 3, 0).expect("complete").severity,
            DeliverySeverity::Ok
        );
        assert_eq!(
            DeliveryReport::new(3, 1, 1),
            Err(RuntimeRecordValidationError::DeliveryCount)
        );
    }

    #[test]
    fn endpoint_and_sync_summary_reject_invalid_values() {
        let address = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(NetworkEndpoint::new(address, 0, 2, 3).is_err());
        assert!(NetworkEndpoint::new(address, 1, 2, 3).is_ok());
        assert!(
            SynchronizationSummary::new(SyncConfidence::Good, 1.0, -1.0, 0.0).is_err()
        );
        assert!(
            SynchronizationSummary::new(SyncConfidence::Good, f64::NAN, 1.0, 0.0).is_err()
        );
    }
}
