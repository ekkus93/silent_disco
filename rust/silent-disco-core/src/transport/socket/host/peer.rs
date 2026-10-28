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
    /// A TCP connection may forward at most one `JoinRequest` to the host actor.
    /// A new connection has a fresh flag; this is not durable identity proof.
    pub(in crate::transport::socket) join_forwarded: AtomicBool,
    pub(in crate::transport::socket) active: AtomicBool,
    pub(in crate::transport::socket) consecutive_failures: AtomicU32,
    pub(in crate::transport::socket) sender: ControlSender,
    pub(in crate::transport::socket) stop: Arc<AtomicBool>,
    pub(in crate::transport::socket) shutdown_stream: Mutex<TcpStream>,
    /// Host clock reading (`TransportClock::now`) at the most recent inbound
    /// frame genuinely attributed to this peer, or at the most recent stream
    /// start delivered to it -- initialized to registration time. Read by
    /// [`super::SocketHostTransport::authorized_routes`] to presume a peer gone
    /// after [`super::super::super::HostTransportConfig::peer_inbound_silence_timeout`]
    /// of silence -- see that field's doc comment for why this exists.
    pub(in crate::transport::socket) last_inbound_millis: AtomicU64,
    /// Whether this peer is currently expected to be sending sync probes, and
    /// so whether silence means it is gone.
    ///
    /// A listener only probes while it has a stream to synchronize to. An
    /// approved listener waiting for the host to start playback sends
    /// nothing at all, legitimately, for as long as the host takes to press
    /// Play. Applying the silence timeout to it evicted every listener
    /// approved more than eight seconds before playback -- at the first
    /// broadcast, which is the stream start itself (Block 52.1). Its
    /// departure before playback is still observed through its control
    /// connection closing.
    ///
    /// Armed by an inbound sync/audio datagram (the listener is provably
    /// probing) or by delivering a stream start (it is now expected to);
    /// disarmed by delivering a stream stop.
    pub(in crate::transport::socket) silence_armed: AtomicBool,
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

    /// Records any inbound frame. Refreshes the silence clock without arming
    /// it: a join request or report says the peer is there, not that it is
    /// probing.
    pub(in crate::transport::socket) fn mark_inbound_activity(
        &self,
        at: crate::domain::MonotonicMillis,
    ) {
        self.last_inbound_millis.store(at.get(), Ordering::Release);
    }

    /// Records an inbound sync/audio datagram, which proves the peer is
    /// probing, so its silence from here on is meaningful.
    pub(in crate::transport::socket) fn mark_inbound_datagram(
        &self,
        at: crate::domain::MonotonicMillis,
    ) {
        self.mark_inbound_activity(at);
        self.silence_armed.store(true, Ordering::Release);
    }

    /// Starts the silence clock when a stream start is delivered: the
    /// listener gets a full timeout from now to begin probing.
    pub(in crate::transport::socket) fn arm_silence_clock(
        &self,
        at: crate::domain::MonotonicMillis,
    ) {
        self.mark_inbound_activity(at);
        self.silence_armed.store(true, Ordering::Release);
    }

    /// Stops applying the silence timeout once a stream stop is delivered:
    /// the listener stops probing when there is nothing left to sync to.
    pub(in crate::transport::socket) fn disarm_silence_clock(&self) {
        self.silence_armed.store(false, Ordering::Release);
    }

    pub(super) fn is_inbound_silent(
        &self,
        now: crate::domain::MonotonicMillis,
        timeout: Duration,
    ) -> bool {
        if !self.silence_armed.load(Ordering::Acquire) {
            return false;
        }
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
