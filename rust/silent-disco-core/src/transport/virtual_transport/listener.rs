use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest, encode_frame};
use crate::runtime::NetworkEndpoint;
use crate::transport::{
    ListenerDatagramRoutes, ListenerTransportNode, TransportChannel, TransportCounters,
    TransportDelivery, TransportError, TransportEvent, TransportPeer,
};

use super::network::VirtualTransportNetwork;
use super::support::{
    network_poisoned, recv_virtual_event, round_trip, shutting_down, try_event, unauthorized,
    update_counters, validate_virtual_listener_identity,
};

pub(super) struct VirtualListenerTransport {
    network: VirtualTransportNetwork,
    endpoint: NetworkEndpoint,
    session_id: SessionId,
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

impl VirtualListenerTransport {
    pub(super) fn new(
        network: VirtualTransportNetwork,
        endpoint: NetworkEndpoint,
        session_id: SessionId,
        device_id: DeviceId,
        control_address: SocketAddr,
        local_routes: ListenerDatagramRoutes,
        event_receiver: Receiver<TransportEvent>,
    ) -> Self {
        Self {
            network,
            endpoint,
            session_id,
            device_id,
            control_address,
            local_routes,
            event_receiver: Mutex::new(event_receiver),
            counters: Arc::new(Mutex::new(TransportCounters::default())),
            shutdown_complete: false,
        }
    }
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
