//! Explicit selection of the wire protocol (owner decision 2026-11-12).
//!
//! A host uses v4 QUIC only when its owner turns the secure-transport switch
//! on; nothing selects it from the presence of an identity, so today's v3
//! listeners keep working by default. A v4 host accepts only v4 listeners.

use std::sync::Arc;

use super::quic_host_transport::QuicHostTransport;
use super::quic_listener_transport::QuicListenerTransport;
use super::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    SocketTransportFactory, TransportClock, TransportError, TransportFactory,
};

/// The wire protocol a host or listener speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportProtocol {
    /// Plaintext TCP control and UDP datagrams (protocol v3). The default.
    #[default]
    LegacyV3,
    /// One authenticated, encrypted QUIC connection per listener (v4).
    /// Requires a host TLS identity, or a pinned host key and listener key.
    SecureV4,
}

/// Builds v4 QUIC hosts and listeners.
#[derive(Debug, Default, Clone, Copy)]
pub struct QuicTransportFactory;

impl TransportFactory for QuicTransportFactory {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        QuicHostTransport::bind(config, clock)
            .map(|node| Box::new(node) as Box<dyn HostTransportNode>)
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        QuicListenerTransport::connect(config, clock)
            .map(|node| Box::new(node) as Box<dyn ListenerTransportNode>)
    }
}

/// The production factory for `protocol`. There is no fallback: a v4 bind
/// or connect that cannot run fails rather than dropping to v3.
#[must_use]
pub fn transport_factory_for(protocol: TransportProtocol) -> Box<dyn TransportFactory> {
    match protocol {
        TransportProtocol::LegacyV3 => Box::new(SocketTransportFactory),
        TransportProtocol::SecureV4 => Box::new(QuicTransportFactory),
    }
}
