use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, ProtocolFrame};
use crate::runtime::NetworkEndpoint;
use crate::transport::{
    HostTransportNode, ListenerDatagramRoutes, TransportChannel, TransportCounters,
    TransportDelivery, TransportError, TransportErrorKind, TransportEvent, TransportPeer,
};

use super::network::{VirtualListenerRegistration, VirtualTransportNetwork};
use super::support::{
    VirtualWireEvent, encode_wire_frame, network_poisoned, recv_virtual_event, shutting_down,
    to_u32, try_event, try_frame, unauthorized, update_counters,
};

pub(super) struct VirtualHostTransport {
    network: VirtualTransportNetwork,
    endpoint: NetworkEndpoint,
    session_id: SessionId,
    event_receiver: Receiver<VirtualWireEvent>,
    counters: Arc<Mutex<TransportCounters>>,
    shutdown_complete: bool,
}

impl VirtualHostTransport {
    pub(super) fn new(
        network: VirtualTransportNetwork,
        endpoint: NetworkEndpoint,
        session_id: SessionId,
        event_receiver: Receiver<VirtualWireEvent>,
        counters: Arc<Mutex<TransportCounters>>,
    ) -> Self {
        Self {
            network,
            endpoint,
            session_id,
            event_receiver,
            counters,
            shutdown_complete: false,
        }
    }

    fn deliver_control(
        &self,
        target: Option<&DeviceId>,
        message: &ControlMessage,
        authorized_only: bool,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(unauthorized(TransportChannel::Control));
        }
        let frame = ProtocolFrame::Control(message.clone());
        let bytes = encode_wire_frame(&frame, TransportChannel::Control)?;
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
        let encoded_len = bytes.len();
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut bytes_sent = 0_u64;
        for (_, listener) in listeners {
            let peer = TransportPeer {
                device_id: None,
                control_address: SocketAddr::new(self.endpoint.address, self.endpoint.control_port),
            };
            if try_frame(
                &listener.event_sender,
                TransportChannel::Control,
                peer,
                bytes.clone(),
                listener.clock.now(),
            )
            .is_ok()
            {
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
        let bytes = encode_wire_frame(frame, channel)?;
        let encoded_len = bytes.len();
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
            let peer = TransportPeer {
                device_id: None,
                control_address: SocketAddr::new(self.endpoint.address, self.endpoint.control_port),
            };
            if try_frame(
                &listener.event_sender,
                channel,
                peer,
                bytes.clone(),
                listener.clock.now(),
            )
            .is_ok()
            {
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
                // The listener is the recipient of this event, not the
                // host -- stamped with the listener's own clock.
                received_at: listener.clock.now(),
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
