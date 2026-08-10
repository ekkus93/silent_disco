//! Pure builder for the bounded, redacted diagnostics snapshot (Block
//! 35.1). Does no I/O of its own -- every input is already resolved by the
//! caller (`app_state.rs`'s `host_diagnostics`), which is what actually
//! locks the runtime, queries the database worker, and reads the
//! notification buffer. Kept pure specifically so it can be tested with
//! plain fixture inputs, the same discipline this codebase already applies
//! to `HostSessionSnapshotDto::from_parts`/`ConnectedListenerDto::from_parts`.

use super::host_transport::ActiveHostSessionSnapshot;
use super::monitor::MonitorStatus;
use super::network::StreamDiagnosticsSnapshot;
use crate::diagnostics_dto::{
    DecodeQueueDiagnosticsDto, DesktopDiagnosticsDto, IdentityDiagnosticsDto,
    ListenerDiagnosticsDto, MonitorDiagnosticsDto, NotificationBridgeDiagnosticsDto,
    PacketizeQueueDiagnosticsDto, ProfileDiagnosticsDto, StorageDiagnosticsDto,
    SynchronizationDiagnosticsDto, TransportDiagnosticsDto, VersionsDiagnosticsDto,
};
use crate::dto::{CoreVersionDto, DesktopErrorDto};
use crate::host_session_dto::{broadcast_delivery_dto, host_connection_dto};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::CoreSnapshot;

/// Schema version for the diagnostics export envelope -- bumped whenever
/// `DesktopDiagnosticsDto`'s shape changes in a way a consumer of a saved
/// export file would need to know about.
pub(crate) const DIAGNOSTICS_SCHEMA_VERSION: u16 = 2;

/// Hard cap on the per-listener detail list (Block 35.1 "bounded display" /
/// 35.3 "truncation/omission reported"). This project's own scope
/// explicitly excludes large-crowd optimization (`CLAUDE.md`), so this is
/// a generous bound against a genuinely pathological session, not a
/// realistic ceiling -- reaching it always sets `listeners_truncated`
/// rather than silently dropping entries and looking complete.
const MAX_DIAGNOSTICS_LISTENERS: usize = 64;

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_diagnostics_snapshot(
    core_snapshot: &CoreSnapshot,
    profile_id: &str,
    device_identity_present: bool,
    signing_identity_present: bool,
    signing_key_fingerprint: Option<String>,
    active: Option<&ActiveHostSessionSnapshot>,
    stream_diagnostics: Option<&StreamDiagnosticsSnapshot>,
    monitor: &MonitorStatus,
    storage: StorageDiagnosticsDto,
    notification_failure: Option<CoreError>,
    app_version: &str,
    now_ms: u64,
) -> DesktopDiagnosticsDto {
    DesktopDiagnosticsDto {
        versions: VersionsDiagnosticsDto {
            core_version: CoreVersionDto::from(silent_disco_core::core_version()),
            app_version: app_version.to_owned(),
            export_schema_version: DIAGNOSTICS_SCHEMA_VERSION,
        },
        profile: ProfileDiagnosticsDto {
            profile_id: profile_id.to_owned(),
            platform: std::env::consts::OS.to_owned(),
        },
        storage,
        identity: IdentityDiagnosticsDto {
            device_identity_present,
            signing_identity_present,
            signing_key_fingerprint,
        },
        endpoint: active.map(host_connection_dto),
        transport: TransportDiagnosticsDto {
            state: core_snapshot.transport_state.wire_name().to_owned(),
            last_delivery: core_snapshot
                .last_delivery
                .map(crate::host_session_dto::DeliveryReportDto::from),
            broadcast: active.map(broadcast_delivery_dto),
        },
        listeners: core_snapshot
            .listeners
            .iter()
            .take(MAX_DIAGNOSTICS_LISTENERS)
            .map(listener_diagnostics_dto)
            .collect(),
        listeners_truncated: core_snapshot.listeners.len() > MAX_DIAGNOSTICS_LISTENERS,
        synchronization: core_snapshot
            .synchronization
            .map(synchronization_diagnostics_dto),
        decode_queue: stream_diagnostics.map(|value| decode_queue_dto(&value.decode)),
        packetize_queue: stream_diagnostics.map(|value| packetize_queue_dto(value.packetize)),
        monitor: monitor_diagnostics_dto(monitor),
        notification_bridge: NotificationBridgeDiagnosticsDto {
            delivery_failure: notification_failure.map(DesktopErrorDto::from),
        },
        last_error: core_snapshot.last_error.clone().map(DesktopErrorDto::from),
        shutting_down: core_snapshot.shutting_down,
        generated_at_ms: now_ms.to_string(),
    }
}

fn listener_diagnostics_dto(
    value: &silent_disco_core::runtime::ListenerSummary,
) -> ListenerDiagnosticsDto {
    ListenerDiagnosticsDto {
        device_id: value.device_id.as_str().to_owned(),
        display_name: value.display_name.clone(),
        trust_state: value.trust_state.wire_name().to_owned(),
        transport_state: value.transport_state.wire_name().to_owned(),
        sync_confidence: value
            .synchronization
            .map(|sync| sync.confidence.wire_name().to_owned()),
    }
}

fn synchronization_diagnostics_dto(
    value: silent_disco_core::runtime::SynchronizationSummary,
) -> SynchronizationDiagnosticsDto {
    SynchronizationDiagnosticsDto {
        confidence: value.confidence.wire_name().to_owned(),
        offset_ms: value.offset_ms.to_string(),
        round_trip_ms: value.round_trip_ms.to_string(),
        drift_ppm: value.drift_ppm.to_string(),
    }
}

fn decode_queue_dto(
    value: &silent_disco_core::audio::DecodeStatistics,
) -> DecodeQueueDiagnosticsDto {
    DecodeQueueDiagnosticsDto {
        state: decode_worker_state_name(value.state),
        queued_chunks: u32::try_from(value.queued_chunks).unwrap_or(u32::MAX),
        queue_capacity_chunks: u32::try_from(value.queue_capacity_chunks).unwrap_or(u32::MAX),
        backpressure_events: value.backpressure_events.to_string(),
        emitted_frames: value.emitted_frames.to_string(),
    }
}

fn decode_worker_state_name(state: silent_disco_core::audio::DecodeWorkerState) -> String {
    use silent_disco_core::audio::DecodeWorkerState;
    match state {
        DecodeWorkerState::Running => "running",
        DecodeWorkerState::Completed => "completed",
        DecodeWorkerState::Cancelled => "cancelled",
        DecodeWorkerState::Failed => "failed",
    }
    .to_owned()
}

fn packetize_queue_dto(value: (usize, usize, u64, u64)) -> PacketizeQueueDiagnosticsDto {
    let (queued_packets, queue_capacity, backpressure_events, emitted_packets) = value;
    PacketizeQueueDiagnosticsDto {
        queued_packets: u32::try_from(queued_packets).unwrap_or(u32::MAX),
        queue_capacity: u32::try_from(queue_capacity).unwrap_or(u32::MAX),
        backpressure_events: backpressure_events.to_string(),
        emitted_packets: emitted_packets.to_string(),
    }
}

fn monitor_diagnostics_dto(status: &MonitorStatus) -> MonitorDiagnosticsDto {
    MonitorDiagnosticsDto {
        enabled: status.enabled,
        active: status.active,
        failure_reason: status.failure_reason.clone(),
        callback_count: status
            .telemetry
            .map(|telemetry| telemetry.callback_count.to_string()),
        frames_written: status
            .telemetry
            .map(|telemetry| telemetry.frames_written.to_string()),
        frames_silence_filled: status
            .telemetry
            .map(|telemetry| telemetry.frames_silence_filled.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::host_transport::ActiveHostSessionSnapshot;
    use super::super::monitor::{MonitorStatus, MonitorTelemetrySnapshot};
    use super::super::network::StreamDiagnosticsSnapshot;
    use super::build_diagnostics_snapshot;
    use crate::diagnostics_dto::StorageDiagnosticsDto;
    use silent_disco_core::audio::{DecodeStatistics, DecodeWorkerState};
    use silent_disco_core::domain::{
        ApprovalMode, DeviceId, SessionId, TransportState, TrustState,
    };
    use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
    use silent_disco_core::runtime::{
        CoreSnapshot, ListenerSummary, NetworkEndpoint, SessionAdvertisement,
    };
    use std::net::{IpAddr, Ipv4Addr};

    fn active_session() -> ActiveHostSessionSnapshot {
        ActiveHostSessionSnapshot {
            advertisement: SessionAdvertisement::new(
                SessionId::new("session-diag").expect("session id"),
                DeviceId::new("device-diag").expect("device id"),
                "Diagnostics Test Session",
                ApprovalMode::InviteCode,
                2,
                None,
            )
            .expect("advertisement"),
            endpoint: NetworkEndpoint::new(
                IpAddr::V4(Ipv4Addr::new(192, 168, 1, 77)),
                41_000,
                41_001,
                41_002,
            )
            .expect("endpoint"),
            worker_running: true,
            last_error: None,
            observed_at_ms: 1_000,
            broadcast: super::super::host_transport::BroadcastDiagnostics {
                frames_attempted: 10,
                frames_failed: 0,
                frames_fully_delivered: 10,
                frames_partially_delivered: 0,
                frames_without_recipients: 0,
                recipients_intended: 10,
                recipients_delivered: 10,
                queue_depth: 0,
                queue_peak_depth: 3,
                queue_overflows: 0,
            },
        }
    }

    fn empty_monitor() -> MonitorStatus {
        MonitorStatus {
            enabled: false,
            active: false,
            failure_reason: None,
            telemetry: None,
        }
    }

    fn active_monitor() -> MonitorStatus {
        MonitorStatus {
            enabled: true,
            active: true,
            failure_reason: None,
            telemetry: Some(MonitorTelemetrySnapshot {
                callback_count: 42,
                frames_written: 4_000,
                frames_silence_filled: 12,
            }),
        }
    }

    fn stream_diagnostics() -> StreamDiagnosticsSnapshot {
        StreamDiagnosticsSnapshot {
            decode: DecodeStatistics {
                state: DecodeWorkerState::Running,
                queued_chunks: 2,
                queue_capacity_chunks: 8,
                maximum_queued_duration_ms: 400,
                backpressure_events: 0,
                emitted_frames: 96_000,
            },
            packetize: (1, 8, 0, 4_800),
        }
    }

    fn available_storage() -> StorageDiagnosticsDto {
        StorageDiagnosticsDto {
            available: true,
            schema_version: Some(3),
            journal_mode: Some("wal".to_owned()),
            foreign_keys_enabled: Some(true),
            integrity_check: Some("ok".to_owned()),
            applied_migration_count: Some(5),
            failure_reason: None,
        }
    }

    /// Block 35.4 "secret redaction": a realistic snapshot -- an
    /// invite-code-gated session, a real endpoint, a signed identity
    /// fingerprint, an active monitor -- must never leak anything that
    /// looks like a raw secret once serialized. This mirrors
    /// `bindings.rs`'s own `output_does_not_include_secret_key_fields`
    /// discipline, applied to this DTO specifically.
    #[test]
    fn no_secret_shaped_content_appears_in_the_serialized_snapshot() {
        let mut snapshot = CoreSnapshot::default();
        snapshot.listeners.push(
            ListenerSummary::new(
                DeviceId::new("listener-diag").expect("device id"),
                "A Listener",
                TrustState::Trusted,
                TransportState::Connected,
            )
            .expect("listener summary"),
        );
        snapshot.last_error = CoreError::new(
            CoreErrorCode::TransportDeliveryFailed,
            "a bounded, already-safe error message",
            ErrorSeverity::Error,
            true,
            None,
        )
        .ok();

        let active = active_session();
        let dto = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            Some("deadbeef00112233".to_owned()),
            Some(&active),
            Some(&stream_diagnostics()),
            &active_monitor(),
            available_storage(),
            None,
            "0.1.0-test",
            1_700_000_000_000,
        );

        let json = serde_json::to_string(&dto).expect("serialize");
        for forbidden in [
            "privateKey",
            "private_key",
            "secret",
            "signingKey\":\"",
            "DER",
            "keyring",
        ] {
            assert!(
                !json.contains(forbidden),
                "diagnostics export must never contain {forbidden:?}, got: {json}"
            );
        }
        // The invite code itself must never appear, even though the
        // session that produced this snapshot uses invite-code approval.
        assert!(!json.to_lowercase().contains("invite_code_secret"));
    }

    /// Block 35.1 "bounded display" / 35.3 "truncation/omission reported":
    /// exceeding the cap must truncate the list and say so, never silently
    /// drop entries while looking complete.
    #[test]
    fn an_oversized_listener_list_is_truncated_and_reported() {
        let mut snapshot = CoreSnapshot::default();
        for index in 0..(super::MAX_DIAGNOSTICS_LISTENERS + 5) {
            snapshot.listeners.push(
                ListenerSummary::new(
                    DeviceId::new(format!("listener-{index}")).expect("device id"),
                    format!("Listener {index}"),
                    TrustState::SessionOnly,
                    TransportState::Connected,
                )
                .expect("listener summary"),
            );
        }

        let dto = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            None,
            None,
            None,
            &empty_monitor(),
            available_storage(),
            None,
            "0.1.0-test",
            0,
        );

        assert_eq!(dto.listeners.len(), super::MAX_DIAGNOSTICS_LISTENERS);
        assert!(dto.listeners_truncated);
    }

    /// A session with fewer listeners than the cap must never be reported
    /// as truncated -- the common case, asserted explicitly rather than
    /// only ever exercising the truncated path.
    #[test]
    fn a_normal_sized_listener_list_is_not_reported_as_truncated() {
        let snapshot = CoreSnapshot::default();
        let dto = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            None,
            None,
            None,
            &empty_monitor(),
            available_storage(),
            None,
            "0.1.0-test",
            0,
        );
        assert!(!dto.listeners_truncated);
        assert!(dto.listeners.is_empty());
    }

    /// Monitor telemetry must be present only while genuinely active, and
    /// absent otherwise -- never fabricated.
    #[test]
    fn monitor_telemetry_reflects_active_state_exactly() {
        let snapshot = CoreSnapshot::default();
        let inactive = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            None,
            None,
            None,
            &empty_monitor(),
            available_storage(),
            None,
            "0.1.0-test",
            0,
        );
        assert!(inactive.monitor.callback_count.is_none());

        let active = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            None,
            None,
            None,
            &active_monitor(),
            available_storage(),
            None,
            "0.1.0-test",
            0,
        );
        assert_eq!(active.monitor.callback_count.as_deref(), Some("42"));
        assert_eq!(active.monitor.frames_written.as_deref(), Some("4000"));
    }

    /// Block 35.4 "export after transport failure": a live transport
    /// failure must appear in the diagnostics snapshot -- `transport.state`
    /// and `last_error` both reflect it -- rather than the whole call
    /// erroring out or the failure being silently dropped. Unlike a startup
    /// failure (no `ReadyRuntime` exists yet), a transport failure happens
    /// on an otherwise-ready runtime, so the snapshot must still succeed and
    /// carry the failure as data.
    #[test]
    #[allow(
        clippy::field_reassign_with_default,
        reason = "CoreSnapshot has ~20 fields; a full literal would obscure the two under test"
    )]
    fn a_transport_failure_is_surfaced_in_the_snapshot_not_hidden() {
        let mut snapshot = CoreSnapshot::default();
        snapshot.transport_state = TransportState::Failed;
        snapshot.last_error = CoreError::new(
            CoreErrorCode::TransportConnectionLost,
            "listener connection lost",
            ErrorSeverity::Error,
            true,
            None,
        )
        .ok();

        let dto = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            None,
            None,
            None,
            &empty_monitor(),
            available_storage(),
            None,
            "0.1.0-test",
            0,
        );

        assert_eq!(dto.transport.state, "failed");
        assert!(dto.last_error.is_some());
    }

    /// Storage query failure must surface honestly (`available: false` and
    /// a reason), never be silently reported as healthy.
    #[test]
    fn a_storage_query_failure_is_never_reported_as_healthy() {
        let snapshot = CoreSnapshot::default();
        let failed_storage = StorageDiagnosticsDto {
            available: false,
            schema_version: None,
            journal_mode: None,
            foreign_keys_enabled: None,
            integrity_check: None,
            applied_migration_count: None,
            failure_reason: Some("database worker unavailable".to_owned()),
        };
        let dto = build_diagnostics_snapshot(
            &snapshot,
            "profile-diag",
            true,
            true,
            None,
            None,
            None,
            &empty_monitor(),
            failed_storage,
            None,
            "0.1.0-test",
            0,
        );
        assert!(!dto.storage.available);
        assert_eq!(
            dto.storage.failure_reason.as_deref(),
            Some("database worker unavailable")
        );
    }
}
