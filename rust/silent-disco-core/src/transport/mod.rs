mod boundary;
mod clock;
pub mod control_protocol;
mod manual_endpoint;
#[cfg(feature = "quic")]
pub mod quic_admission;
#[cfg(all(test, feature = "quic"))]
mod quic_admission_tests;
#[cfg(feature = "quic")]
pub mod quic_datagram;
#[cfg(all(test, feature = "quic"))]
mod quic_datagram_tests;
#[cfg(feature = "quic")]
pub mod quic_framing;
#[cfg(feature = "quic")]
mod quic_host_node;
#[cfg(feature = "quic")]
pub mod quic_host_transport;
#[cfg(all(test, feature = "quic"))]
mod quic_host_transport_tests;
#[cfg(feature = "quic")]
pub mod quic_host_worker;
#[cfg(feature = "quic")]
pub mod quic_listener_transport;
#[cfg(all(test, feature = "quic"))]
mod quic_listener_transport_tests;
#[cfg(feature = "quic")]
pub mod quic_listener_worker;
#[cfg(all(test, feature = "quic"))]
mod quic_reconnect_tests;
#[cfg(feature = "quic")]
pub mod quic_transport_factory;
#[cfg(all(test, feature = "quic"))]
mod quic_transport_factory_tests;
#[cfg(all(test, feature = "quic"))]
mod quic_worker_tests;
pub mod secure_admission;
pub mod secure_control_external_key;
pub mod secure_control_session;
pub mod secure_control_tls;
pub mod secure_frame_channel;
pub mod secure_identity;
pub mod secure_listener_proof;
pub mod secure_replay;
pub mod secure_trust;
mod socket;
mod types;
mod virtual_fault;
mod virtual_reconnect;
mod virtual_transport;

#[cfg(test)]
mod authorization_tests;
#[cfg(test)]
mod handshake_tests;
#[cfg(test)]
mod key_bound_namespace_tests;
#[cfg(test)]
mod late_join_liveness_tests;
#[cfg(test)]
mod liveness_tests;
#[cfg(test)]
mod manual_endpoint_fuzz_tests;
#[cfg(test)]
mod secure_admission_tests;
#[cfg(test)]
mod secure_control_external_key_tests;
#[cfg(test)]
mod secure_control_session_tests;
#[cfg(test)]
mod secure_control_tls_tests;
#[cfg(test)]
mod secure_frame_channel_tests;
#[cfg(test)]
mod secure_listener_proof_tests;
#[cfg(test)]
mod secure_replay_fuzz_tests;
#[cfg(test)]
mod secure_trust_fuzz_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod virtual_fault_tests;
#[cfg(test)]
mod virtual_reconnect_tests;
#[cfg(test)]
mod virtual_tests;
#[cfg(test)]
mod virtual_wire_tests;

pub use boundary::{HostTransportNode, ListenerTransportNode, TransportFactory};
pub use clock::{ManualTransportClock, SystemTransportClock, TransportClock};
pub use manual_endpoint::{
    MAX_MANUAL_ENDPOINT_PAYLOAD_BYTES, ManualHostEndpoint, supports_endpoint_version,
};
#[cfg(feature = "quic")]
pub use quic_transport_factory::{
    QuicTransportFactory, SECURE_V4_ENDPOINT_VERSION, TransportProtocol, transport_factory_for,
};
pub use socket::{SocketHostTransport, SocketListenerTransport, SocketTransportFactory};
pub use types::{
    DEFAULT_IO_TIMEOUT, DEFAULT_MAX_CONSECUTIVE_FAILURES, DEFAULT_MAX_TRANSPORT_PEERS,
    DEFAULT_OPERATION_TIMEOUT, DEFAULT_PEER_INBOUND_SILENCE_TIMEOUT,
    DEFAULT_TRANSPORT_EVENT_CAPACITY, DEFAULT_TRANSPORT_QUEUE_CAPACITY, DatagramSendFailures,
    HostTransportConfig, ListenerDatagramRoutes, ListenerTransportConfig,
    MAX_TRANSPORT_EVENT_CAPACITY, MAX_TRANSPORT_PEERS, MAX_TRANSPORT_QUEUE_CAPACITY,
    SecureHostTarget, TransportChannel, TransportCounters, TransportDelivery, TransportError,
    TransportErrorKind, TransportEvent, TransportPeer,
};
pub use virtual_fault::{
    DeterministicPrng, FaultInjectingVirtualTransportFactory, VirtualUdpFaultConfig,
};
pub use virtual_reconnect::ReconnectDelayingTransportFactory;
pub use virtual_transport::{VirtualTransportFactory, VirtualTransportNetwork};

/// Returns the production standard-IP transport factory.
///
/// Virtual transport is available only through explicit construction of
/// [`VirtualTransportFactory`]; no production fallback selects it implicitly.
#[must_use]
pub const fn production_transport_factory() -> SocketTransportFactory {
    SocketTransportFactory
}
