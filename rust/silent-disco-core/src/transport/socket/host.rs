#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::single_match_else
)]
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, ProtocolFrame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::super::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, TransportChannel,
    TransportClock, TransportCounters, TransportDelivery, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
use super::host_workers::{
    shutting_down_error, spawn_accept_loop, spawn_udp_receiver, worker_registry_error,
};
use super::shared::{
    ControlSender, CounterState, join_workers, recv_event, send_event, shutdown_stream,
};

pub(super) struct PeerState {
    pub(super) number: u64,
    pub(super) remote: SocketAddr,
    pub(super) identity: Mutex<Option<DeviceId>>,
    pub(super) authorized: AtomicBool,
    pub(super) active: AtomicBool,
    pub(super) consecutive_failures: AtomicU32,
    pub(super) sender: ControlSender,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) shutdown_stream: Mutex<TcpStream>,
}

impl PeerState {
    pub(super) fn transport_peer(&self) -> TransportPeer {
        let device_id = self
            .identity
            .lock()
            .ok()
            .and_then(|identity| identity.clone());
        TransportPeer {
            device_id,
            control_address: self.remote,
        }
    }

    pub(super) fn device_id(&self) -> Option<DeviceId> {
        self.identity
            .lock()
            .ok()
            .and_then(|identity| identity.clone())
    }

    pub(super) fn close(&self) -> Result<(), TransportError> {
        self.active.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        shutdown_stream(&self.shutdown_stream)
    }
}

#[derive(Clone)]
pub(super) struct PeerRoute {
    pub(super) peer: Arc<PeerState>,
    pub(super) routes: ListenerDatagramRoutes,
}

pub struct SocketHostTransport {
    endpoint: NetworkEndpoint,
    session_id: SessionId,
    config: HostTransportConfig,
    pub(super) stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    peers: Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    sync_socket: Arc<UdpSocket>,
    audio_socket: Arc<UdpSocket>,
    event_sender: SyncSender<TransportEvent>,
    event_receiver: Receiver<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    shutdown_complete: bool,
}

impl SocketHostTransport {
    pub fn bind(
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, TransportError> {
        config.validate()?;
        let listener = TcpListener::bind(SocketAddr::new(config.bind_address, config.control_port))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Bind,
                    TransportChannel::Control,
                    "failed to bind TCP control listener",
                    &error,
                )
            })?;
        listener.set_nonblocking(true).map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to configure TCP control listener",
                &error,
            )
        })?;
        let sync_socket = Arc::new(
            UdpSocket::bind(SocketAddr::new(config.bind_address, config.sync_port)).map_err(
                |error| {
                    TransportError::io(
                        TransportErrorKind::Bind,
                        TransportChannel::Synchronization,
                        "failed to bind UDP synchronization endpoint",
                        &error,
                    )
                },
            )?,
        );
        let audio_socket = Arc::new(
            UdpSocket::bind(SocketAddr::new(config.bind_address, config.audio_port)).map_err(
                |error| {
                    TransportError::io(
                        TransportErrorKind::Bind,
                        TransportChannel::Audio,
                        "failed to bind UDP audio endpoint",
                        &error,
                    )
                },
            )?,
        );
        sync_socket
            .set_read_timeout(Some(config.io_timeout))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Synchronization,
                    "failed to configure synchronization read timeout",
                    &error,
                )
            })?;
        audio_socket
            .set_read_timeout(Some(config.io_timeout))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Audio,
                    "failed to configure audio read timeout",
                    &error,
                )
            })?;

        let control_address = listener.local_addr().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to query bound control address",
                &error,
            )
        })?;
        let sync_address = sync_socket.local_addr().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Synchronization,
                "failed to query bound synchronization address",
                &error,
            )
        })?;
        let audio_address = audio_socket.local_addr().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Audio,
                "failed to query bound audio address",
                &error,
            )
        })?;
        let endpoint = NetworkEndpoint::new(
            control_address.ip(),
            control_address.port(),
            sync_address.port(),
            audio_address.port(),
        )
        .map_err(|error| {
            TransportError::new(
                TransportErrorKind::Bind,
                TransportChannel::Runtime,
                error.to_string(),
            )
        })?;

        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CounterState::default());
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let (event_sender, event_receiver) = sync_channel(config.event_queue_capacity);
        let next_peer = Arc::new(AtomicU64::new(1));

        let accept_handle = spawn_accept_loop(
            listener,
            config.clone(),
            stop.clone(),
            counters.clone(),
            peers.clone(),
            routes.clone(),
            workers.clone(),
            event_sender.clone(),
            clock.clone(),
            next_peer,
        )?;
        workers
            .lock()
            .map_err(worker_registry_error)?
            .push(accept_handle);

        let sync_handle = spawn_udp_receiver(
            sync_socket.clone(),
            TransportChannel::Synchronization,
            config.session_id.clone(),
            stop.clone(),
            counters.clone(),
            routes.clone(),
            event_sender.clone(),
            clock.clone(),
            config.io_timeout,
        )?;
        workers
            .lock()
            .map_err(worker_registry_error)?
            .push(sync_handle);

        let event_sender_for_runtime = event_sender.clone();
        let clock_for_runtime = clock.clone();
        let audio_handle = spawn_udp_receiver(
            audio_socket.clone(),
            TransportChannel::Audio,
            config.session_id.clone(),
            stop.clone(),
            counters.clone(),
            routes.clone(),
            event_sender,
            clock,
            config.io_timeout,
        )?;
        workers
            .lock()
            .map_err(worker_registry_error)?
            .push(audio_handle);

        Ok(Self {
            endpoint,
            session_id: config.session_id.clone(),
            config,
            stop,
            counters,
            peers,
            routes,
            sync_socket,
            audio_socket,
            event_sender: event_sender_for_runtime,
            event_receiver,
            clock: clock_for_runtime,
            workers,
            shutdown_complete: false,
        })
    }

    fn pending_peer_for_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Arc<PeerState>, TransportError> {
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
                peer.active.load(Ordering::Acquire) && peer.device_id().as_ref() == Some(device_id)
            })
            .cloned()
            .ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::PeerNotFound,
                    TransportChannel::Control,
                    "identified pending control peer is not connected",
                )
            })
    }

    fn peer_for_device(&self, device_id: &DeviceId) -> Result<Arc<PeerState>, TransportError> {
        let routes = self.routes.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer route registry is poisoned",
            )
        })?;
        routes
            .get(device_id)
            .map(|route| route.peer.clone())
            .filter(|peer| peer.active.load(Ordering::Acquire))
            .ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::PeerNotFound,
                    TransportChannel::Control,
                    "authorized peer is not connected",
                )
            })
    }

    fn authorized_routes(&self) -> Result<Vec<(DeviceId, PeerRoute)>, TransportError> {
        let routes = self.routes.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer route registry is poisoned",
            )
        })?;
        Ok(routes
            .iter()
            .filter(|(_, route)| route.peer.active.load(Ordering::Acquire))
            .map(|(device_id, route)| (device_id.clone(), route.clone()))
            .collect())
    }

    fn record_peer_result(&self, peer: &PeerState, result: &Result<usize, TransportError>) {
        if result.is_ok() {
            peer.consecutive_failures.store(0, Ordering::Release);
            return;
        }
        let failures = peer
            .consecutive_failures
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        if failures >= self.config.max_consecutive_failures {
            drop(peer.close());
        }
    }

    fn broadcast_datagram(
        &self,
        channel: TransportChannel,
        frame: &ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        match (channel, frame) {
            (
                TransportChannel::Synchronization,
                ProtocolFrame::SyncRequest(_) | ProtocolFrame::SyncResponse(_),
            )
            | (TransportChannel::Audio, ProtocolFrame::Audio(_)) => {}
            _ => {
                return Err(TransportError::new(
                    TransportErrorKind::Protocol,
                    channel,
                    "protocol frame does not belong on the requested datagram channel",
                ));
            }
        }
        if frame.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "outbound datagram belongs to a different session",
            ));
        }
        let bytes =
            encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
        let routes = self.authorized_routes()?;
        let intended = u32::try_from(routes.len()).map_err(|_| {
            TransportError::new(
                TransportErrorKind::Delivery,
                channel,
                "peer count exceeds delivery accounting range",
            )
        })?;
        let socket = match channel {
            TransportChannel::Synchronization => &self.sync_socket,
            TransportChannel::Audio => &self.audio_socket,
            TransportChannel::Control | TransportChannel::Runtime => {
                return Err(TransportError::new(
                    TransportErrorKind::InvalidConfiguration,
                    channel,
                    "datagram delivery requires a datagram channel",
                ));
            }
        };
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut sent_bytes = 0_u64;
        for (_, route) in routes {
            let destination = match channel {
                TransportChannel::Synchronization => route.routes.synchronization,
                TransportChannel::Audio => route.routes.audio,
                TransportChannel::Control | TransportChannel::Runtime => {
                    return Err(TransportError::new(
                        TransportErrorKind::InvalidConfiguration,
                        channel,
                        "datagram route requires a datagram channel",
                    ));
                }
            };
            match socket.send_to(&bytes, destination) {
                Ok(written) if written == bytes.len() => {
                    successful = successful.saturating_add(1);
                    sent_bytes =
                        sent_bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
                    self.counters.datagram_sent(channel, written);
                    route.peer.consecutive_failures.store(0, Ordering::Release);
                }
                Ok(_) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                    self.record_peer_result(
                        &route.peer,
                        &Err(TransportError::new(
                            TransportErrorKind::Delivery,
                            channel,
                            "datagram send reported a partial write",
                        )),
                    );
                }
                Err(error) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                    self.record_peer_result(
                        &route.peer,
                        &Err(TransportError::io(
                            TransportErrorKind::Delivery,
                            channel,
                            "failed to send datagram",
                            &error,
                        )),
                    );
                }
            }
        }
        TransportDelivery::new(intended, successful, failed, sent_bytes)
    }
}

impl HostTransportNode for SocketHostTransport {
    fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    fn authorize_peer(
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
        let peer = self.pending_peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        match result {
            Ok(written) => {
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
        let peer = self.peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        match result {
            Ok(written) => {
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
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
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

impl Drop for SocketHostTransport {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}
