use crate::dto::DesktopErrorDto;
use crate::platform::host_transport::ActiveHostSessionSnapshot;
use crate::platform::network_dto::MonitorStatusDto;
use crate::runtime_dto::AudioSourceSummaryDto;
use silent_disco_core::domain::HostLifecycle;
use silent_disco_core::runtime::{
    CoreSnapshot, DeliveryReport, JoinRequestSummary, ListenerSummary, RecoverableAction,
};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostConnectionDto {
    pub host_address: String,
    pub control_port: u16,
    pub sync_port: u16,
    pub audio_port: u16,
    pub session_id: String,
    pub protocol_version: u16,
    pub invite_code_required: bool,
    pub expires_at_ms: Option<String>,
}

/// A freshly generated, signed P2 QR invitation for the desktop's active
/// host session (Block 31). Never cached backend-side -- each call to the
/// command that produces this is a brand-new signature over a brand-new
/// nonce/expiry, so "refresh" and "stale invitation is not silently reused"
/// (31.2) are automatic: there is no server-held "current invitation" to
/// silently keep serving.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostInvitationDto {
    pub payload: String,
    pub expires_at_ms: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct PendingJoinRequestDto {
    pub request_id: String,
    pub device_id: String,
    pub display_name: String,
    pub trust_state: String,
    pub invite_code_valid: bool,
    pub received_at_ms: String,
    pub age_ms: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct DeliveryReportDto {
    pub intended_peers: u32,
    pub successful_peers: u32,
    pub failed_peers: u32,
    pub severity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ConnectedListenerDto {
    pub device_id: String,
    pub display_name: String,
    pub trust_state: String,
    pub transport_state: String,
    pub last_contact_ms: Option<String>,
    pub last_contact_age_ms: Option<String>,
    pub sync_confidence: Option<String>,
    pub estimated_offset_ms: Option<String>,
    pub round_trip_time_ms: Option<String>,
    pub drift_ppm: Option<String>,
    pub last_control_delivery_state: String,
    pub retry_available: bool,
    pub resync_available: bool,
    pub can_remove: bool,
    pub last_error: Option<DesktopErrorDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HostSessionSnapshotDto {
    pub revision: String,
    pub host_lifecycle: String,
    pub transport_state: String,
    pub playback_state: String,
    pub playback_position_ms: String,
    /// True once the current/most recent stream reached its own natural end,
    /// distinct from an explicit stop -- both otherwise present as the same
    /// generic `playback_state` of `stopped`.
    pub stream_ended_naturally: bool,
    pub audio_source: Option<AudioSourceSummaryDto>,
    pub session_name: String,
    pub connection: Option<HostConnectionDto>,
    pub pending_join_requests: Vec<PendingJoinRequestDto>,
    pub connected_listeners: Vec<ConnectedListenerDto>,
    pub last_delivery: Option<DeliveryReportDto>,
    pub recoverable_action: Option<String>,
    pub playback_controls_enabled: bool,
    pub transport_worker_running: bool,
    pub transport_error: Option<String>,
    pub broadcast: Option<BroadcastDeliveryDto>,
    pub last_error: Option<DesktopErrorDto>,
    /// Local monitor status (Block 34). Independent of `connection`/network
    /// state -- monitor audio affects only what is heard at this desktop
    /// machine, never what a listener receives.
    pub monitor: MonitorStatusDto,
}

/// Delivery and queue-pressure accounting for the real-time broadcast path,
/// so partial delivery, zero-recipient broadcasts, and queue overflow are
/// visible instead of being folded into a single last-error string.
///
/// Counts are per delivery attempt rather than per listener identity: the
/// transport reports intended/successful/failed totals, and attributing a
/// failure to a specific peer would need a shared-transport change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct BroadcastDeliveryDto {
    pub frames_attempted: String,
    pub frames_failed: String,
    pub frames_fully_delivered: String,
    pub frames_partially_delivered: String,
    pub frames_without_recipients: String,
    pub recipients_intended: String,
    pub recipients_delivered: String,
    pub queue_depth: String,
    pub queue_peak_depth: String,
    pub queue_overflows: String,
}

impl HostSessionSnapshotDto {
    #[must_use]
    pub(crate) fn from_parts(
        snapshot: &CoreSnapshot,
        active: Option<&ActiveHostSessionSnapshot>,
        monitor: MonitorStatusDto,
    ) -> Self {
        let observed_at_ms = active.map_or(0, |value| value.observed_at_ms);
        let connection = active.map(|value| HostConnectionDto {
            host_address: value.endpoint.address.to_string(),
            control_port: value.endpoint.control_port,
            sync_port: value.endpoint.sync_port,
            audio_port: value.endpoint.audio_port,
            session_id: value.advertisement.session_id.as_str().to_owned(),
            protocol_version: value.advertisement.protocol_version,
            invite_code_required: value.advertisement.approval_mode
                == silent_disco_core::domain::ApprovalMode::InviteCode,
            expires_at_ms: None,
        });
        let broadcast = active.map(|value| BroadcastDeliveryDto {
            frames_attempted: value.broadcast.frames_attempted.to_string(),
            frames_failed: value.broadcast.frames_failed.to_string(),
            frames_fully_delivered: value.broadcast.frames_fully_delivered.to_string(),
            frames_partially_delivered: value.broadcast.frames_partially_delivered.to_string(),
            frames_without_recipients: value.broadcast.frames_without_recipients.to_string(),
            recipients_intended: value.broadcast.recipients_intended.to_string(),
            recipients_delivered: value.broadcast.recipients_delivered.to_string(),
            queue_depth: value.broadcast.queue_depth.to_string(),
            queue_peak_depth: value.broadcast.queue_peak_depth.to_string(),
            queue_overflows: value.broadcast.queue_overflows.to_string(),
        });
        let last_delivery_state = snapshot
            .last_delivery
            .as_ref()
            .map_or("not_observed", |report| report.severity.wire_name())
            .to_owned();
        let retry_available = snapshot.recoverable_action == Some(RecoverableAction::Retry);
        let resync_available =
            snapshot.recoverable_action == Some(RecoverableAction::Resynchronize);
        let can_remove = matches!(
            snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners
                | HostLifecycle::Ready
                | HostLifecycle::Streaming
                | HostLifecycle::Paused
        );
        let playback_controls_enabled = can_remove
            && active.is_some_and(|value| value.worker_running)
            && snapshot.host_draft.audio_source.is_some();
        let audio_source =
            snapshot
                .host_draft
                .audio_source
                .as_ref()
                .map(|source| AudioSourceSummaryDto {
                    source_id: source.source_id.clone(),
                    display_name: source.display_name.clone(),
                    byte_length: source.byte_length.map(|length| length.to_string()),
                    duration_ms: source.duration_ms.map(|duration| duration.to_string()),
                });

        Self {
            revision: snapshot.revision.get().to_string(),
            host_lifecycle: snapshot.host_lifecycle.wire_name().to_owned(),
            transport_state: snapshot.transport_state.wire_name().to_owned(),
            playback_state: snapshot.playback_state.wire_name().to_owned(),
            playback_position_ms: snapshot.playback_position_ms.to_string(),
            stream_ended_naturally: snapshot.stream_ended_naturally,
            audio_source,
            session_name: snapshot.host_draft.session_name.clone(),
            connection,
            pending_join_requests: snapshot
                .pending_join_requests
                .iter()
                .map(|request| PendingJoinRequestDto::from_parts(request, observed_at_ms))
                .collect(),
            connected_listeners: snapshot
                .listeners
                .iter()
                .map(|listener| {
                    ConnectedListenerDto::from_parts(
                        listener,
                        observed_at_ms,
                        &last_delivery_state,
                        retry_available,
                        resync_available,
                        can_remove,
                    )
                })
                .collect(),
            last_delivery: snapshot.last_delivery.map(DeliveryReportDto::from),
            recoverable_action: snapshot
                .recoverable_action
                .map(recoverable_action_name)
                .map(str::to_owned),
            playback_controls_enabled,
            transport_worker_running: active.is_some_and(|value| value.worker_running),
            transport_error: active.and_then(|value| value.last_error.clone()),
            broadcast,
            last_error: snapshot.last_error.clone().map(DesktopErrorDto::from),
            monitor,
        }
    }
}

impl PendingJoinRequestDto {
    fn from_parts(value: &JoinRequestSummary, observed_at_ms: u64) -> Self {
        let received_at_ms = value.received_at.get();
        Self {
            request_id: value.request_id.as_str().to_owned(),
            device_id: value.device_id.as_str().to_owned(),
            display_name: value.display_name.clone(),
            trust_state: value.trust_state.wire_name().to_owned(),
            invite_code_valid: value.invite_code_valid,
            received_at_ms: received_at_ms.to_string(),
            age_ms: observed_at_ms.saturating_sub(received_at_ms).to_string(),
        }
    }
}

impl ConnectedListenerDto {
    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        value: &ListenerSummary,
        observed_at_ms: u64,
        last_delivery_state: &str,
        retry_available: bool,
        resync_available: bool,
        can_remove: bool,
    ) -> Self {
        let last_contact_ms = value
            .last_contact
            .map(silent_disco_core::domain::MonotonicMillis::get);
        Self {
            device_id: value.device_id.as_str().to_owned(),
            display_name: value.display_name.clone(),
            trust_state: value.trust_state.wire_name().to_owned(),
            transport_state: value.transport_state.wire_name().to_owned(),
            last_contact_ms: last_contact_ms.map(|time| time.to_string()),
            last_contact_age_ms: last_contact_ms
                .map(|time| observed_at_ms.saturating_sub(time).to_string()),
            sync_confidence: value
                .synchronization
                .map(|summary| summary.confidence.wire_name().to_owned()),
            estimated_offset_ms: value
                .synchronization
                .map(|summary| summary.offset_ms.to_string()),
            round_trip_time_ms: value
                .synchronization
                .map(|summary| summary.round_trip_ms.to_string()),
            drift_ppm: value
                .synchronization
                .map(|summary| summary.drift_ppm.to_string()),
            last_control_delivery_state: last_delivery_state.to_owned(),
            retry_available,
            resync_available,
            can_remove,
            last_error: value.last_error.clone().map(DesktopErrorDto::from),
        }
    }
}

impl From<DeliveryReport> for DeliveryReportDto {
    fn from(value: DeliveryReport) -> Self {
        Self {
            intended_peers: value.intended_peers,
            successful_peers: value.successful_peers,
            failed_peers: value.failed_peers,
            severity: value.severity.wire_name().to_owned(),
        }
    }
}

const fn recoverable_action_name(value: RecoverableAction) -> &'static str {
    match value {
        RecoverableAction::Retry => "retry",
        RecoverableAction::Rescan => "rescan",
        RecoverableAction::Reconnect => "reconnect",
        RecoverableAction::Resynchronize => "resynchronize",
        RecoverableAction::ReselectAudioSource => "reselect_audio_source",
    }
}

#[cfg(test)]
mod tests {
    use super::HostSessionSnapshotDto;
    use crate::platform::host_transport::ActiveHostSessionSnapshot;
    use crate::platform::network_dto::MonitorStatusDto;
    use silent_disco_core::domain::{
        ApprovalMode, DeviceId, HostLifecycle, MonotonicMillis, RequestId, SessionId,
        SyncConfidence, TransportState, TrustState,
    };
    use silent_disco_core::runtime::{
        AudioSourceDescriptor, CoreSnapshot, DeliveryReport, JoinRequestSummary, ListenerSummary,
        NetworkEndpoint, SessionAdvertisement, SynchronizationSummary,
    };
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn projection_exposes_age_sync_delivery_and_core_capabilities() {
        let mut snapshot = CoreSnapshot {
            host_lifecycle: HostLifecycle::Ready,
            last_delivery: Some(DeliveryReport::new(2, 1, 1).expect("delivery")),
            playback_position_ms: 12_345,
            stream_ended_naturally: true,
            ..CoreSnapshot::default()
        };
        snapshot.host_draft.audio_source = Some(
            AudioSourceDescriptor::new("source-1", "Track One.wav", Some(4_096), Some(60_000))
                .expect("source"),
        );
        snapshot.pending_join_requests.push(
            JoinRequestSummary::new(
                RequestId::new("request-1").expect("request"),
                DeviceId::new("listener-1").expect("device"),
                "Listener One",
                TrustState::SessionOnly,
                true,
                MonotonicMillis::new(100),
            )
            .expect("join"),
        );
        let mut listener = ListenerSummary::new(
            DeviceId::new("listener-2").expect("device"),
            "Listener Two",
            TrustState::Trusted,
            TransportState::Connected,
        )
        .expect("listener");
        listener.last_contact = Some(MonotonicMillis::new(150));
        listener.synchronization = Some(
            SynchronizationSummary::new(SyncConfidence::Good, -2.5, 18.0, 1.25).expect("sync"),
        );
        snapshot.listeners.push(listener);
        let endpoint = NetworkEndpoint::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4100, 4101, 4102)
            .expect("endpoint");
        let advertisement = SessionAdvertisement::new(
            SessionId::new("session-1").expect("session"),
            DeviceId::new("host-1").expect("host"),
            "Session One",
            ApprovalMode::Manual,
            2,
            Some(endpoint),
        )
        .expect("advertisement");
        let active = ActiveHostSessionSnapshot {
            advertisement,
            endpoint,
            worker_running: true,
            last_error: None,
            observed_at_ms: 250,
            broadcast: crate::platform::host_transport::BroadcastDiagnostics {
                frames_attempted: 400,
                frames_failed: 1,
                frames_fully_delivered: 380,
                frames_partially_delivered: 12,
                frames_without_recipients: 7,
                recipients_intended: 800,
                recipients_delivered: 772,
                queue_depth: 3,
                queue_peak_depth: 41,
                queue_overflows: 2,
            },
        };

        let dto = HostSessionSnapshotDto::from_parts(
            &snapshot,
            Some(&active),
            MonitorStatusDto {
                enabled: false,
                active: false,
                failure_reason: None,
            },
        );
        // Broadcast delivery must reach the UI as distinguishable outcomes,
        // not be folded into one last-error string.
        let broadcast = dto.broadcast.as_ref().expect("broadcast diagnostics");
        assert_eq!(broadcast.frames_attempted, "400");
        assert_eq!(broadcast.frames_partially_delivered, "12");
        assert_eq!(broadcast.frames_without_recipients, "7");
        assert_eq!(broadcast.recipients_delivered, "772");
        assert_eq!(broadcast.queue_peak_depth, "41");
        assert_eq!(broadcast.queue_overflows, "2");
        // Position, natural completion, and the selected source must reach
        // the frontend snapshot rather than only existing on the actor.
        assert_eq!(dto.playback_position_ms, "12345");
        assert!(dto.stream_ended_naturally);
        let audio_source = dto.audio_source.as_ref().expect("audio source summary");
        assert_eq!(audio_source.display_name, "Track One.wav");
        assert_eq!(audio_source.duration_ms.as_deref(), Some("60000"));
        assert_eq!(dto.pending_join_requests[0].age_ms, "150");
        assert_eq!(
            dto.connected_listeners[0].last_contact_age_ms.as_deref(),
            Some("100")
        );
        assert_eq!(
            dto.connected_listeners[0].sync_confidence.as_deref(),
            Some("good")
        );
        assert_eq!(
            dto.connected_listeners[0].last_control_delivery_state,
            "partial_failure"
        );
        assert!(dto.connected_listeners[0].can_remove);
        assert_eq!(
            dto.last_delivery.as_ref().map(|value| value.failed_peers),
            Some(1)
        );
    }
}
