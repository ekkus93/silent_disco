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
        let peer = {
            let peers = self.peers.lock().map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer registry is poisoned",
                )
            })?;
            peers
                .values()
                .find(|peer| {
                    peer.active.load(Ordering::Acquire)
                        && peer.device_id().as_ref() == Some(device_id)
                })
                .cloned()
        }
        .ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "no pending control peer matches the requested device",
            )
        })?;
        if routes.synchronization.ip() != peer.remote.ip() || routes.audio.ip() != peer.remote.ip()
        {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Runtime,
                "datagram routes must use the authenticated control peer address",
            ));
        }
        let mut route_registry = self.routes.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer route registry is poisoned",
            )
        })?;
        if route_registry.contains_key(device_id) {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "device is already authorized",
            ));
        }
        peer.authorized.store(true, Ordering::Release);
        route_registry.insert(
            device_id.clone(),
            PeerRoute {
                peer: peer.clone(),
                routes,
            },
        );
        drop(route_registry);
        if let Err(error) = self.send_cached_playback_controls(&peer) {
            if let Ok(mut routes) = self.routes.lock() {
                routes.remove(device_id);
            }
            peer.authorized.store(false, Ordering::Release);
            return Err(error);
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
