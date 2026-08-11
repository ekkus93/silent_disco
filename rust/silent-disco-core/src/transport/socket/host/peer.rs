use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::DeviceId;

use super::super::super::{ListenerDatagramRoutes, TransportError, TransportPeer};
use super::super::shared::{ControlSender, shutdown_stream};

pub(in crate::transport::socket) struct PeerState {
    pub(in crate::transport::socket) number: u64,
    pub(in crate::transport::socket) remote: SocketAddr,
    pub(in crate::transport::socket) identity: Mutex<Option<DeviceId>>,
    pub(in crate::transport::socket) authorized: AtomicBool,
    pub(in crate::transport::socket) active: AtomicBool,
    pub(in crate::transport::socket) consecutive_failures: AtomicU32,
    pub(in crate::transport::socket) sender: ControlSender,
    pub(in crate::transport::socket) stop: Arc<AtomicBool>,
    pub(in crate::transport::socket) shutdown_stream: Mutex<TcpStream>,
    /// Host clock reading (`TransportClock::now`) at the most recent inbound
    /// sync/audio datagram genuinely attributed to this peer -- initialized
    /// to registration time so a freshly authorized peer that hasn't sent
    /// its first sync probe yet isn't immediately evicted. Read by
    /// [`super::SocketHostTransport::authorized_routes`] to presume a peer gone
    /// after [`super::super::super::HostTransportConfig::peer_inbound_silence_timeout`]
    /// of silence -- see that field's doc comment for why this exists.
    pub(in crate::transport::socket) last_inbound_millis: AtomicU64,
}

impl PeerState {
    pub(in crate::transport::socket) fn transport_peer(&self) -> TransportPeer {
        let device_id = self
            .identity
            .lock()
            .ok()
            .and_then(|identity| identity.clone());
        TransportPeer {
            device_id,
            control_address: self.remote,
        }
    }

    pub(in crate::transport::socket) fn device_id(&self) -> Option<DeviceId> {
        self.identity
            .lock()
            .ok()
            .and_then(|identity| identity.clone())
    }

    pub(in crate::transport::socket) fn close(&self) -> Result<(), TransportError> {
        self.active.store(false, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        shutdown_stream(&self.shutdown_stream)
    }

    pub(in crate::transport::socket) fn mark_inbound_activity(
        &self,
        at: crate::domain::MonotonicMillis,
    ) {
        self.last_inbound_millis.store(at.get(), Ordering::Release);
    }

    pub(super) fn is_inbound_silent(
        &self,
        now: crate::domain::MonotonicMillis,
        timeout: Duration,
    ) -> bool {
        let elapsed = now
            .get()
            .saturating_sub(self.last_inbound_millis.load(Ordering::Acquire));
        elapsed >= u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)
    }
}

#[derive(Clone)]
pub(in crate::transport::socket) struct PeerRoute {
    pub(in crate::transport::socket) peer: Arc<PeerState>,
    pub(in crate::transport::socket) routes: ListenerDatagramRoutes,
}
