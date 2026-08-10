use crate::platform::storage_inspection::ProfileStorageInspection;
use crate::profile::{ProfileDisplayName, ProfileId};
use serde::{Deserialize, Serialize};
use silent_disco_core::CoreVersion;
use std::fmt;
use ts_rs::TS;

const MAX_TRUSTED_DEVICE_SUMMARIES: usize = 256;
const MAX_ERROR_MESSAGE_CHARS: usize = 512;

/// Shared-core version returned by the desktop bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CoreVersionDto {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl From<CoreVersion> for CoreVersionDto {
    fn from(value: CoreVersion) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
            patch: value.patch,
        }
    }
}

/// Safe frontend-visible profile identity without native paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProfileSummaryDto {
    pub profile_id: String,
    pub display_name: String,
}

impl ProfileSummaryDto {
    #[must_use]
    pub fn new(profile_id: &ProfileId, display_name: &ProfileDisplayName) -> Self {
        Self {
            profile_id: profile_id.as_str().to_owned(),
            display_name: display_name.as_str().to_owned(),
        }
    }
}

/// Stable desktop bridge lifecycle. Domain lifecycle remains Rust-core-owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "details",
    rename_all = "camelCase"
)]
#[ts(tag = "kind", content = "details", rename_all = "camelCase")]
pub enum BridgeLifecycleDto {
    Closed,
    Opening { profile_id: String },
    Ready { profile_id: String },
    Failed { error: DesktopErrorDto },
}

/// Whole-application shutdown phase (Block 36.1/36.3), distinct from
/// [`BridgeLifecycleDto`]'s profile-focused open/reopen lifecycle -- see
/// `app_shutdown.rs`'s module doc comment for why these are two separate
/// state machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "details",
    rename_all = "camelCase"
)]
#[ts(tag = "kind", content = "details", rename_all = "camelCase")]
pub enum AppShutdownPhaseDto {
    NotRequested,
    ShuttingDown,
    Terminated,
    ShutdownFailed { error: DesktopErrorDto },
}

impl From<crate::app_shutdown::AppShutdownPhase> for AppShutdownPhaseDto {
    fn from(value: crate::app_shutdown::AppShutdownPhase) -> Self {
        match value {
            crate::app_shutdown::AppShutdownPhase::NotRequested => Self::NotRequested,
            crate::app_shutdown::AppShutdownPhase::ShuttingDown => Self::ShuttingDown,
            crate::app_shutdown::AppShutdownPhase::Terminated => Self::Terminated,
            crate::app_shutdown::AppShutdownPhase::ShutdownFailed(error) => {
                Self::ShutdownFailed { error }
            }
        }
    }
}

/// Structured redacted desktop error suitable for IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DesktopErrorDto {
    pub code: String,
    pub subsystem: String,
    pub severity: String,
    pub retryable: bool,
    pub message: String,
}

impl DesktopErrorDto {
    #[must_use]
    pub fn new(
        code: &str,
        subsystem: &str,
        severity: &str,
        retryable: bool,
        message: &str,
    ) -> Self {
        Self {
            code: code.to_owned(),
            subsystem: subsystem.to_owned(),
            severity: severity.to_owned(),
            retryable,
            message: bounded_message(message),
        }
    }
}

/// One applied database migration. Timestamps are strings to preserve u64 precision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MigrationSummaryDto {
    pub version: u32,
    pub applied_at_ms: String,
    pub checksum: String,
}

/// Validated persisted tuning values. Millisecond u64 values are decimal strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct StoredSettingsSummaryDto {
    pub sync_sample_window: u16,
    pub sync_cadence_ms: String,
    pub startup_buffer_ms: String,
    pub late_packet_threshold_ms: String,
    pub hard_resync_threshold_ms: String,
    pub sync_drift_threshold_ms: f64,
    pub scan_window_ms: String,
    pub updated_at_ms: String,
}

/// Redacted trusted-device summary. Key bytes and key references never cross IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TrustedDeviceSummaryDto {
    pub device_id: String,
    pub display_name: String,
    pub trust_state: String,
    pub first_seen_ms: String,
    pub last_seen_ms: String,
    pub updated_at_ms: String,
    pub has_public_key: bool,
    pub has_private_key_reference: bool,
}

/// Read-only typed storage inspection returned by the initial desktop bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct StorageInspectionDto {
    pub sqlite_version: String,
    pub foreign_keys_enabled: bool,
    pub journal_mode: String,
    pub busy_timeout_ms: u32,
    pub synchronous_policy: String,
    pub schema_version: u32,
    pub applied_migrations: Vec<MigrationSummaryDto>,
    pub integrity_check: String,
    pub settings: Option<StoredSettingsSummaryDto>,
    pub trusted_devices: Vec<TrustedDeviceSummaryDto>,
}

impl TryFrom<ProfileStorageInspection> for StorageInspectionDto {
    type Error = DtoConversionError;

    fn try_from(value: ProfileStorageInspection) -> Result<Self, Self::Error> {
        if value.trusted_devices.len() > MAX_TRUSTED_DEVICE_SUMMARIES {
            return Err(DtoConversionError::TrustedDeviceLimitExceeded {
                found: value.trusted_devices.len(),
                maximum: MAX_TRUSTED_DEVICE_SUMMARIES,
            });
        }

        let settings = value.settings.map(|stored| StoredSettingsSummaryDto {
            sync_sample_window: stored.tuning.sync_sample_window,
            sync_cadence_ms: stored.tuning.sync_cadence_ms.to_string(),
            startup_buffer_ms: stored.tuning.startup_buffer_ms.to_string(),
            late_packet_threshold_ms: stored.tuning.late_packet_threshold_ms.to_string(),
            hard_resync_threshold_ms: stored.tuning.hard_resync_threshold_ms.to_string(),
            sync_drift_threshold_ms: stored.tuning.sync_drift_threshold_ms,
            scan_window_ms: stored.tuning.scan_window_ms.to_string(),
            updated_at_ms: stored.updated_at_ms.to_string(),
        });
        let trusted_devices = value
            .trusted_devices
            .into_iter()
            .map(|device| TrustedDeviceSummaryDto {
                device_id: device.device_id.as_str().to_owned(),
                display_name: device.display_name,
                trust_state: device.trust_state.wire_name().to_owned(),
                first_seen_ms: device.first_seen_ms.to_string(),
                last_seen_ms: device.last_seen_ms.to_string(),
                updated_at_ms: device.updated_at_ms.to_string(),
                has_public_key: device.public_key.is_some(),
                has_private_key_reference: device.private_key_ref.is_some(),
            })
            .collect();
        let applied_migrations = value
            .metadata
            .applied_migrations
            .into_iter()
            .map(|migration| MigrationSummaryDto {
                version: migration.version,
                applied_at_ms: migration.applied_at_ms.to_string(),
                checksum: migration.checksum,
            })
            .collect();

        Ok(Self {
            sqlite_version: value.metadata.sqlite_version,
            foreign_keys_enabled: value.metadata.foreign_keys_enabled,
            journal_mode: value.metadata.journal_mode,
            busy_timeout_ms: value.metadata.busy_timeout_ms,
            synchronous_policy: value.metadata.synchronous_policy.stable_name().to_owned(),
            schema_version: value.metadata.schema_version,
            applied_migrations,
            integrity_check: value.metadata.integrity_check,
            settings,
            trusted_devices,
        })
    }
}

/// A bridge DTO conversion rejected an unsafe or unbounded value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DtoConversionError {
    TrustedDeviceLimitExceeded { found: usize, maximum: usize },
}

impl fmt::Display for DtoConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrustedDeviceLimitExceeded { found, maximum } => write!(
                formatter,
                "trusted-device summary count {found} exceeds the IPC limit {maximum}"
            ),
        }
    }
}

impl std::error::Error for DtoConversionError {}

fn bounded_message(message: &str) -> String {
    let mut result = String::new();
    for character in message.chars().take(MAX_ERROR_MESSAGE_CHARS) {
        if character == '\0' || character.is_control() && character != '\n' && character != '\t' {
            result.push('�');
        } else {
            result.push(character);
        }
    }
    if message.chars().count() > MAX_ERROR_MESSAGE_CHARS {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeLifecycleDto, DesktopErrorDto, StorageInspectionDto, TrustedDeviceSummaryDto,
        bounded_message,
    };
    use silent_disco_core::domain::{DeviceId, TrustState, TuningSettings};
    use silent_disco_core::storage::{
        DatabaseMetadata, MigrationRecord, StoredSettings, SynchronousPolicy, TrustedDevice,
    };

    #[test]
    fn tagged_lifecycle_round_trip_preserves_kind() {
        let value = BridgeLifecycleDto::Failed {
            error: DesktopErrorDto::new(
                "desktop.storage.open_failed",
                "storage",
                "fatal",
                false,
                "database unavailable",
            ),
        };
        let json = serde_json::to_value(&value).expect("serialize lifecycle");
        assert_eq!(json["kind"], "failed");
        assert_eq!(
            json["details"]["error"]["code"],
            "desktop.storage.open_failed"
        );
        assert_eq!(
            serde_json::from_value::<BridgeLifecycleDto>(json).expect("deserialize lifecycle"),
            value
        );
    }

    #[test]
    fn storage_conversion_redacts_key_material_and_uses_string_timestamps() {
        let inspection = crate::platform::storage_inspection::ProfileStorageInspection {
            metadata: DatabaseMetadata {
                sqlite_version: "3.50.0".to_owned(),
                foreign_keys_enabled: true,
                journal_mode: "wal".to_owned(),
                busy_timeout_ms: 2_000,
                synchronous_policy: SynchronousPolicy::Full,
                schema_version: 2,
                applied_migrations: vec![MigrationRecord {
                    version: 1,
                    applied_at_ms: u64::MAX,
                    checksum: "fnv1a64:abc".to_owned(),
                }],
                integrity_check: "ok".to_owned(),
                owner_thread_id: "ThreadId(7)".to_owned(),
                owner_thread_name: "silent-disco-database".to_owned(),
            },
            settings: Some(StoredSettings {
                tuning: TuningSettings::default(),
                updated_at_ms: u64::MAX,
            }),
            trusted_devices: vec![TrustedDevice {
                device_id: DeviceId::new("phone-1").expect("valid device ID"),
                display_name: "Listener phone".to_owned(),
                public_key: Some(vec![1, 2, 3]),
                private_key_ref: Some("secret-service:item-1".to_owned()),
                trust_state: TrustState::Trusted,
                first_seen_ms: 1,
                last_seen_ms: 2,
                updated_at_ms: u64::MAX,
            }],
        };

        let dto = StorageInspectionDto::try_from(inspection).expect("convert inspection");
        assert_eq!(
            dto.applied_migrations[0].applied_at_ms,
            u64::MAX.to_string()
        );
        assert_eq!(
            dto.settings.as_ref().expect("settings").updated_at_ms,
            u64::MAX.to_string()
        );
        assert_eq!(
            dto.trusted_devices,
            vec![TrustedDeviceSummaryDto {
                device_id: "phone-1".to_owned(),
                display_name: "Listener phone".to_owned(),
                trust_state: "trusted".to_owned(),
                first_seen_ms: "1".to_owned(),
                last_seen_ms: "2".to_owned(),
                updated_at_ms: u64::MAX.to_string(),
                has_public_key: true,
                has_private_key_reference: true,
            }]
        );

        let json = serde_json::to_string(&dto).expect("serialize inspection DTO");
        assert!(!json.contains("secret-service:item-1"));
        assert!(!json.contains("[1,2,3]"));
        assert!(!json.contains("ownerThread"));
    }

    #[test]
    fn error_message_is_bounded_and_control_characters_are_replaced() {
        let source = format!("a{}{}", '\u{0007}', "b".repeat(600));
        let bounded = bounded_message(&source);
        assert!(bounded.chars().count() <= 513);
        assert!(bounded.contains('�'));
        assert!(bounded.ends_with('…'));
    }
}
