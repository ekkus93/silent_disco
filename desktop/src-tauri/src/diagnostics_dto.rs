//! Bounded, redacted diagnostics DTOs (Block 35.1). Every field here is
//! safe to leave the process: no raw private paths, no key material, no
//! invite-code/session secrets. Reused by both the live diagnostics screen
//! (`get_host_diagnostics`) and the file export
//! (`export_host_diagnostics`), so the two never drift out of sync with
//! each other -- one DTO, two consumers.

use crate::dto::{CoreVersionDto, DesktopErrorDto};
use crate::host_session_dto::{BroadcastDeliveryDto, DeliveryReportDto, HostConnectionDto};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct VersionsDiagnosticsDto {
    pub core_version: CoreVersionDto,
    pub app_version: String,
    pub export_schema_version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ProfileDiagnosticsDto {
    pub profile_id: String,
    pub platform: String,
}

/// Storage subsystem health. `available: false` means the metadata query
/// itself failed (`failure_reason` explains why) -- never fabricated as
/// healthy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct StorageDiagnosticsDto {
    pub available: bool,
    pub schema_version: Option<u32>,
    pub journal_mode: Option<String>,
    pub foreign_keys_enabled: Option<bool>,
    pub integrity_check: Option<String>,
    pub applied_migration_count: Option<u32>,
    pub failure_reason: Option<String>,
}

/// Identity presence and a public-key fingerprint only -- never the
/// private key, never the symmetric device-identity secret, never DER
/// bytes (Block 35.1 "identity availability without secrets").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct IdentityDiagnosticsDto {
    pub device_identity_present: bool,
    pub signing_identity_present: bool,
    pub signing_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct TransportDiagnosticsDto {
    pub state: String,
    pub last_delivery: Option<DeliveryReportDto>,
    pub broadcast: Option<BroadcastDeliveryDto>,
}

/// Bounded per-listener summary -- deliberately narrower than
/// `ConnectedListenerDto` (no retry/resync-availability UI flags, which
/// are a live-session-control concern, not a diagnostics concern).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ListenerDiagnosticsDto {
    pub device_id: String,
    pub display_name: String,
    pub trust_state: String,
    pub transport_state: String,
    pub sync_confidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct SynchronizationDiagnosticsDto {
    pub confidence: String,
    pub offset_ms: String,
    pub round_trip_ms: String,
    pub drift_ppm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DecodeQueueDiagnosticsDto {
    pub state: String,
    pub queued_chunks: u32,
    pub queue_capacity_chunks: u32,
    pub backpressure_events: String,
    pub emitted_frames: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PacketizeQueueDiagnosticsDto {
    pub queued_packets: u32,
    pub queue_capacity: u32,
    pub backpressure_events: String,
    pub emitted_packets: String,
}

/// Live monitor status and render-callback counters (Block 35.1 "local
/// monitor and render counters"). Counters are `None` whenever `active` is
/// false -- there is nothing live to report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct MonitorDiagnosticsDto {
    pub enabled: bool,
    pub active: bool,
    pub failure_reason: Option<String>,
    pub callback_count: Option<String>,
    pub frames_written: Option<String>,
    pub frames_silence_filled: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct NotificationBridgeDiagnosticsDto {
    pub delivery_failure: Option<DesktopErrorDto>,
}

/// Full bounded diagnostics snapshot (Block 35.1). Every field is safe to
/// display, copy, or write to a file as-is -- see the module doc comment.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DesktopDiagnosticsDto {
    pub versions: VersionsDiagnosticsDto,
    pub profile: ProfileDiagnosticsDto,
    pub storage: StorageDiagnosticsDto,
    pub identity: IdentityDiagnosticsDto,
    pub endpoint: Option<HostConnectionDto>,
    pub transport: TransportDiagnosticsDto,
    pub listeners: Vec<ListenerDiagnosticsDto>,
    /// True whenever the real listener count exceeded the bounded
    /// `listeners` list above -- 35.3 "truncation/omission reported": a
    /// bounded export must say so, not silently drop entries and look
    /// complete.
    pub listeners_truncated: bool,
    pub synchronization: Option<SynchronizationDiagnosticsDto>,
    pub decode_queue: Option<DecodeQueueDiagnosticsDto>,
    pub packetize_queue: Option<PacketizeQueueDiagnosticsDto>,
    pub monitor: MonitorDiagnosticsDto,
    pub notification_bridge: NotificationBridgeDiagnosticsDto,
    pub last_error: Option<DesktopErrorDto>,
    pub shutting_down: bool,
    /// Wall-clock capture time, for a frontend "stale data" indicator
    /// (35.2) -- never used for sync/playback scheduling, which remains
    /// monotonic-only per this project's rules.
    pub generated_at_ms: String,
}
