//! Typed bridge from the local desktop transport worker to the authoritative Rust actor.

use super::host_join_projection::PendingJoinProjection;
use super::host_pending_handshake::send_pending_hello;
use super::network_error::DesktopNetworkError;
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame, SyncResponse};
use silent_disco_core::runtime::{
    AudioEvent, CoreActorHandle, CoreSnapshot, SessionAdvertisement, SynchronizationSummary,
    TransportEvent as CoreTransportEvent,
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
                ..
            } => {
                let device_id = self
                    .pending
                    .register(request, received_at, advertisement, sink)?;
                Ok(send_pending_hello(node, &device_id, advertisement))
            }
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                frame: ProtocolFrame::SyncRequest(request),
                received_at,
                ..
            } => {
                let response = ProtocolFrame::SyncResponse(SyncResponse {
                    session_id: request.session_id,
                    correlation_id: request.correlation_id,
                    t1_listener_send_elapsed_ms: request.t1_listener_send_elapsed_ms,
                    t2_host_receive_elapsed_ms: received_at,
                    t3_host_send_elapsed_ms: self.clock.now(),
                });
                Ok(node
                    .broadcast_sync(&response)
                    .err()
                    .map(|error| DesktopNetworkError::transport(&error).to_string()))
            }
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::SynchronizationReport(report)),
                ..
            } => {
                // The listener is the only side that ever knows this (see
                // `SynchronizationReport`'s doc comment: the host never sees
                // `t4`) -- this is a straight relay into the same
                // `AudioEvent` the actor already exposes per listener, not a
                // host-side computation. A malformed report (non-finite
                // values from a corrupted/adversarial peer) is reported as a
                // visible warning and otherwise ignored, not fatal to the
                // whole event-processing loop.
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
            RuntimeTransportEvent::PeerDisconnected { peer, error, .. } => {
                let Some(device_id) = peer.device_id else {
                    return Ok(error.map(|error| error.to_string()));
                };
                self.pending.remove(&device_id);
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
