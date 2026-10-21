use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::super::super::{
    HostTransportNode, ListenerDatagramRoutes, TransportChannel, TransportCounters,
    TransportDelivery, TransportError, TransportErrorKind, TransportEvent,
};
use super::super::host_workers::shutting_down_error;
use super::super::shared::{join_workers, recv_event};
use super::SocketHostTransport;
use super::peer::PeerState;

impl HostTransportNode for SocketHostTransport {
    fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    fn authorize_peer(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        self.authorize_peer_with_routes(device_id, routes)
    }

    fn authorize_peer_ports(
        &self,
        device_id: &DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(shutting_down_error());
        }
        let peer_ip = {
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
                .map(|peer| peer.remote.ip())
        }
        .ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "no pending control peer matches the requested device",
            )
        })?;
        let routes = ListenerDatagramRoutes {
            synchronization: SocketAddr::new(peer_ip, sync_port),
            audio: SocketAddr::new(peer_ip, audio_port),
        };
        self.authorize_peer_with_routes(device_id, routes)
    }

    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError> {
        let route = self
            .routes
            .lock()
            .map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer route registry is poisoned",
                )
            })?
            .remove(device_id);
        let Some(route) = route else {
            return Err(TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "authorized peer is not connected",
            ));
        };
        route.peer.close()
    }

    fn send_pending_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound pending control message belongs to a different session",
            ));
        }
        // Pending Hello/approval delivery must not interleave with late-join
        // authorization replay. Acquire the ordering guard before looking up
        // the peer so a concurrent authorization cannot insert its route and
        // replay StreamStart ahead of this pending control frame.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self.pending_peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        match result {
            Ok(written) => {
                self.update_silence_clock(&peer, message);
                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
            }
            Err(error) => {
                self.counters.delivery_failure();
                Err(error)
            }
        }
    }

    fn send_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound control message belongs to a different session",
            ));
        }
        // Select the authorized route only after acquiring the replay/delivery
        // guard. A concurrent disconnect may retire the route while a targeted
        // send is waiting here; using an earlier selection could send to a
        // disconnected peer after the delivery order becomes available.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self.peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        match result {
            Ok(written) => {
                self.update_silence_clock(&peer, message);
                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
            }
            Err(error) => {
                self.counters.delivery_failure();
                Err(error)
            }
        }
    }

    fn broadcast_control(
        &self,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound control message belongs to a different session",
            ));
        }
        // Serialize with late-join authorization and its cached replay. Without
        // this guard, a newly registered peer can receive a live boundary
        // before its cached StreamStart, or receive the boundary twice.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        self.playback_controls
            .lock()
            .map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "playback control cache is poisoned",
                )
            })?
            .update(message);
        let peers: Vec<Arc<PeerState>> = self
            .authorized_routes()?
            .into_iter()
            .map(|(_, route)| route.peer)
            .collect();
        let intended = u32::try_from(peers.len()).map_err(|_| {
            TransportError::new(
                TransportErrorKind::Delivery,
                TransportChannel::Control,
                "peer count exceeds delivery accounting range",
            )
        })?;
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut sent_bytes = 0_u64;
        for peer in peers {
            let result = peer.sender.send(bytes.clone());
            self.record_peer_result(&peer, &result);
            match result {
                Ok(written) => {
                    self.update_silence_clock(&peer, message);
                    successful = successful.saturating_add(1);
                    sent_bytes =
                        sent_bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
                }
                Err(_) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                }
            }
        }
        TransportDelivery::new(intended, successful, failed, sent_bytes)
    }

    fn broadcast_sync(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.broadcast_datagram(TransportChannel::Synchronization, frame)
    }

    fn broadcast_audio(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.broadcast_datagram(TransportChannel::Audio, frame)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        recv_event(&self.event_receiver, timeout)
    }

    fn counters(&self) -> TransportCounters {
        self.counters.snapshot()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.stop.store(true, Ordering::Release);
        let mut shutdown_error = None;
        match self.peers.lock() {
            Ok(peers) => {
                for peer in peers.values() {
                    if let Err(error) = peer.close() {
                        shutdown_error.get_or_insert(error);
                    }
                }
            }
            Err(_) => {
                shutdown_error = Some(TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer registry is poisoned during shutdown",
                ));
            }
        }
        let join_result = join_workers(&self.workers);
        self.shutdown_complete = true;
        shutdown_error.map_or(join_result, Err)
    }
}
