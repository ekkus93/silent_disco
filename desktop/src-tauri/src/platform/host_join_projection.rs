//! Converts a validated local control-channel join frame into an authoritative core fact.

use super::host_transport_events::DesktopHostTransportEventSink;
use super::network_error::DesktopNetworkError;
use silent_disco_core::domain::{ApprovalMode, DeviceId, RequestId, TrustState};
use silent_disco_core::protocol::JoinRequest;
use silent_disco_core::runtime::{
    JoinRequestSummary, SessionAdvertisement, TransportEvent as CoreTransportEvent,
};
use std::collections::HashMap;
use std::net::SocketAddr;

/// A pending join's listener-reported sync/audio UDP ports -- needed once
/// (if) the join is approved, to call `authorize_peer_ports`. The host
/// cannot discover these ports any other way (the listener's audio channel
/// is receive-only and never sends a datagram the host could observe a
/// source address from).
struct PendingJoinPorts {
    control_address: SocketAddr,
    sync_port: u16,
    audio_port: u16,
    display_name: String,
}

pub(super) struct PendingJoinProjection {
    requests: HashMap<DeviceId, PendingJoinPorts>,
    // Retain ownership after approval removes pending ports. An old TCP
    // reader can finish after a reconnect and must not disconnect its successor.
    current_peers: HashMap<DeviceId, SocketAddr>,
    next_sequence: u64,
}

impl PendingJoinProjection {
    pub(super) fn new() -> Self {
        Self {
            requests: HashMap::new(),
            current_peers: HashMap::new(),
            next_sequence: 1,
        }
    }

    pub(super) fn register(
        &mut self,
        request: &JoinRequest,
        control_address: SocketAddr,
        received_at: silent_disco_core::domain::MonotonicMillis,
        advertisement: &SessionAdvertisement,
        sink: &dyn DesktopHostTransportEventSink,
    ) -> Result<DeviceId, DesktopNetworkError> {
        let device_id = request.device.device_id.clone();
        if let Some(pending) = self.requests.get(&device_id) {
            if pending.control_address == control_address {
                return Ok(device_id);
            }
            return Err(DesktopNetworkError::invalid_state(
                "a different TCP peer is already awaiting approval for this listener ID",
            ));
        }
        let snapshot = sink
            .current_snapshot()
            .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
        let request_id = RequestId::new(format!("desktop-join-{}", self.next_sequence))
            .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let invite_code_valid = advertisement.approval_mode != ApprovalMode::InviteCode
            || request.invite_code.as_deref() == snapshot.host_draft.invite_code.as_deref();
        let display_name = request.device.display_name.clone();
        let summary = JoinRequestSummary::new(
            request_id,
            device_id.clone(),
            display_name.clone(),
            TrustState::SessionOnly,
            invite_code_valid,
            received_at,
        )
        .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))?;
        sink.submit_transport_event(CoreTransportEvent::JoinRequested(summary))
            .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
        self.current_peers.insert(device_id.clone(), control_address);
        self.requests.insert(
            device_id.clone(),
            PendingJoinPorts {
                control_address,
                sync_port: request.sync_port,
                audio_port: request.audio_port,
                display_name,
            },
        );
        Ok(device_id)
    }

    pub(super) fn pending_from_another_peer(
        &self,
        device_id: &DeviceId,
        control_address: SocketAddr,
    ) -> bool {
        self.requests
            .get(device_id)
            .is_some_and(|pending| pending.control_address != control_address)
    }

    /// A retired reader cannot clear a newer peer's pending ports or submit
    /// a false disconnect after its replacement has already joined.
    pub(super) fn remove_if_current(
        &mut self,
        device_id: &DeviceId,
        control_address: SocketAddr,
    ) -> bool {
        if self.current_peers.get(device_id) != Some(&control_address) {
            return false;
        }
        self.current_peers.remove(device_id);
        self.requests.remove(device_id);
        true
    }

    /// Removes and returns one pending listener's reported sync/audio ports,
    /// for the caller to authorize datagram routing after a successful join
    /// approval. Returns `None` if the listener never sent a `JoinRequest`
    /// through this projection (e.g. test scaffolding), in which case
    /// authorization is simply skipped rather than treated as an error.
    pub(super) fn take_ports(&mut self, device_id: &DeviceId) -> Option<(u16, u16)> {
        self.requests
            .remove(device_id)
            .map(|ports| (ports.sync_port, ports.audio_port))
    }

    pub(super) fn take_pending_listener(
        &mut self,
        device_id: &DeviceId,
    ) -> Option<(u16, u16, String)> {
        self.requests
            .remove(device_id)
            .map(|pending| (pending.sync_port, pending.audio_port, pending.display_name))
    }
}

#[cfg(test)]
mod generation_tests {
    use super::{PendingJoinPorts, PendingJoinProjection};
    use silent_disco_core::domain::DeviceId;
    use std::net::SocketAddr;

    #[test]
    fn retired_socket_cannot_remove_replacement_pending_join_or_authorized_identity() {
        let device = DeviceId::new("reconnecting-listener").expect("device");
        let old: SocketAddr = "127.0.0.1:43101".parse().expect("old address");
        let new: SocketAddr = "127.0.0.1:43102".parse().expect("new address");
        let mut projection = PendingJoinProjection::new();
        projection.current_peers.insert(device.clone(), new);
        projection.requests.insert(
            device.clone(),
            PendingJoinPorts {
                control_address: new,
                sync_port: 43000,
                audio_port: 43001,
                display_name: "Replacement".to_owned(),
            },
        );
        assert!(projection.pending_from_another_peer(&device, old));
        assert!(!projection.pending_from_another_peer(&device, new));
        assert!(!projection.remove_if_current(&device, old));
        assert_eq!(projection.take_ports(&device), Some((43000, 43001)));
        // Approval consumes pending ports but not the live TCP ownership.
        assert!(!projection.remove_if_current(&device, old));
        assert!(projection.remove_if_current(&device, new));
        assert!(!projection.current_peers.contains_key(&device));
    }

    #[test]
    fn matching_disconnect_removes_pending_join_but_unknown_peer_cannot() {
        let device = DeviceId::new("pending-listener").expect("device");
        let current: SocketAddr = "127.0.0.1:43103".parse().expect("address");
        let mut projection = PendingJoinProjection::new();
        projection.current_peers.insert(device.clone(), current);
        projection.requests.insert(
            device.clone(),
            PendingJoinPorts {
                control_address: current,
                sync_port: 43002,
                audio_port: 43003,
                display_name: "Pending".to_owned(),
            },
        );
        assert!(!projection.remove_if_current(
            &device,
            "127.0.0.1:43104".parse().expect("other address"),
        ));
        assert!(projection.remove_if_current(&device, current));
        assert_eq!(projection.take_ports(&device), None);
    }
}
