//! Converts a validated local control-channel join frame into an authoritative core fact.

use super::host_transport_events::DesktopHostTransportEventSink;
use super::network_error::DesktopNetworkError;
use silent_disco_core::domain::{ApprovalMode, DeviceId, RequestId, TrustState};
use silent_disco_core::protocol::JoinRequest;
use silent_disco_core::runtime::{
    JoinRequestSummary, SessionAdvertisement, TransportEvent as CoreTransportEvent,
};
use std::collections::HashMap;

pub(super) struct PendingJoinProjection {
    requests: HashMap<DeviceId, RequestId>,
    next_sequence: u64,
}

impl PendingJoinProjection {
    pub(super) fn new() -> Self {
        Self {
            requests: HashMap::new(),
            next_sequence: 1,
        }
    }

    pub(super) fn register(
        &mut self,
        request: JoinRequest,
        received_at: silent_disco_core::domain::MonotonicMillis,
        advertisement: &SessionAdvertisement,
        sink: &dyn DesktopHostTransportEventSink,
    ) -> Result<DeviceId, DesktopNetworkError> {
        let device_id = request.device.device_id.clone();
        if self.requests.contains_key(&device_id) {
            return Ok(device_id);
        }
        let snapshot = sink
            .current_snapshot()
            .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
        let request_id = RequestId::new(format!("desktop-join-{}", self.next_sequence))
            .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let invite_code_valid = advertisement.approval_mode != ApprovalMode::InviteCode
            || request.invite_code.as_deref() == snapshot.host_draft.invite_code.as_deref();
        let summary = JoinRequestSummary::new(
            request_id.clone(),
            device_id.clone(),
            request.device.display_name,
            TrustState::SessionOnly,
            invite_code_valid,
            received_at,
        )
        .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))?;
        sink.submit_transport_event(CoreTransportEvent::JoinRequested(summary))
            .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
        self.requests.insert(device_id.clone(), request_id);
        Ok(device_id)
    }

    pub(super) fn remove(&mut self, device_id: &DeviceId) {
        self.requests.remove(device_id);
    }
}
