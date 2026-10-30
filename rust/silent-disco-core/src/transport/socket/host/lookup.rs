use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::domain::DeviceId;
use crate::protocol::ControlMessage;

use super::super::super::{TransportChannel, TransportError, TransportErrorKind};
use super::SocketHostTransport;
use super::peer::{PeerRoute, PeerState};

impl SocketHostTransport {
    pub(super) fn pending_peer_for_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Arc<PeerState>, TransportError> {
        let peers = self.peers.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer registry is poisoned",
            )
        })?;
        let mut candidates = peers.values().filter(|peer| {
            peer.active.load(Ordering::Acquire)
                && !peer.authorized.load(Ordering::Acquire)
                && !peer.admission_denied.load(Ordering::Acquire)
                && peer.device_id().as_ref() == Some(device_id)
        });
        let peer = candidates.next().cloned().ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "identified pending control peer is not connected",
            )
        })?;
        // Legacy v3 exposes only a self-claimed device ID to the approval
        // API. Selecting the newest of two pending TCP generations could
        // authorize a different connection from the one the host approved.
        // Refuse ambiguous admission until one generation disconnects.
        if candidates.next().is_some() {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "multiple pending TCP peers claim the same device ID; admission is ambiguous",
            ));
        }
        Ok(peer)
    }

    pub(super) fn peer_for_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Arc<PeerState>, TransportError> {
        let routes = self.routes.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer route registry is poisoned",
            )
        })?;
        routes
            .get(device_id)
            .map(|route| route.peer.clone())
            .filter(|peer| peer.active.load(Ordering::Acquire))
            .ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::PeerNotFound,
                    TransportChannel::Control,
                    "authorized peer is not connected",
                )
            })
    }

    /// Returns every currently-active authorized route, evicting (and
    /// excluding from the result) any peer that has sent nothing for longer
    /// than `peer_inbound_silence_timeout` -- see [`super::peer::PeerState::last_inbound_millis`]'s
    /// doc comment for why this exists. Eviction reuses [`super::peer::PeerState::close`],
    /// the same mechanism `record_peer_result`'s `max_consecutive_failures`
    /// path already uses, so it surfaces through the identical, already-
    /// tested `PeerDisconnected` event path.
    pub(super) fn authorized_routes(&self) -> Result<Vec<(DeviceId, PeerRoute)>, TransportError> {
        let routes = self.routes.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer route registry is poisoned",
            )
        })?;
        let now = self.clock.now();
        Ok(routes
            .iter()
            .filter(|(_, route)| {
                if !route.peer.active.load(Ordering::Acquire) {
                    return false;
                }
                if route
                    .peer
                    .is_inbound_silent(now, self.config.peer_inbound_silence_timeout)
                {
                    drop(route.peer.close());
                    return false;
                }
                true
            })
            .map(|(device_id, route)| (device_id.clone(), route.clone()))
            .collect())
    }

    /// Arms or disarms a peer's silence clock according to a control
    /// message just delivered to it -- see [`super::peer::PeerState::silence_armed`].
    pub(super) fn update_silence_clock(&self, peer: &PeerState, message: &ControlMessage) {
        match message {
            ControlMessage::StreamStart(_) => peer.arm_silence_clock(self.clock.now()),
            ControlMessage::Stop(_) => peer.disarm_silence_clock(),
            _ => {}
        }
    }

    pub(super) fn record_peer_result(
        &self,
        peer: &PeerState,
        result: &Result<usize, TransportError>,
    ) {
        if result.is_ok() {
            peer.consecutive_failures.store(0, Ordering::Release);
            return;
        }
        let failures = peer
            .consecutive_failures
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        if failures >= self.config.max_consecutive_failures {
            drop(peer.close());
        }
    }
}
