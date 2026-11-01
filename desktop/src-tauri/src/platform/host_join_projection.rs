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

// Bound pending actor projections independently of TCP accept capacity.
// A stalled actor must not retain an unlimited set of claimed identities.
const MAX_PENDING_JOIN_REQUESTS: usize = 128;

/// A pending join's listener-reported sync/audio UDP ports -- needed once
/// (if) the join is approved, to call `authorize_peer_ports`. The host
/// cannot discover these ports any other way (the listener's audio channel
/// is receive-only and never sends a datagram the host could observe a
/// source address from).
struct PendingJoinPorts {
    request_id: RequestId,
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
        if self.requests.len() >= MAX_PENDING_JOIN_REQUESTS {
            return Err(DesktopNetworkError::invalid_state(
                "desktop pending join capacity exhausted",
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
            request_id.clone(),
            device_id.clone(),
            display_name.clone(),
            TrustState::SessionOnly,
            invite_code_valid,
            received_at,
        )
        .map_err(|error| DesktopNetworkError::invalid_argument(error.to_string()))?;
        // Publish the ports before enqueueing JoinRequested: the actor may
        // consume the event and approve immediately on another thread. An
        // approval must never race ahead of the UDP route it needs.
        // A replacement may be pending while the previous authorized socket
        // remains live; do not transfer ownership until authorization succeeds.
        let inserted_owner = !self.current_peers.contains_key(&device_id);
        self.current_peers
            .entry(device_id.clone())
            .or_insert(control_address);
        self.requests.insert(
            device_id.clone(),
            PendingJoinPorts {
                request_id,
                control_address,
                sync_port: request.sync_port,
                audio_port: request.audio_port,
                display_name,
            },
        );
        if let Err(error) = sink.submit_transport_event(CoreTransportEvent::JoinRequested(summary))
        {
            // The actor did not accept this join. A retry must not inherit
            // ghost pending ports, and a failed replacement must not evict
            // the existing authorized TCP owner.
            self.requests.remove(&device_id);
            if inserted_owner {
                self.current_peers.remove(&device_id);
            }
            return Err(DesktopNetworkError::invalid_state(error.to_string()));
        }
        Ok(device_id)
    }

    /// Actor decisions must belong to the exact TCP join generation that
    /// produced their request ID, not merely the claimed device ID.
    pub(super) fn pending_request_matches(
        &self,
        device_id: &DeviceId,
        request_id: &RequestId,
    ) -> bool {
        self.requests
            .get(device_id)
            .is_some_and(|pending| &pending.request_id == request_id)
    }

    /// Reject delayed events from a retired same-ID TCP generation.
    pub(super) fn owns_peer(&self, device: &DeviceId, address: SocketAddr) -> bool {
        self.current_peers.get(device) == Some(&address)
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
        // A failed replacement join may disconnect while the old authorized
        // socket is still live. Retire only the matching pending request.
        if self
            .requests
            .get(device_id)
            .is_some_and(|pending| pending.control_address == control_address)
        {
            self.requests.remove(device_id);
        }
        if self.current_peers.get(device_id) != Some(&control_address) {
            return false;
        }
        self.current_peers.remove(device_id);
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
    ) -> Option<(u16, u16, String, SocketAddr)> {
        self.requests.remove(device_id).map(|pending| {
            (
                pending.sync_port,
                pending.audio_port,
                pending.display_name,
                pending.control_address,
            )
        })
    }

    /// Transfer ownership only after the native transport has actually
    /// authorized the replacement. Sending an approval frame is insufficient:
    /// UDP route authorization can still fail, and rejection must never
    /// displace a working previous connection.
    pub(super) fn mark_authorized(&mut self, device_id: DeviceId, control_address: SocketAddr) {
        self.current_peers.insert(device_id, control_address);
    }
}

#[cfg(test)]
mod generation_tests {
    use super::{PendingJoinPorts, PendingJoinProjection};
    use silent_disco_core::domain::{DeviceId, RequestId};
    use std::net::SocketAddr;

    #[test]
    fn pending_join_capacity_refuses_new_ids_without_displacing_existing_approvals() {
        use super::MAX_PENDING_JOIN_REQUESTS;
        use crate::platform::host_transport_events::TestTransportEventSink;
        use silent_disco_core::domain::{ApprovalMode, MonotonicMillis, SessionId};
        use silent_disco_core::protocol::{DeviceIdentity, JoinRequest, PROTOCOL_VERSION};
        use silent_disco_core::runtime::SessionAdvertisement;

        let session = SessionId::new("desktop-pending-capacity").expect("session");
        let advertisement = SessionAdvertisement::new(
            session.clone(),
            DeviceId::new("capacity-host").expect("host"),
            "Capacity host",
            ApprovalMode::Manual,
            PROTOCOL_VERSION,
            None,
        )
        .expect("advertisement");
        let mut projection = PendingJoinProjection::new();
        let sink = TestTransportEventSink;
        let make_request = |id: &str| JoinRequest {
            session_id: session.clone(),
            device: DeviceIdentity {
                device_id: DeviceId::new(id).expect("device"),
                display_name: id.to_owned(),
            },
            invite_code: None,
            sync_port: 42000,
            audio_port: 42001,
        };
        for n in 0..MAX_PENDING_JOIN_REQUESTS {
            let id = format!("pending-{n}");
            let address = format!("127.0.0.1:{}", 40000 + n)
                .parse()
                .expect("socket address");
            projection
                .register(
                    &make_request(&id),
                    address,
                    MonotonicMillis::new(1),
                    &advertisement,
                    &sink,
                )
                .expect("capacity slot");
        }
        assert_eq!(projection.requests.len(), MAX_PENDING_JOIN_REQUESTS);
        let overflow = make_request("over-capacity");
        let overflow_address = "127.0.0.1:50000".parse().expect("address");
        assert!(
            projection
                .register(
                    &overflow,
                    overflow_address,
                    MonotonicMillis::new(2),
                    &advertisement,
                    &sink,
                )
                .is_err(),
            "a saturated pending table must not accept an unapprovable join"
        );
        assert_eq!(projection.requests.len(), MAX_PENDING_JOIN_REQUESTS);
        let first = DeviceId::new("pending-0").expect("first");
        assert_eq!(projection.take_ports(&first), Some((42000, 42001)));
        projection
            .register(
                &overflow,
                overflow_address,
                MonotonicMillis::new(3),
                &advertisement,
                &sink,
            )
            .expect("released capacity admits another request");
        assert_eq!(projection.requests.len(), MAX_PENDING_JOIN_REQUESTS);
    }

    #[test]
    fn failed_actor_submission_rolls_back_ports_without_eviction_or_ghost_admission() {
        use crate::platform::host_transport_events::DesktopHostTransportEventSink;
        use silent_disco_core::domain::{ApprovalMode, MonotonicMillis, SessionId};
        use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
        use silent_disco_core::protocol::{DeviceIdentity, JoinRequest, PROTOCOL_VERSION};
        use silent_disco_core::runtime::{
            AudioEvent, CoreSnapshot, SessionAdvertisement, TransportEvent,
        };

        struct RefusingActor;
        impl DesktopHostTransportEventSink for RefusingActor {
            fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
                Ok(CoreSnapshot::default())
            }

            fn submit_transport_event(&self, _event: TransportEvent) -> Result<(), CoreError> {
                Err(CoreError::new(
                    CoreErrorCode::WorkerStopped,
                    "actor mailbox closed",
                    ErrorSeverity::Error,
                    true,
                    None,
                )
                .expect("valid failure"))
            }

            fn submit_audio_event(&self, _event: AudioEvent) -> Result<(), CoreError> {
                Ok(())
            }
        }

        let session = SessionId::new("desktop-actor-refusal").expect("session");
        let advertisement = SessionAdvertisement::new(
            session.clone(),
            DeviceId::new("host-actor-refusal").expect("host"),
            "Host",
            ApprovalMode::Manual,
            PROTOCOL_VERSION,
            None,
        )
        .expect("advertisement");
        let request = JoinRequest {
            session_id: session,
            device: DeviceIdentity {
                device_id: DeviceId::new("returning-listener").expect("device"),
                display_name: "Returning listener".to_owned(),
            },
            invite_code: None,
            sync_port: 42011,
            audio_port: 42012,
        };
        let old: SocketAddr = "127.0.0.1:43111".parse().expect("old");
        let replacement: SocketAddr = "127.0.0.1:43112".parse().expect("replacement");
        let mut projection = PendingJoinProjection::new();
        projection
            .current_peers
            .insert(request.device.device_id.clone(), old);

        for _ in 0..128 {
            assert!(
                projection
                    .register(
                        &request,
                        replacement,
                        MonotonicMillis::new(100),
                        &advertisement,
                        &RefusingActor,
                    )
                    .is_err()
            );
            assert!(projection.requests.is_empty(), "no ghost approval ports");
            assert_eq!(
                projection.current_peers.get(&request.device.device_id),
                Some(&old),
                "failed replacement cannot retire the authorized route"
            );
        }

        // A first-time listener has no prior TCP owner to preserve.
        projection.current_peers.clear();
        assert!(
            projection
                .register(
                    &request,
                    replacement,
                    MonotonicMillis::new(101),
                    &advertisement,
                    &RefusingActor,
                )
                .is_err()
        );
        assert!(projection.requests.is_empty());
        assert!(projection.current_peers.is_empty());
        // A healthy actor can now accept the same identity without hitting
        // the pending capacity or inheriting ports from a failed generation.
        projection
            .register(
                &request,
                replacement,
                MonotonicMillis::new(102),
                &advertisement,
                &crate::platform::host_transport_events::TestTransportEventSink,
            )
            .expect("retry after actor recovery");
        assert_eq!(
            projection.take_pending_listener(&request.device.device_id),
            Some((42011, 42012, "Returning listener".to_owned(), replacement))
        );
    }

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
                request_id: RequestId::new("fixture-request").expect("request ID"),
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
    fn failed_replacement_join_cannot_disconnect_still_authorized_old_socket() {
        let device = DeviceId::new("same-id-reconnect").expect("device");
        let old: SocketAddr = "127.0.0.1:43201".parse().expect("old");
        let replacement: SocketAddr = "127.0.0.1:43202".parse().expect("replacement");
        let mut projection = PendingJoinProjection::new();
        projection.current_peers.insert(device.clone(), old);
        projection.requests.insert(
            device.clone(),
            PendingJoinPorts {
                request_id: RequestId::new("fixture-request").expect("request ID"),
                control_address: replacement,
                sync_port: 43004,
                audio_port: 43005,
                display_name: "Replacement".to_owned(),
            },
        );
        // Rejected replacement disconnects: remove only its pending ports.
        assert!(!projection.remove_if_current(&device, replacement));
        assert_eq!(projection.current_peers.get(&device), Some(&old));
        assert_eq!(projection.take_ports(&device), None);
        assert!(projection.remove_if_current(&device, old));
    }

    #[test]
    fn approved_replacement_ignores_retired_old_socket_disconnect() {
        let device = DeviceId::new("approved-reconnect").expect("device");
        let old: SocketAddr = "127.0.0.1:43203".parse().expect("old");
        let replacement: SocketAddr = "127.0.0.1:43204".parse().expect("replacement");
        let mut projection = PendingJoinProjection::new();
        projection.current_peers.insert(device.clone(), old);
        projection.requests.insert(
            device.clone(),
            PendingJoinPorts {
                request_id: RequestId::new("fixture-request").expect("request ID"),
                control_address: replacement,
                sync_port: 43006,
                audio_port: 43007,
                display_name: "Approved replacement".to_owned(),
            },
        );
        assert_eq!(
            projection.take_pending_listener(&device),
            Some((43006, 43007, "Approved replacement".to_owned(), replacement))
        );
        // The old socket remains current until native authorization succeeds.
        assert_eq!(projection.current_peers.get(&device), Some(&old));
        projection.mark_authorized(device.clone(), replacement);
        assert!(!projection.remove_if_current(&device, old));
        assert_eq!(projection.current_peers.get(&device), Some(&replacement));
        assert!(projection.remove_if_current(&device, replacement));
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
                request_id: RequestId::new("fixture-request").expect("request ID"),
                control_address: current,
                sync_port: 43002,
                audio_port: 43003,
                display_name: "Pending".to_owned(),
            },
        );
        assert!(
            !projection
                .remove_if_current(&device, "127.0.0.1:43104".parse().expect("other address"),)
        );
        assert!(projection.remove_if_current(&device, current));
        assert_eq!(projection.take_ports(&device), None);
    }
}
