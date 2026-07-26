use core::fmt;
use std::error::Error;

/// Minimum number of synchronization samples retained for estimation.
pub const MIN_SYNC_SAMPLE_WINDOW: u16 = 4;
/// Maximum number of synchronization samples retained for estimation.
pub const MAX_SYNC_SAMPLE_WINDOW: u16 = 32;
/// Minimum synchronization cadence in milliseconds.
pub const MIN_SYNC_CADENCE_MS: u64 = 500;
/// Maximum synchronization cadence in milliseconds.
pub const MAX_SYNC_CADENCE_MS: u64 = 5_000;
/// Minimum startup buffer in milliseconds.
pub const MIN_STARTUP_BUFFER_MS: u64 = 100;
/// Maximum startup buffer in milliseconds.
pub const MAX_STARTUP_BUFFER_MS: u64 = 1_500;
/// Minimum late-packet threshold in milliseconds.
pub const MIN_LATE_PACKET_THRESHOLD_MS: u64 = 10;
/// Maximum late-packet threshold in milliseconds.
pub const MAX_LATE_PACKET_THRESHOLD_MS: u64 = 250;
/// Minimum hard-resynchronization threshold in milliseconds.
pub const MIN_HARD_RESYNC_THRESHOLD_MS: u64 = 40;
/// Maximum hard-resynchronization threshold in milliseconds.
pub const MAX_HARD_RESYNC_THRESHOLD_MS: u64 = 500;
/// Required separation between late-packet and hard-resynchronization thresholds.
pub const MIN_RESYNC_THRESHOLD_GAP_MS: u64 = 20;
/// Minimum synchronization drift threshold in milliseconds.
pub const MIN_SYNC_DRIFT_THRESHOLD_MS: f64 = 4.0;
/// Maximum synchronization drift threshold in milliseconds.
pub const MAX_SYNC_DRIFT_THRESHOLD_MS: f64 = 100.0;
/// Minimum discovery scan window in milliseconds.
pub const MIN_SCAN_WINDOW_MS: u64 = 1_000;
/// Maximum discovery scan window in milliseconds.
pub const MAX_SCAN_WINDOW_MS: u64 = 10_000;

/// Shared, validated timing and buffering settings.
#[derive(Debug, Clone, PartialEq)]
pub struct TuningSettings {
    pub sync_sample_window: u16,
    pub sync_cadence_ms: u64,
    pub startup_buffer_ms: u64,
    pub late_packet_threshold_ms: u64,
    pub hard_resync_threshold_ms: u64,
    pub sync_drift_threshold_ms: f64,
    pub scan_window_ms: u64,
}

impl Default for TuningSettings {
    fn default() -> Self {
        Self {
            sync_sample_window: 12,
            sync_cadence_ms: 2_000,
            startup_buffer_ms: 400,
            late_packet_threshold_ms: 40,
            hard_resync_threshold_ms: 120,
            sync_drift_threshold_ms: 18.0,
            scan_window_ms: 3_000,
        }
    }
}

impl TuningSettings {
    /// Validates every supported range and the cross-field resynchronization rule.
    ///
    /// # Errors
    ///
    /// Returns a stable validation error when any field is unsupported.
    pub fn validate(&self) -> Result<(), TuningSettingsValidationError> {
        if !(MIN_SYNC_SAMPLE_WINDOW..=MAX_SYNC_SAMPLE_WINDOW)
            .contains(&self.sync_sample_window)
        {
            return Err(TuningSettingsValidationError::SyncSampleWindow);
        }
        if !(MIN_SYNC_CADENCE_MS..=MAX_SYNC_CADENCE_MS).contains(&self.sync_cadence_ms) {
            return Err(TuningSettingsValidationError::SyncCadence);
        }
        if !(MIN_STARTUP_BUFFER_MS..=MAX_STARTUP_BUFFER_MS).contains(&self.startup_buffer_ms) {
            return Err(TuningSettingsValidationError::StartupBuffer);
        }
        if !(MIN_LATE_PACKET_THRESHOLD_MS..=MAX_LATE_PACKET_THRESHOLD_MS)
            .contains(&self.late_packet_threshold_ms)
        {
            return Err(TuningSettingsValidationError::LatePacketThreshold);
        }
        if !(MIN_HARD_RESYNC_THRESHOLD_MS..=MAX_HARD_RESYNC_THRESHOLD_MS)
            .contains(&self.hard_resync_threshold_ms)
        {
            return Err(TuningSettingsValidationError::HardResyncThreshold);
        }
        if !self.sync_drift_threshold_ms.is_finite()
            || !(MIN_SYNC_DRIFT_THRESHOLD_MS..=MAX_SYNC_DRIFT_THRESHOLD_MS)
                .contains(&self.sync_drift_threshold_ms)
        {
            return Err(TuningSettingsValidationError::SyncDriftThreshold);
        }
        if !(MIN_SCAN_WINDOW_MS..=MAX_SCAN_WINDOW_MS).contains(&self.scan_window_ms) {
            return Err(TuningSettingsValidationError::ScanWindow);
        }
        let minimum_hard_resync = self
            .late_packet_threshold_ms
            .checked_add(MIN_RESYNC_THRESHOLD_GAP_MS)
            .ok_or(TuningSettingsValidationError::ThresholdRelationship)?;
        if self.hard_resync_threshold_ms < minimum_hard_resync {
            return Err(TuningSettingsValidationError::ThresholdRelationship);
        }
        Ok(())
    }
}

/// Stable validation failures for [`TuningSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningSettingsValidationError {
    SyncSampleWindow,
    SyncCadence,
    StartupBuffer,
    LatePacketThreshold,
    HardResyncThreshold,
    SyncDriftThreshold,
    ScanWindow,
    ThresholdRelationship,
}

impl fmt::Display for TuningSettingsValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SyncSampleWindow => "sync sample window is outside the supported range",
            Self::SyncCadence => "sync cadence is outside the supported range",
            Self::StartupBuffer => "startup buffer is outside the supported range",
            Self::LatePacketThreshold => {
                "late-packet threshold is outside the supported range"
            }
            Self::HardResyncThreshold => {
                "hard-resynchronization threshold is outside the supported range"
            }
            Self::SyncDriftThreshold => {
                "sync drift threshold must be finite and inside the supported range"
            }
            Self::ScanWindow => "scan window is outside the supported range",
            Self::ThresholdRelationship => {
                "hard-resynchronization threshold must exceed the late-packet threshold by at least 20 milliseconds"
            }
        })
    }
}

impl Error for TuningSettingsValidationError {}

#[cfg(test)]
mod tests {
    use super::{TuningSettings, TuningSettingsValidationError};

    #[test]
    fn defaults_match_the_current_android_tuning_values() {
        assert_eq!(
            TuningSettings::default(),
            TuningSettings {
                sync_sample_window: 12,
                sync_cadence_ms: 2_000,
                startup_buffer_ms: 400,
                late_packet_threshold_ms: 40,
                hard_resync_threshold_ms: 120,
                sync_drift_threshold_ms: 18.0,
                scan_window_ms: 3_000,
            }
        );
    }

    #[test]
    fn rejects_non_finite_drift_and_invalid_threshold_relationship() {
        let non_finite = TuningSettings {
            sync_drift_threshold_ms: f64::NAN,
            ..TuningSettings::default()
        };
        assert_eq!(
            non_finite.validate(),
            Err(TuningSettingsValidationError::SyncDriftThreshold)
        );

        let invalid_relationship = TuningSettings {
            late_packet_threshold_ms: 100,
            hard_resync_threshold_ms: 110,
            ..TuningSettings::default()
        };
        assert_eq!(
            invalid_relationship.validate(),
            Err(TuningSettingsValidationError::ThresholdRelationship)
        );
    }
}
