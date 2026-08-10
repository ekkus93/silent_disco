use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest, decode_frame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,
    ListenerTransportNode, TransportChannel, TransportClock, TransportCounters, TransportDelivery,
    TransportError, TransportErrorKind, TransportEvent, TransportFactory, TransportPeer,
};

#[derive(Clone, Default)]
pub struct VirtualTransportNetwork {
    inner: Arc<Mutex<VirtualNetworkState>>,
}

#[derive(Default)]
struct VirtualNetworkState {
    next_port: u16,
    hosts: HashMap<NetworkEndpoint, VirtualHostRegistration>,
}

struct VirtualHostRegistration {
    session_id: crate::domain::SessionId,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    counters: Arc<Mutex<TransportCounters>>,
    listeners: HashMap<DeviceId, VirtualListenerRegistration>,
}

#[derive(Clone)]
struct VirtualListenerRegistration {
    event_sender: SyncSender<TransportEvent>,
    control_address: SocketAddr,
    routes: ListenerDatagramRoutes,
    authorized: bool,
}

#[derive(Clone, Default)]
pub struct VirtualTransportFactory {
    network: VirtualTransportNetwork,
}

impl VirtualTransportFactory {
    #[must_use]
    pub fn new(network: VirtualTransportNetwork) -> Self {
        Self { network }
    }

    #[must_use]
    pub fn network(&self) -> VirtualTransportNetwork {
        self.network.clone()
    }
}

impl TransportFactory for VirtualTransportFactory {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        config.validate()?;
        let (event_sender, event_receiver) = sync_channel(config.event_queue_capacity);
        let mut state = self.network.inner.lock().map_err(network_poisoned)?;
        let endpoint = allocate_endpoint(&mut state, config.bind_address)?;
        let counters = Arc::new(Mutex::new(TransportCounters::default()));
        state.hosts.insert(
            endpoint,
            VirtualHostRegistration {
                session_id: config.session_id.clone(),
                event_sender,
                clock: clock.clone(),
                counters: counters.clone(),
                listeners: HashMap::new(),
            },
        );
        Ok(Box::new(VirtualHostTransport {
            network: self.network.clone(),
            endpoint,
            session_id: config.session_id,
            event_receiver,
            counters,
            clock,
            shutdown_complete: false,
        }))
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        _clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        config.validate()?;
        let (event_sender, event_receiver) = sync_channel(config.event_queue_capacity);
        let mut state = self.network.inner.lock().map_err(network_poisoned)?;
        let control_port = allocate_port(&mut state)?;
        let sync_port = allocate_port(&mut state)?;
        let audio_port = allocate_port(&mut state)?;
        let control_address = SocketAddr::new(config.local_address, control_port);
        let routes = ListenerDatagramRoutes {
            synchronization: SocketAddr::new(config.local_address, sync_port),
            audio: SocketAddr::new(config.local_address, audio_port),
        };
        let host = state.hosts.get_mut(&config.endpoint).ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::Connect,
                TransportChannel::Runtime,
                "virtual host endpoint is not bound on this network",
            )
        })?;
        if host.session_id != config.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Runtime,
                "virtual listener session does not match the host session",
            ));
        }
        if host.listeners.contains_key(&config.device_id) {
            return Err(TransportError::new(
                TransportErrorKind::Connect,
                TransportChannel::Control,
                "virtual listener device is already connected",
            ));
        }
        host.listeners.insert(
            config.device_id.clone(),
            VirtualListenerRegistration {
                event_sender,
                control_address,
                routes,
                authorized: false,
            },
        );
        try_event(
            &host.event_sender,
            TransportEvent::PeerAccepted {
                peer: TransportPeer {
                    device_id: None,
                    control_address,
                },
                received_at: host.clock.now(),
            },
        )?;
        Ok(Box::new(VirtualListenerTransport {
            network: self.network.clone(),
            endpoint: config.endpoint,
            session_id: config.session_id,
            device_id: config.device_id,
            control_address,
            local_routes: routes,
            event_receiver: Mutex::new(event_receiver),
            counters: Arc::new(Mutex::new(TransportCounters::default())),
            shutdown_complete: false,
        }))
    }
}

struct VirtualHostTransport {
    network: VirtualTransportNetwork,
    endpoint: NetworkEndpoint,
    session_id: crate::domain::SessionId,
    event_receiver: Receiver<TransportEvent>,
    counters: Arc<Mutex<TransportCounters>>,
    clock: Arc<dyn TransportClock>,
    shutdown_complete: bool,
}

impl VirtualHostTransport {
    fn deliver_control(
        &self,
        target: Option<&DeviceId>,
        message: &ControlMessage,
        authorized_only: bool,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(unauthorized(TransportChannel::Control));
        }
        let frame = round_trip(
            &ProtocolFrame::Control(message.clone()),
            TransportChannel::Control,
        )?;
        let state = self.network.inner.lock().map_err(network_poisoned)?;
        let host = state.hosts.get(&self.endpoint).ok_or_else(shutting_down)?;
        let listeners: Vec<(&DeviceId, &VirtualListenerRegistration)> = host
            .listeners
            .iter()
            .filter(|(device_id, listener)| {
                (!authorized_only || listener.authorized)
                    && target.is_none_or(|target| target == *device_id)
            })
            .collect();
        if target.is_some() && listeners.is_empty() {
            return Err(TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                if authorized_only {
                    "virtual authorized peer is not connected"
                } else {
                    "virtual identified pending peer is not connected"
                },
            ));
        }
        let intended = to_u32(listeners.len(), TransportChannel::Control)?;
        let encoded_len = encode_frame(&frame)
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?
            .len();
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut bytes_sent = 0_u64;
        for (_, listener) in listeners {
            let event = TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                peer: TransportPeer {
                    device_id: None,
                    control_address: SocketAddr::new(
                        self.endpoint.address,
                        self.endpoint.control_port,
                    ),
                },
                frame: frame.clone(),
                received_at: self.clock.now(),
            };
            if try_event(&listener.event_sender, event).is_ok() {
                successful = successful.saturating_add(1);
                bytes_sent =
                    bytes_sent.saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
            } else {
                failed = failed.saturating_add(1);
            }
        }
        update_counters(&self.counters, |counters| {
            counters.control_frames_sent = counters
                .control_frames_sent
                .saturating_add(u64::from(successful));
            counters.bytes_sent = counters.bytes_sent.saturating_add(bytes_sent);
            counters.delivery_failures =
                counters.delivery_failures.saturating_add(u64::from(failed));
        })?;
        TransportDelivery::new(intended, successful, failed, bytes_sent)
    }

    fn deliver_datagram(
        &self,
        channel: TransportChannel,
        frame: &ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        if frame.session_id() != &self.session_id {
            return Err(unauthorized(channel));
        }
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
                    "virtual frame does not belong on the requested channel",
                ));
            }
        }
        let frame = round_trip(frame, channel)?;
        let encoded_len = encode_frame(&frame)
            .map_err(|error| TransportError::protocol(channel, &error))?
            .len();
        let state = self.network.inner.lock().map_err(network_poisoned)?;
        let host = state.hosts.get(&self.endpoint).ok_or_else(shutting_down)?;
        let listeners: Vec<&VirtualListenerRegistration> = host
            .listeners
            .values()
            .filter(|listener| listener.authorized)
            .collect();
        let intended = to_u32(listeners.len(), channel)?;
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut bytes_sent = 0_u64;
        for listener in listeners {
            let event = TransportEvent::FrameReceived {
                channel,
                peer: TransportPeer {
                    device_id: None,
                    control_address: SocketAddr::new(
                        self.endpoint.address,
                        self.endpoint.control_port,
                    ),
                },
                frame: frame.clone(),
                received_at: self.clock.now(),
            };
            if try_event(&listener.event_sender, event).is_ok() {
                successful = successful.saturating_add(1);
                bytes_sent =
                    bytes_sent.saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
            } else {
                failed = failed.saturating_add(1);
            }
        }
        update_counters(&self.counters, |counters| {
            match channel {
                TransportChannel::Synchronization => {
                    counters.sync_datagrams_sent = counters
                        .sync_datagrams_sent
                        .saturating_add(u64::from(successful));
                }
                TransportChannel::Audio => {
                    counters.audio_datagrams_sent = counters
                        .audio_datagrams_sent
                        .saturating_add(u64::from(successful));
                }
                TransportChannel::Control | TransportChannel::Runtime => {}
            }
            counters.bytes_sent = counters.bytes_sent.saturating_add(bytes_sent);
            counters.delivery_failures =
                counters.delivery_failures.saturating_add(u64::from(failed));
        })?;
        TransportDelivery::new(intended, successful, failed, bytes_sent)
    }
}

impl HostTransportNode for VirtualHostTransport {
    fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    fn authorize_peer(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        let mut state = self.network.inner.lock().map_err(network_poisoned)?;
        let host = state
            .hosts
            .get_mut(&self.endpoint)
            .ok_or_else(shutting_down)?;
        let event_sender = host.event_sender.clone();
        let received_at = host.clock.now();
        let control_address = {
            let listener = host.listeners.get_mut(device_id).ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::PeerNotFound,
                    TransportChannel::Control,
                    "virtual pending peer is not connected",
                )
            })?;
            if listener.routes != routes {
                return Err(TransportError::new(
                    TransportErrorKind::Unauthorized,
                    TransportChannel::Runtime,
                    "virtual datagram routes do not match the connected listener",
                ));
            }
            listener.authorized = true;
            listener.control_address
        };
        drop(state);
        try_event(
            &event_sender,
            TransportEvent::PeerAuthorized {
                peer: TransportPeer {
                    device_id: Some(device_id.clone()),
                    control_address,
                },
                routes,
                received_at,
            },
        )
    }

    fn authorize_peer_ports(
        &self,
        device_id: &DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError> {
        let control_address = {
            let mut state = self.network.inner.lock().map_err(network_poisoned)?;
            let host = state
                .hosts
                .get_mut(&self.endpoint)
                .ok_or_else(shutting_down)?;
            let listener = host.listeners.get(device_id).ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::PeerNotFound,
                    TransportChannel::Control,
                    "virtual pending peer is not connected",
                )
            })?;
            listener.control_address
        };
        let routes = ListenerDatagramRoutes {
            synchronization: SocketAddr::new(control_address.ip(), sync_port),
            audio: SocketAddr::new(control_address.ip(), audio_port),
        };
        self.authorize_peer(device_id, routes)
    }

    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError> {
        let mut state = self.network.inner.lock().map_err(network_poisoned)?;
        let host = state
            .hosts
            .get_mut(&self.endpoint)
            .ok_or_else(shutting_down)?;
        let listener = host.listeners.remove(device_id).ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "virtual peer is not connected",
            )
        })?;
        try_event(
            &listener.event_sender,
            TransportEvent::PeerDisconnected {
                peer: TransportPeer {
                    device_id: None,
                    control_address: SocketAddr::new(
                        self.endpoint.address,
                        self.endpoint.control_port,
                    ),
                },
                error: None,
                received_at: self.clock.now(),
            },
        )
    }

    fn send_pending_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.deliver_control(Some(device_id), message, false)
    }

    fn send_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.deliver_control(Some(device_id), message, true)
    }

    fn broadcast_control(
        &self,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.deliver_control(None, message, true)
    }

    fn broadcast_sync(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.deliver_datagram(TransportChannel::Synchronization, frame)
    }

    fn broadcast_audio(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.deliver_datagram(TransportChannel::Audio, frame)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        recv_virtual_event(&self.event_receiver, timeout)
    }

    fn counters(&self) -> TransportCounters {
        self.counters
            .lock()
            .map_or_else(|_| TransportCounters::default(), |value| *value)
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.network
            .inner
            .lock()
            .map_err(network_poisoned)?
            .hosts
            .remove(&self.endpoint);
        self.shutdown_complete = true;
        Ok(())
    }
}

impl Drop for VirtualHostTransport {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}

struct VirtualListenerTransport {
    network: VirtualTransportNetwork,
    endpoint: NetworkEndpoint,
    session_id: crate::domain::SessionId,
    device_id: DeviceId,
    control_address: SocketAddr,
    local_routes: ListenerDatagramRoutes,
    /// The receiver has its own lock, held only for the duration of a
    /// receive. It is deliberately *not* the lock the send methods use:
    /// `recv_event` takes `&self` so a poll parked here cannot delay a
    /// concurrent send, which is what previously made a clock-sync probe's
    /// measured round trip include the poll's own wait.
    event_receiver: Mutex<Receiver<TransportEvent>>,
    counters: Arc<Mutex<TransportCounters>>,
    shutdown_complete: bool,
}

impl ListenerTransportNode for VirtualListenerTransport {
    fn local_routes(&self) -> ListenerDatagramRoutes {
        self.local_routes
    }

    fn send_control(&self, message: &ControlMessage) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(unauthorized(TransportChannel::Control));
        }
        validate_virtual_listener_identity(&self.device_id, message)?;
        let frame = round_trip(
            &ProtocolFrame::Control(message.clone()),
            TransportChannel::Control,
        )?;
        let encoded_len = encode_frame(&frame)
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?
            .len();
        let state = self.network.inner.lock().map_err(network_poisoned)?;
        let host = state.hosts.get(&self.endpoint).ok_or_else(shutting_down)?;
        let listener = host
            .listeners
            .get(&self.device_id)
            .ok_or_else(shutting_down)?;
        if !matches!(message, ControlMessage::JoinRequest(_)) && !listener.authorized {
            return Err(unauthorized(TransportChannel::Control));
        }
        try_event(
            &host.event_sender,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                peer: TransportPeer {
                    device_id: Some(self.device_id.clone()),
                    control_address: self.control_address,
                },
                frame,
                received_at: host.clock.now(),
            },
        )?;
        update_counters(&host.counters, |counters| {
            counters.control_frames_received = counters.control_frames_received.saturating_add(1);
            counters.bytes_received = counters
                .bytes_received
                .saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
        })?;
        update_counters(&self.counters, |counters| {
            counters.control_frames_sent = counters.control_frames_sent.saturating_add(1);
            counters.bytes_sent = counters
                .bytes_sent
                .saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
        })?;
        TransportDelivery::new(1, 1, 0, u64::try_from(encoded_len).unwrap_or(u64::MAX))
    }

    fn send_sync_request(
        &self,
        request: &SyncRequest,
    ) -> Result<TransportDelivery, TransportError> {
        if request.session_id != self.session_id {
            return Err(unauthorized(TransportChannel::Synchronization));
        }
        let frame = round_trip(
            &ProtocolFrame::SyncRequest(request.clone()),
            TransportChannel::Synchronization,
        )?;
        let encoded_len = encode_frame(&frame)
            .map_err(|error| TransportError::protocol(TransportChannel::Synchronization, &error))?
            .len();
        let state = self.network.inner.lock().map_err(network_poisoned)?;
        let host = state.hosts.get(&self.endpoint).ok_or_else(shutting_down)?;
        let listener = host
            .listeners
            .get(&self.device_id)
            .ok_or_else(shutting_down)?;
        if !listener.authorized {
            return Err(unauthorized(TransportChannel::Synchronization));
        }
        try_event(
            &host.event_sender,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                peer: TransportPeer {
                    device_id: Some(self.device_id.clone()),
                    control_address: self.control_address,
                },
                frame,
                received_at: host.clock.now(),
            },
        )?;
        update_counters(&host.counters, |counters| {
            counters.sync_datagrams_received = counters.sync_datagrams_received.saturating_add(1);
            counters.bytes_received = counters
                .bytes_received
                .saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
        })?;
        update_counters(&self.counters, |counters| {
            counters.sync_datagrams_sent = counters.sync_datagrams_sent.saturating_add(1);
            counters.bytes_sent = counters
                .bytes_sent
                .saturating_add(u64::try_from(encoded_len).unwrap_or(u64::MAX));
        })?;
        TransportDelivery::new(1, 1, 0, u64::try_from(encoded_len).unwrap_or(u64::MAX))
    }

    fn recv_event(&self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let receiver = self
            .event_receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recv_virtual_event(&receiver, timeout)
    }

    fn counters(&self) -> TransportCounters {
        self.counters
            .lock()
            .map_or_else(|_| TransportCounters::default(), |value| *value)
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown_complete {
            return Ok(());
        }
        let mut state = self.network.inner.lock().map_err(network_poisoned)?;
        if let Some(host) = state.hosts.get_mut(&self.endpoint) {
            host.listeners.remove(&self.device_id);
        }
        self.shutdown_complete = true;
        Ok(())
    }
}

impl Drop for VirtualListenerTransport {
    fn drop(&mut self) {
        drop(self.shutdown());
    }
}

fn allocate_endpoint(
    state: &mut VirtualNetworkState,
    requested_address: IpAddr,
) -> Result<NetworkEndpoint, TransportError> {
    let address = if requested_address.is_unspecified() {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        requested_address
    };
    let control_port = allocate_port(state)?;
    let sync_port = allocate_port(state)?;
    let audio_port = allocate_port(state)?;
    NetworkEndpoint::new(address, control_port, sync_port, audio_port).map_err(|error| {
        TransportError::new(
            TransportErrorKind::Bind,
            TransportChannel::Runtime,
            error.to_string(),
        )
    })
}

fn allocate_port(state: &mut VirtualNetworkState) -> Result<u16, TransportError> {
    let current = state.next_port.max(20_000);
    let next = current.checked_add(1).ok_or_else(|| {
        TransportError::new(
            TransportErrorKind::Bind,
            TransportChannel::Runtime,
            "virtual transport port space is exhausted",
        )
    })?;
    state.next_port = next;
    Ok(current)
}

fn round_trip(
    frame: &ProtocolFrame,
    channel: TransportChannel,
) -> Result<ProtocolFrame, TransportError> {
    let bytes = encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
    decode_frame(&bytes).map_err(|error| TransportError::protocol(channel, &error))
}

fn try_event(
    sender: &SyncSender<TransportEvent>,
    event: TransportEvent,
) -> Result<(), TransportError> {
    match sender.try_send(event) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(TransportError::new(
            TransportErrorKind::QueueFull,
            TransportChannel::Runtime,
            "virtual transport event queue is full",
        )),
        Err(TrySendError::Disconnected(_)) => Err(shutting_down()),
    }
}

fn recv_virtual_event(
    receiver: &Receiver<TransportEvent>,
    timeout: Duration,
) -> Result<TransportEvent, TransportError> {
    match receiver.recv_timeout(timeout) {
        Ok(event) => Ok(event),
        Err(RecvTimeoutError::Timeout) => Err(TransportError::new(
            TransportErrorKind::Timeout,
            TransportChannel::Runtime,
            "timed out waiting for a virtual transport event",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(shutting_down()),
    }
}

fn update_counters(
    counters: &Mutex<TransportCounters>,
    update: impl FnOnce(&mut TransportCounters),
) -> Result<(), TransportError> {
    let mut counters = counters.lock().map_err(|_| {
        TransportError::new(
            TransportErrorKind::WorkerPanicked,
            TransportChannel::Runtime,
            "virtual transport counters are poisoned",
        )
    })?;
    update(&mut counters);
    Ok(())
}

fn validate_virtual_listener_identity(
    device_id: &DeviceId,
    message: &ControlMessage,
) -> Result<(), TransportError> {
    let matches = match message {
        ControlMessage::JoinRequest(value) => &value.device.device_id == device_id,
        ControlMessage::Heartbeat(value) => &value.listener_id == device_id,
        ControlMessage::Disconnect(value) => &value.listener_id == device_id,
        ControlMessage::ResyncNotice(value) => &value.listener_id == device_id,
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => false,
    };
    if matches {
        Ok(())
    } else {
        Err(unauthorized(TransportChannel::Control))
    }
}

fn to_u32(value: usize, channel: TransportChannel) -> Result<u32, TransportError> {
    u32::try_from(value).map_err(|_| {
        TransportError::new(
            TransportErrorKind::Delivery,
            channel,
            "peer count exceeds delivery accounting range",
        )
    })
}

fn unauthorized(channel: TransportChannel) -> TransportError {
    TransportError::new(
        TransportErrorKind::Unauthorized,
        channel,
        "transport frame is not authorized for this session or peer",
    )
}

fn shutting_down() -> TransportError {
    TransportError::new(
        TransportErrorKind::ShuttingDown,
        TransportChannel::Runtime,
        "virtual transport node is not available",
    )
}

fn network_poisoned<T>(_: std::sync::PoisonError<T>) -> TransportError {
    TransportError::new(
        TransportErrorKind::WorkerPanicked,
        TransportChannel::Runtime,
        "virtual transport network is poisoned",
    )
}
