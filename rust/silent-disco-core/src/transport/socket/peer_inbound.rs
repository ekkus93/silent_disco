//! Inbound control handling and peer teardown, shared by every control transport.
//!
//! The plaintext TCP reader and the TLS pump both feed decoded outcomes here.
//! Frame validation, the one-join-per-connection rule, busy refusal of a join
//! the actor cannot take, and the ordered terminal disconnect are therefore
//! the same for both paths.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, JoinRejection, ProtocolFrame, encode_frame};

use super::super::{TransportChannel, TransportError, TransportErrorKind, TransportEvent};
use super::host::PeerState;
use super::host_workers::join_delivery::forward_join_or_refuse;
use super::host_workers::remove_route_if_current;
use super::peer_registry::PeerRegistry;
use super::shared::{
    ReadControlOutcome, classify_rejection, is_best_effort_event, send_authoritative_event_bounded,
    send_event, shutdown_stream,
};

pub(super) struct PeerInbound {
    pub(super) peer: Arc<PeerState>,
    registry: PeerRegistry,
}

impl PeerInbound {
    pub(super) fn new(peer: Arc<PeerState>, registry: PeerRegistry) -> Self {
        Self { peer, registry }
    }

    /// Handles one outcome from the transport's reader. Returns `false` when
    /// reading must stop. `terminal_error` records the reason the peer ends,
    /// which the terminal disconnect reports.
    pub(super) fn handle(
        &self,
        outcome: ReadControlOutcome,
        terminal_error: &mut Option<TransportError>,
    ) -> bool {
        let registry = &self.registry;
        let peer = &self.peer;
        match outcome {
            ReadControlOutcome::Frame(frame, bytes) => {
                registry.counters.frame_received(bytes);
                match validate_host_frame(peer, &frame) {
                    Ok(()) => {
                        let received_at = registry.clock.now();
                        peer.mark_inbound_activity(received_at);
                        // A join accepted by the transport reader must
                        // reach the host actor, or receive an explicit
                        // busy rejection. Dropping it from a full
                        // event queue would leave the listener waiting
                        // indefinitely for an approval.
                        let pending_join = match &frame {
                            ProtocolFrame::Control(ControlMessage::JoinRequest(request)) => {
                                Some(request.device.device_id.clone())
                            }
                            _ => None,
                        };
                        // Shared classification protects critical controls from lossy delivery.
                        let event = TransportEvent::FrameReceived {
                            channel: TransportChannel::Control,
                            peer: peer.transport_peer(),
                            frame,
                            received_at,
                        };
                        if pending_join.is_some()
                            && peer.join_forwarded.swap(true, Ordering::AcqRel)
                        {
                            // One accepted join per connection. A
                            // retransmitted JoinRequest must not create
                            // another host-actor approval or enrollment.
                            // A rejected first delivery closes this
                            // connection; a fresh connection may
                            // submit a new request.
                            registry.counters.duplicate_join_suppression();
                            true
                        } else if let Some(listener_id) = pending_join {
                            match forward_join_or_refuse(
                                &registry.event_sender,
                                &registry.counters,
                                event,
                                || {
                                    // A completed writer acknowledgement, not
                                    // merely an attempted send, is a refusal.
                                    let refusal = ProtocolFrame::Control(
                                        ControlMessage::JoinRejection(JoinRejection {
                                            session_id: registry.config.session_id.clone(),
                                            listener_id,
                                            reason:
                                                "Host busy: join queue full; reconnect and retry"
                                                    .to_owned(),
                                        }),
                                    );
                                    let bytes = encode_frame(&refusal).map_err(|error| {
                                        TransportError::protocol(TransportChannel::Control, &error)
                                    })?;
                                    peer.sender.send(bytes).map(|_| ())
                                },
                            ) {
                                Ok(true) => true,
                                Ok(false) => {
                                    *terminal_error = Some(TransportError::new(
                                        TransportErrorKind::QueueFull,
                                        TransportChannel::Control,
                                        "host join queue full; busy rejection delivered",
                                    ));
                                    false
                                }
                                Err(error) => {
                                    // Closed or congested writer: do not claim the
                                    // listener received a retryable busy response.
                                    *terminal_error = Some(error);
                                    false
                                }
                            }
                        } else if is_best_effort_event(&event) {
                            // Drop only best-effort telemetry when the bounded
                            // actor queue is full; critical controls stay ordered.
                            send_event(&registry.event_sender, &registry.counters, event);
                            true
                        } else {
                            // A permanently stopped host actor must not strand
                            // this peer forever behind a full event queue.
                            // Refuse the control by closing its connection;
                            // the authoritative terminal event reports why.
                            let delivered = send_authoritative_event_bounded(
                                &registry.event_sender,
                                &peer.stop,
                                &registry.counters,
                                event,
                                registry.config.operation_timeout,
                            );
                            if !delivered {
                                *terminal_error = Some(TransportError::new(
                                    TransportErrorKind::QueueFull,
                                    TransportChannel::Control,
                                    "host control event delivery timed out; peer disconnected",
                                ));
                            }
                            delivered
                        }
                    }
                    Err(error) => {
                        // A diagnostic Rejected event is lossy under
                        // queue pressure. Preserve the reason in the
                        // authoritative terminal disconnect as well.
                        *terminal_error = Some(error.clone());
                        classify_rejection(&registry.counters, &error);
                        self.report_rejection(error);
                        false
                    }
                }
            }
            ReadControlOutcome::Closed => false,
            ReadControlOutcome::Rejected(error) => {
                *terminal_error = Some(error.clone());
                classify_rejection(&registry.counters, &error);
                self.report_rejection(error);
                false
            }
        }
    }

    fn report_rejection(&self, error: TransportError) {
        send_event(
            &self.registry.event_sender,
            &self.registry.counters,
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                source: self.peer.remote,
                error,
                received_at: self.registry.clock.now(),
            },
        );
    }

    /// Ends the peer after its reader stops: closes the connection, retires its
    /// route, delivers the ordered terminal disconnect, and releases its slot.
    pub(super) fn finish(&self, terminal_error: Option<TransportError>) {
        let registry = &self.registry;
        let peer = &self.peer;
        peer.active.store(false, Ordering::Release);
        peer.stop.store(true, Ordering::Release);
        drop(shutdown_stream(&peer.shutdown_stream));
        // Reserve the peer slot until terminal delivery to bound parked
        // readers under hostile reconnect churn and a stalled actor.
        if let Some(device_id) = peer.device_id() {
            // A replacement connection for the same device may have been
            // authorized while this retired reader was unwinding. Never
            // let its cleanup remove the replacement's live UDP route.
            let _ = remove_route_if_current(&registry.routes, &device_id, peer.number);
        }
        registry.counters.closed();
        // Preserve disconnect without parking the peer worker on a stalled actor.
        if !registry.terminal_mailbox.push(
            TransportEvent::PeerDisconnected {
                peer: peer.transport_peer(),
                error: terminal_error,
                received_at: registry.clock.now(),
            },
            registry.config.max_peers,
        ) {
            // Mailbox overflow violates the joint admission budget.
            registry.counters.delivery_failure();
            registry.counters.authoritative_event_failure();
            registry.counters.mark_worker_panicked();
            registry.stop.store(true, Ordering::Release);
        }
        // Release the admission slot only after terminal delivery.
        drop(
            registry
                .peers
                .lock()
                .ok()
                .and_then(|mut peers| peers.remove(&peer.number)),
        );
    }
}

fn validate_host_frame(peer: &PeerState, frame: &ProtocolFrame) -> Result<(), TransportError> {
    let ProtocolFrame::Control(message) = frame else {
        return Err(TransportError::new(
            TransportErrorKind::Protocol,
            TransportChannel::Control,
            "non-control frame reached host control validation",
        ));
    };
    match message {
        ControlMessage::JoinRequest(request) => {
            let mut identity = peer.identity.lock().map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Control,
                    "peer identity is poisoned",
                )
            })?;
            if let Some(verified) = &peer.verified {
                // A TLS-proven listener is identified by its key alone. The
                // claimed ID must be the one that key maps to, or the join is
                // refused before it reaches the host actor.
                if request.device.device_id != verified.canonical_device_id() {
                    return Err(TransportError::new(
                        TransportErrorKind::Unauthorized,
                        TransportChannel::Control,
                        "claimed device ID does not match the verified listener key",
                    ));
                }
            }
            if identity
                .as_ref()
                .is_some_and(|current| current != &request.device.device_id)
            {
                return Err(TransportError::new(
                    TransportErrorKind::Unauthorized,
                    TransportChannel::Control,
                    "control peer attempted to change device identity",
                ));
            }
            *identity = Some(request.device.device_id.clone());
            Ok(())
        }
        ControlMessage::Heartbeat(value) => require_authorized_identity(peer, &value.listener_id),
        ControlMessage::Disconnect(value) => require_authorized_identity(peer, &value.listener_id),
        ControlMessage::ResyncNotice(value) => {
            require_authorized_identity(peer, &value.listener_id)
        }
        ControlMessage::SynchronizationReport(value) => {
            require_authorized_identity(peer, &value.listener_id)
        }
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::TrackBoundary(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "listener sent a host-only control message",
        )),
    }
}

fn require_authorized_identity(
    peer: &PeerState,
    device_id: &DeviceId,
) -> Result<(), TransportError> {
    if !peer.authorized.load(Ordering::Acquire) || peer.device_id().as_ref() != Some(device_id) {
        return Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "control frame was not sent by the authorized peer identity",
        ));
    }
    Ok(())
}
