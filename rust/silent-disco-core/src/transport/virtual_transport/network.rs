use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use crate::domain::{DeviceId, SessionId};
use crate::runtime::NetworkEndpoint;
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,
    ListenerTransportNode, TransportChannel, TransportClock, TransportCounters, TransportError,
    TransportErrorKind, TransportEvent, TransportFactory, TransportPeer,
};

use super::host::VirtualHostTransport;
use super::listener::VirtualListenerTransport;
use super::support::{
    VirtualWireEvent, allocate_endpoint, allocate_port, network_poisoned, try_event,
};

#[derive(Clone, Default)]
pub struct VirtualTransportNetwork {
    pub(super) inner: Arc<Mutex<VirtualNetworkState>>,
}

#[derive(Default)]
pub(super) struct VirtualNetworkState {
    pub(super) next_port: u16,
    pub(super) hosts: HashMap<NetworkEndpoint, VirtualHostRegistration>,
}

pub(super) struct VirtualHostRegistration {
    pub(super) session_id: SessionId,
    pub(super) event_sender: SyncSender<VirtualWireEvent>,
    pub(super) clock: Arc<dyn TransportClock>,
    pub(super) counters: Arc<Mutex<TransportCounters>>,
    pub(super) listeners: HashMap<DeviceId, VirtualListenerRegistration>,
}

#[derive(Clone)]
pub(super) struct VirtualListenerRegistration {
    pub(super) event_sender: SyncSender<VirtualWireEvent>,
    pub(super) control_address: SocketAddr,
    pub(super) routes: ListenerDatagramRoutes,
    pub(super) authorized: bool,
    /// This listener's own clock -- every event pushed into its
    /// `event_sender` is stamped with this, not the host's clock,
    /// because `received_at` records when the *recipient* observed the
    /// event. Distinct from the host's clock so a Lab node with its own
    /// configured offset/drift is actually visible in the events it
    /// receives.
    pub(super) clock: Arc<dyn TransportClock>,
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

#[cfg(test)]
impl VirtualTransportNetwork {
    /// Injects one raw protocol frame into a connected listener's virtual
    /// receive queue. This test-only seam is intentionally byte-level: it
    /// proves `ListenerTransportNode::recv_event` performs the production
    /// decode instead of accepting a pre-decoded `ProtocolFrame`.
    pub(crate) fn inject_listener_wire_frame_for_test(
        &self,
        endpoint: NetworkEndpoint,
        device_id: &DeviceId,
        channel: TransportChannel,
        bytes: Vec<u8>,
    ) -> Result<(), TransportError> {
        let state = self.inner.lock().map_err(network_poisoned)?;
        let host = state.hosts.get(&endpoint).ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::Connect,
                TransportChannel::Runtime,
                "virtual host endpoint is not bound on this network",
            )
        })?;
        let listener = host.listeners.get(device_id).ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::PeerNotFound,
                channel,
                "virtual listener is not connected",
            )
        })?;
        super::support::try_frame(
            &listener.event_sender,
            channel,
            TransportPeer {
                device_id: None,
                control_address: SocketAddr::new(endpoint.address, endpoint.control_port),
            },
            bytes,
            listener.clock.now(),
        )
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
                clock,
                counters: counters.clone(),
                listeners: HashMap::new(),
            },
        );
        Ok(Box::new(VirtualHostTransport::new(
            self.network.clone(),
            endpoint,
            config.session_id,
            event_receiver,
            counters,
        )))
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
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
                clock,
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
        Ok(Box::new(VirtualListenerTransport::new(
            self.network.clone(),
            config.endpoint,
            config.session_id,
            config.device_id,
            control_address,
            routes,
            event_receiver,
        )))
    }
}
