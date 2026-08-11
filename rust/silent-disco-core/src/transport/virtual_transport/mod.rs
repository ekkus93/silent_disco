//! In-process virtual transport, split by concern: shared network state and
//! the [`TransportFactory`](crate::transport::TransportFactory) impl
//! ([`network`]), the host-side [`HostTransportNode`](crate::transport::HostTransportNode)
//! implementation ([`host`]), the listener-side
//! [`ListenerTransportNode`](crate::transport::ListenerTransportNode)
//! implementation ([`listener`]), and shared helpers ([`support`]).
//!
//! Only [`VirtualTransportFactory`] and [`VirtualTransportNetwork`] are part
//! of this module's public surface (re-exported at `crate::transport::*`);
//! everything else is `pub(super)`, reachable across these sibling files but
//! not outside `virtual_transport`.

mod host;
mod listener;
mod network;
mod support;

pub use network::{VirtualTransportFactory, VirtualTransportNetwork};
