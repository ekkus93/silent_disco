use std::sync::atomic::Ordering;

use crate::domain::DeviceId;
use crate::protocol::{ProtocolFrame, encode_frame};

use super::super::super::{
    ListenerDatagramRoutes, TransportChannel, TransportError, TransportErrorKind, TransportEvent,
};
use super::super::host_workers::shutting_down_error;
use super::super::shared::send_event;
use super::SocketHostTransport;
use super::peer::PeerRoute;

impl SocketHostTransport {
    pub(super) fn authorize_peer_with_routes(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(shutting_down_error());
        }
        // A reconnect may arrive before the retired socket's reader exits.
        // Select the newest *pending* connection, never the old authorized
        // peer that still has the same device identity.
        let peer = self.pending_peer_for_device(device_id)?;
        if routes.synchronization.ip() != peer.remote.ip() || routes.audio.ip() != peer.remote.ip()
        {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Runtime,
                "datagram routes must use the authenticated control peer address",
            ));
        }
        // The cache snapshot and route registration must be atomic relative to
        // live control delivery; otherwise a broadcast can race ahead of the
        // replayed StreamStart or be delivered twice.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        // The peer may disconnect while authorization waits behind a cached
        // replay or live control broadcast. Do not publish a dead route or a
        // spurious PeerAuthorized event after the initial pending-peer lookup.
        if !peer.active.load(Ordering::Acquire) {
            return Err(TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "pending control peer disconnected before authorization",
            ));
        }
        // Replay before switching the live route. If the new socket cannot
        // receive the authoritative snapshot, the old TCP/UDP route must
        // remain usable instead of being replaced by a broken connection.
        self.send_cached_playback_controls(&peer)?;
        if !peer.active.load(Ordering::Acquire) {
            return Err(TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "pending control peer disconnected during playback replay",
            ));
        }
        let previous = {
            let mut route_registry = self.routes.lock().map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer route registry is poisoned",
                )
            })?;
            let previous = route_registry.insert(
                device_id.clone(),
                PeerRoute {
                    peer: peer.clone(),
                    routes,
                },
            );
            peer.authorized.store(true, Ordering::Release);
            previous
        };
        // Explicit authorization atomically hands the identity to the new
        // socket. Retire the old writer/reader; its delayed cleanup checks
        // peer number and cannot remove the replacement UDP route.
        if let Some(previous) = previous {
            previous.peer.authorized.store(false, Ordering::Release);
            drop(previous.peer.close());
        }
        send_event(
            &self.event_sender,
            &self.counters,
            TransportEvent::PeerAuthorized {
                peer: peer.transport_peer(),
                routes,
                received_at: self.clock.now(),
            },
        );
        Ok(())
    }

    fn send_cached_playback_controls(
        &self,
        peer: &super::peer::PeerState,
    ) -> Result<(), TransportError> {
        let controls = self
            .playback_controls
            .lock()
            .map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "playback control cache is poisoned",
                )
            })?
            .snapshot();
        for message in controls {
            let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
                .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
            let result = peer.sender.send(bytes);
            self.record_peer_result(peer, &result);
            let written = result?;
            self.counters.frame_sent(written);
            self.update_silence_clock(peer, &message);
        }
        Ok(())
    }
}
