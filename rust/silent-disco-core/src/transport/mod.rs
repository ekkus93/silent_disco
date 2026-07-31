mod boundary;
mod clock;
mod socket;
mod types;
mod virtual_transport;

#[cfg(test)]
mod tests;

pub use boundary::{HostTransportNode, ListenerTransportNode, TransportFactory};
pub use clock::{ManualTransportClock, SystemTransportClock, TransportClock};
pub use socket::{SocketHostTransport, SocketListenerTransport, SocketTransportFactory};
pub use types::{
    DEFAULT_IO_TIMEOUT, DEFAULT_MAX_CONSECUTIVE_FAILURES, DEFAULT_MAX_TRANSPORT_PEERS,
    DEFAULT_OPERATION_TIMEOUT, DEFAULT_TRANSPORT_EVENT_CAPACITY, DEFAULT_TRANSPORT_QUEUE_CAPACITY,
    HostTransportConfig, ListenerDatagramRoutes, ListenerTransportConfig,
    MAX_TRANSPORT_EVENT_CAPACITY, MAX_TRANSPORT_PEERS, MAX_TRANSPORT_QUEUE_CAPACITY,
    TransportChannel, TransportCounters, TransportDelivery, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
pub use virtual_transport::{VirtualTransportFactory, VirtualTransportNetwork};

/// Returns the production standard-IP transport factory.
///
/// Virtual transport is available only through explicit construction of
/// [`VirtualTransportFactory`]; no production fallback selects it implicitly.
#[must_use]
pub const fn production_transport_factory() -> SocketTransportFactory {
    SocketTransportFactory
}
