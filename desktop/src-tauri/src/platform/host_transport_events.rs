//! Typed bridge from the local desktop transport worker to the authoritative Rust actor.

use super::host_join_projection::PendingJoinProjection;
use super::host_pending_handshake::send_pending_hello;
use super::network_error::DesktopNetworkError;
use silent_disco_core::domain::{DeviceId, TransportState, TrustState};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{
    ControlMessage, JoinRejection, ProtocolFrame, SyncResponse, SynchronizationReport,
};
use silent_disco_core::runtime::{
    AudioEvent, CoreActorHandle, CoreSnapshot, ListenerSummary, SessionAdvertisement,
    SynchronizationSummary, TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportNode, TransportChannel, TransportClock, TransportEvent as RuntimeTransportEvent,
};
use std::sync::Arc;

pub(crate) trait DesktopHostTransportEventSink: Send + Sync + 'static {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError>;
    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError>;
    fn submit_audio_event(&self, event: AudioEvent) -> Result<(), CoreError>;
}

impl DesktopHostTransportEventSink for CoreActorHandle {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        CoreActorHandle::current_snapshot(self)
    }

    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError> {
        CoreActorHandle::submit_transport_event(self, event)
    }

    fn submit_audio_event(&self, event: AudioEvent) -> Result<(), CoreError> {
        CoreActorHandle::submit_audio_event(self, event)
    }
}

pub(crate) struct HostTransportEventProcessor {
    pending: PendingJoinProjection,
    clock: Arc<dyn TransportClock>,
}

impl HostTransportEventProcessor {
    pub(crate) fn new(clock: Arc<dyn TransportClock>) -> Self {
        Self {
            pending: PendingJoinProjection::new(),
            clock,
        }
    }

    /// Removes and returns one pending listener's reported sync/audio
    /// ports, for the caller to authorize datagram routing after a
    /// successful join approval.
    pub(crate) fn take_pending_ports(
        &mut self,
        device_id: &silent_disco_core::domain::DeviceId,
    ) -> Option<(u16, u16)> {
        self.pending.take_ports(device_id)
    }

    pub(crate) fn take_pending_listener(
        &mut self,
        device_id: &silent_disco_core::domain::DeviceId,
    ) -> Option<(u16, u16, String, std::net::SocketAddr)> {
        self.pending.take_pending_listener(device_id)
    }

    pub(crate) fn mark_authorized_listener(
        &mut self,
        device_id: silent_disco_core::domain::DeviceId,
        control_address: std::net::SocketAddr,
    ) {
        self.pending.mark_authorized(device_id, control_address);
    }

    pub(crate) fn connected_listener(
        device_id: silent_disco_core::domain::DeviceId,
        display_name: String,
        trusted_for_future: bool,
    ) -> Result<ListenerSummary, silent_disco_core::runtime::RuntimeRecordValidationError> {
        ListenerSummary::new(
            device_id,
            display_name,
            if trusted_for_future {
                TrustState::Trusted
            } else {
                TrustState::SessionOnly
            },
            TransportState::Connected,
        )
    }

    pub(super) fn process(
        &mut self,
        event: RuntimeTransportEvent,
        node: &dyn HostTransportNode,
        advertisement: &SessionAdvertisement,
        sink: &dyn DesktopHostTransportEventSink,
    ) -> Result<Option<String>, DesktopNetworkError> {
        match event {
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinRequest(request)),
                received_at,
                peer,
            } => {
                // A second unverified TCP socket claiming the same device ID
                // must not replace an in-flight approval's UDP ports. Refuse
                // it explicitly rather than leaving the listener spinning.
                if self
                    .pending
                    .pending_from_another_peer(&request.device.device_id, peer.control_address)
                {
                    return Ok(Some(Self::duplicate_pending_warning(
                        node,
                        advertisement,
                        &request.device.device_id,
                    )));
                }
                // Actor mailbox failure or pending-capacity exhaustion must
                // not terminate the desktop host or strand the listener.
                // Only successfully projected requests receive Hello.
                let device_id = match self.pending.register(
                    &request,
                    peer.control_address,
                    received_at,
                    advertisement,
                    sink,
                ) {
                    Ok(device_id) => device_id,
                    Err(_) => {
                        return Ok(Some(Self::refuse_unprojected_join(
                            node,
                            advertisement,
                            &request.device.device_id,
                        )));
                    }
                };
                Ok(send_pending_hello(node, &device_id, advertisement))
            }
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                frame: ProtocolFrame::SyncRequest(request),
                received_at,
                peer,
            } => {
                let Some(listener_id) = peer.device_id else {
                    return Ok(Some(
                        "Clock synchronization request has no admitted listener identity"
                            .to_owned(),
                    ));
                };
                let response = ProtocolFrame::SyncResponse(SyncResponse {
                    session_id: request.session_id,
                    correlation_id: request.correlation_id,
                    t1_listener_send_elapsed_ms: request.t1_listener_send_elapsed_ms,
                    t2_host_receive_elapsed_ms: received_at,
                    t3_host_send_elapsed_ms: self.clock.now(),
                });
                Ok(node
                    .send_sync_to_peer(&listener_id, &response)
                    .err()
                    .map(|error| DesktopNetworkError::transport(&error).to_string()))
            }
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::ResyncNotice(notice)),
                peer,
                ..
            } => Ok(Self::replay_resync(
                node,
                &notice.listener_id,
                peer.device_id.as_ref(),
            )),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::SynchronizationReport(report)),
                ..
            } => Self::forward_sync_report(report, sink),
            RuntimeTransportEvent::PeerDisconnected { peer, error, .. } => {
                let Some(device_id) = peer.device_id else {
                    return Ok(error.map(|error| error.to_string()));
                };
                if !self
                    .pending
                    .remove_if_current(&device_id, peer.control_address)
                {
                    // Old TCP teardown after a same-ID reconnect is not a
                    // disconnect of the currently projected listener.
                    return Ok(None);
                }
                let visible = error.as_ref().map(ToString::to_string);
                let error =
                    error.map(|error| DesktopNetworkError::transport(&error).core_error(None));
                sink.submit_transport_event(CoreTransportEvent::ListenerDisconnected {
                    device_id,
                    error,
                })
                .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
                Ok(visible)
            }
            RuntimeTransportEvent::Rejected { error, .. } => Ok(Some(error.to_string())),
            RuntimeTransportEvent::PeerAccepted { .. }
            | RuntimeTransportEvent::PeerAuthorized { .. }
            | RuntimeTransportEvent::FrameReceived { .. } => Ok(None),
        }
    }

    /// Relay the listener's clock-quality report to the authoritative actor.
    /// Only the listener observes t4; a malformed/non-finite report must
    /// become a visible warning rather than aborting the host event loop.
    fn forward_sync_report(
        report: SynchronizationReport,
        sink: &dyn DesktopHostTransportEventSink,
    ) -> Result<Option<String>, DesktopNetworkError> {
        match SynchronizationSummary::new(
            report.confidence,
            report.offset_ms,
            report.round_trip_ms,
            report.drift_ppm,
        ) {
            Ok(summary) => {
                sink.submit_audio_event(AudioEvent::SynchronizationUpdated {
                    device_id: report.listener_id,
                    summary,
                })
                .map_err(|error| DesktopNetworkError::invalid_state(error.to_string()))?;
                Ok(None)
            }
            Err(error) => Ok(Some(format!(
                "listener {} sent an invalid synchronization report: {error}",
                report.listener_id.as_str()
            ))),
        }
    }

    /// Replay only the host transport's authoritative controls to the
    /// admitted requester. The shared socket reader already checks identity,
    /// but this platform boundary must not route a forged recovery event.
    fn replay_resync(
        node: &dyn HostTransportNode,
        listener_id: &DeviceId,
        peer_id: Option<&DeviceId>,
    ) -> Option<String> {
        if peer_id != Some(listener_id) {
            return Some("Recovery request identity did not match its TCP peer".to_owned());
        }
        node.replay_cached_playback_to_peer(listener_id)
            .err()
            .map(|error| DesktopNetworkError::transport(&error).to_string())
    }

    /// Explicitly refuse an unprojected generation; the actor has not
    /// accepted its JoinRequested event and no UDP route was retained.
    fn refuse_unprojected_join(
        node: &dyn HostTransportNode,
        advertisement: &SessionAdvertisement,
        device_id: &DeviceId,
    ) -> String {
        let rejection = ControlMessage::JoinRejection(JoinRejection {
            session_id: advertisement.session_id.clone(),
            listener_id: device_id.clone(),
            reason: "host_busy".to_owned(),
        });
        match node.send_pending_control(device_id, &rejection) {
            Ok(delivery) if delivery.report.successful_peers > 0 => {
                "Desktop join refused before actor admission (host busy)".to_owned()
            }
            Ok(_) => "Desktop join refused but no TCP peer received the refusal".to_owned(),
            Err(error) => format!("Desktop join refused; rejection delivery failed: {error}"),
        }
    }

    /// Keep the rejection visible even when sending the refusal fails.
    /// The unverified duplicate never replaces an existing pending route.
    fn duplicate_pending_warning(
        node: &dyn HostTransportNode,
        advertisement: &SessionAdvertisement,
        device_id: &silent_disco_core::domain::DeviceId,
    ) -> String {
        let rejection = ControlMessage::JoinRejection(JoinRejection {
            session_id: advertisement.session_id.clone(),
            listener_id: device_id.clone(),
            reason: "Another connection with this listener ID is awaiting approval".to_owned(),
        });
        let delivery_error = node
            .send_pending_control(device_id, &rejection)
            .err()
            .map(|error| format!("; rejection delivery failed: {error}"))
            .unwrap_or_default();
        format!("Duplicate pending listener identity rejected{delivery_error}")
    }

    /// Runs the exact production host event projection for Lab Mode while
    /// converting the desktop-private adapter error into bounded text at the
    /// module boundary. Lab therefore reuses the real join projection,
    /// pending-port authorization data, Hello response, sync response, and
    /// disconnect handling without exposing `DesktopNetworkError` outside
    /// the production platform module.
    #[cfg(feature = "lab-mode")]
    pub(crate) fn process_for_lab(
        &mut self,
        event: RuntimeTransportEvent,
        node: &dyn HostTransportNode,
        advertisement: &SessionAdvertisement,
        sink: &dyn DesktopHostTransportEventSink,
    ) -> Result<Option<String>, String> {
        self.process(event, node, advertisement, sink)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
pub(super) struct TestTransportEventSink;

#[cfg(test)]
impl DesktopHostTransportEventSink for TestTransportEventSink {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(CoreSnapshot::default())
    }

    fn submit_transport_event(&self, _event: CoreTransportEvent) -> Result<(), CoreError> {
        Ok(())
    }

    fn submit_audio_event(&self, _event: AudioEvent) -> Result<(), CoreError> {
        Ok(())
    }
}
