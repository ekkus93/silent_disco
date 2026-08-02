use crate::dto::DesktopErrorDto;
use crate::platform::host_transport::ActiveHostSessionSnapshot;
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
    pub session_name: String,
    pub connection: Option<HostConnectionDto>,
    pub pending_join_requests: Vec<PendingJoinRequestDto>,
    pub connected_listeners: Vec<ConnectedListenerDto>,
    pub last_delivery: Option<DeliveryReportDto>,
    pub recoverable_action: Option<String>,
    pub playback_controls_enabled: bool,
    pub transport_worker_running: bool,
    pub transport_error: Option<String>,
    pub last_error: Option<DesktopErrorDto>,
}

impl HostSessionSnapshotDto {
    #[must_use]
    pub(crate) fn from_parts(
        snapshot: &CoreSnapshot,
        active: Option<&ActiveHostSessionSnapshot>,
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

        Self {
            revision: snapshot.revision.get().to_string(),
            host_lifecycle: snapshot.host_lifecycle.wire_name().to_owned(),
            transport_state: snapshot.transport_state.wire_name().to_owned(),
            playback_state: snapshot.playback_state.wire_name().to_owned(),
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
            last_error: snapshot.last_error.clone().map(DesktopErrorDto::from),
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
    use silent_disco_core::domain::{
        ApprovalMode, DeviceId, HostLifecycle, MonotonicMillis, RequestId, SessionId,
        SyncConfidence, TransportState, TrustState,
    };
    use silent_disco_core::runtime::{
        CoreSnapshot, DeliveryReport, JoinRequestSummary, ListenerSummary, NetworkEndpoint,
        SessionAdvertisement, SynchronizationSummary,
    };
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn projection_exposes_age_sync_delivery_and_core_capabilities() {
        let mut snapshot = CoreSnapshot {
            host_lifecycle: HostLifecycle::Ready,
            last_delivery: Some(DeliveryReport::new(2, 1, 1).expect("delivery")),
            ..CoreSnapshot::default()
        };
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
        };

        let dto = HostSessionSnapshotDto::from_parts(&snapshot, Some(&active));
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
