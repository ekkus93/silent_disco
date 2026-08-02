#![allow(clippy::missing_errors_doc)]
use std::sync::Arc;
use std::time::Duration;

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest};
use crate::runtime::NetworkEndpoint;

use super::{
    HostTransportConfig, ListenerDatagramRoutes, ListenerTransportConfig, TransportClock,
    TransportCounters, TransportDelivery, TransportError, TransportEvent,
};

pub trait HostTransportNode: Send {
    fn endpoint(&self) -> NetworkEndpoint;
    fn authorize_peer(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError>;
    /// Authorizes a peer for datagram routing using only its reported
    /// sync/audio ports, resolving the IP from the already-authenticated
    /// control peer address instead of requiring the caller to supply and
    /// re-validate it. Intended for callers (like the `UniFFI` boundary) that
    /// only learn a listener's ports (e.g. from `JoinRequest`), never a
    /// `SocketAddr` they could construct and validate themselves.
    fn authorize_peer_ports(
        &self,
        device_id: &DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError>;
    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError>;
    /// Sends one control message to an identified TCP peer before datagram authorization.
    ///
    /// This is intentionally narrower than `send_control`: it supports the pre-approval
    /// `Hello` exchange without granting synchronization or audio routes.
    fn send_pending_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError>;
    fn send_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError>;
    fn broadcast_control(
        &self,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError>;
    fn broadcast_sync(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError>;
    fn broadcast_audio(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError>;
    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError>;
    fn counters(&self) -> TransportCounters;
    fn shutdown(&mut self) -> Result<(), TransportError>;
}

pub trait ListenerTransportNode: Send {
    fn local_routes(&self) -> ListenerDatagramRoutes;
    fn send_control(&self, message: &ControlMessage) -> Result<TransportDelivery, TransportError>;
    fn send_sync_request(&self, request: &SyncRequest)
    -> Result<TransportDelivery, TransportError>;
    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError>;
    fn counters(&self) -> TransportCounters;
    fn shutdown(&mut self) -> Result<(), TransportError>;
}

pub trait TransportFactory: Send + Sync {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError>;

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError>;
}
