mod host;
mod host_workers;
mod listener;
// Block 44 implicit-shutdown tests intentionally sit beside the shared shutdown policy.
#[cfg_attr(test, allow(clippy::items_after_test_module))]
mod shared;

pub use host::SocketHostTransport;
pub use listener::SocketListenerTransport;

use std::sync::Arc;

use super::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    TransportClock, TransportError, TransportFactory,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct SocketTransportFactory;

impl TransportFactory for SocketTransportFactory {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        SocketHostTransport::bind(config, clock)
            .map(|runtime| Box::new(runtime) as Box<dyn HostTransportNode>)
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        SocketListenerTransport::connect(config, clock)
            .map(|runtime| Box::new(runtime) as Box<dyn ListenerTransportNode>)
    }
}
