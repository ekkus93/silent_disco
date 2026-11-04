use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::super::super::{
    HostTransportNode, ListenerDatagramRoutes, TransportChannel, TransportCounters,
    TransportDelivery, TransportError, TransportErrorKind, TransportEvent,
};
use super::super::host_workers::shutting_down_error;
use super::super::shared::{join_workers, recv_event};
use super::SocketHostTransport;
use super::peer::PeerState;

// These controls carry one intended listener ID. A misrouted admission or
// terminal control must not affect another TCP generation. Legacy v3 still
// lacks cryptographic authentication; this only enforces routing integrity.
fn addressed_listener(message: &ControlMessage) -> Option<&DeviceId> {
    match message {
        ControlMessage::JoinApproval(value) => Some(&value.listener_id),
        ControlMessage::JoinRejection(value) => Some(&value.listener_id),
        ControlMessage::Disconnect(value) => Some(&value.listener_id),
        ControlMessage::ResyncNotice(value) => Some(&value.listener_id),
        _ => None,
    }
}

fn check_control_recipient(
    device_id: &DeviceId,
    message: &ControlMessage,
) -> Result<(), TransportError> {
    if addressed_listener(message).is_some_and(|recipient| recipient != device_id) {
        return Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "listener-specific control is addressed to a different device",
        ));
    }
    Ok(())
}

impl HostTransportNode for SocketHostTransport {
    fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    fn authorize_peer(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        self.authorize_peer_with_routes(device_id, routes)
    }

    fn authorize_peer_ports(
        &self,
        device_id: &DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(shutting_down_error());
        }
        if sync_port == 0 || audio_port == 0 {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "listener UDP routes must use nonzero ports",
            ));
        }
        let peer_ip = self.pending_peer_for_device(device_id)?.remote.ip();
        let routes = ListenerDatagramRoutes {
            synchronization: SocketAddr::new(peer_ip, sync_port),
            audio: SocketAddr::new(peer_ip, audio_port),
        };
        self.authorize_peer_with_routes(device_id, routes)
    }

    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError> {
        // Serialize removal with live and replayed control delivery, so a
        // targeted sender cannot transmit after the route has been retired.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let route = self
            .routes
            .lock()
            .map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer route registry is poisoned",
                )
            })?
            .remove(device_id);
        let Some(route) = route else {
            return Err(TransportError::new(
                TransportErrorKind::PeerNotFound,
                TransportChannel::Control,
                "authorized peer is not connected",
            ));
        };
        route.peer.close()
    }

    fn send_disconnect(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if !matches!(message, ControlMessage::Disconnect(_))
            || message.session_id() != &self.session_id
        {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "targeted disconnect must belong to this session",
            ));
        }
        check_control_recipient(device_id, message)?;
        // Keep the TCP Disconnect and UDP route retirement in the same
        // critical section as cached replay, broadcasts and authorization.
        // A concurrent same-identity reconnect cannot inherit the old kick.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self.peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        let written = match result {
            Ok(written) => written,
            Err(error) => {
                self.counters.delivery_failure();
                // A failed kick cannot leave a listener authorized for UDP
                // playback. Retire this exact TCP generation even if its
                // Disconnect frame was never acknowledged by the writer.
                drop(peer.close());
                return Err(error);
            }
        };
        let mut routes = self.routes.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer route registry is poisoned",
            )
        })?;
        if routes
            .get(device_id)
            .is_some_and(|route| route.peer.number == peer.number)
        {
            routes.remove(device_id);
        }
        peer.close()?;
        TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
    }

    fn send_pending_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound pending control message belongs to a different session",
            ));
        }
        check_control_recipient(device_id, message)?;
        // A pending peer has not passed admission. Restrict this low-level
        // send path to admission and terminal controls so a mistaken caller
        // cannot bypass authorization and leak playback or queue metadata.
        if !matches!(
            message,
            ControlMessage::Hello(_)
                | ControlMessage::JoinApproval(_)
                | ControlMessage::JoinRejection(_)
                | ControlMessage::Disconnect(_)
        ) {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "pending TCP peers may receive admission controls only",
            ));
        }
        // This legacy socket transport has no cryptographically verified
        // listener identity. Even a direct desktop/core caller must not grant
        // persistent trust by setting a self-declared JoinApproval flag.
        if matches!(
            message,
            ControlMessage::JoinApproval(approval) if approval.trusted_for_future
        ) {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "legacy v3 cannot grant durable trust without verified cryptographic identity",
            ));
        }
        // Pending Hello/approval delivery must not interleave with late-join
        // authorization replay. Acquire the ordering guard before looking up
        // the peer so a concurrent authorization cannot insert its route and
        // replay StreamStart ahead of this pending control frame.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self.pending_peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        match result {
            Ok(written) => {
                // The same ordering gate protects the authorization recheck.
                // Once the host has refused this TCP attempt, a delayed
                // approval cannot revive it or redirect it to a new peer.
                if matches!(
                    message,
                    ControlMessage::JoinRejection(_) | ControlMessage::Disconnect(_)
                ) {
                    peer.admission_denied.store(true, Ordering::Release);
                }
                self.update_silence_clock(&peer, message);
                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
            }
            Err(error) => {
                self.counters.delivery_failure();
                // An unacknowledged approval, rejection or Hello cannot be
                // retried on this unverified TCP generation. Require a new
                // join rather than permitting a later stale authorization.
                peer.admission_denied.store(true, Ordering::Release);
                drop(peer.close());
                Err(error)
            }
        }
    }

    fn reject_pending_peer_at(
        &self,
        control_address: SocketAddr,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if !matches!(message, ControlMessage::JoinRejection(_))
            || message.session_id() != &self.session_id
        {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "pending refusal requires a same-session JoinRejection",
            ));
        }
        check_control_recipient(device_id, message)?;
        // The ID-only lookup is deliberately ambiguous when two unverified
        // sockets claim the same device. Target the actual incoming TCP peer.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self
            .peers
            .lock()
            .map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer registry is poisoned",
                )
            })?
            .values()
            .find(|peer| {
                peer.remote == control_address
                    && peer.active.load(Ordering::Acquire)
                    && !peer.authorized.load(Ordering::Acquire)
                    && peer.device_id().as_ref() == Some(device_id)
            })
            .cloned()
            .ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::PeerNotFound,
                    TransportChannel::Control,
                    "matching pending TCP generation is not connected",
                )
            })?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        // Even if the refusal write fails, this generation must never be
        // eligible for a delayed approval or another pending send.
        peer.admission_denied.store(true, Ordering::Release);
        drop(peer.close());
        let written = result?;
        TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
    }

    fn send_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound control message belongs to a different session",
            ));
        }
        check_control_recipient(device_id, message)?;
        // Legacy v3 has no verified peer identity. No host-facing send API
        // may turn a claimed listener ID into durable trusted identity.
        if matches!(
            message,
            ControlMessage::JoinApproval(approval) if approval.trusted_for_future
        ) {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "legacy v3 cannot grant durable trust without verified cryptographic identity",
            ));
        }
        // Select the authorized route only after acquiring the replay/delivery
        // guard. A concurrent disconnect may retire the route while a targeted
        // send is waiting here; using an earlier selection could send to a
        // disconnected peer after the delivery order becomes available.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self.peer_for_device(device_id)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        let result = peer.sender.send(bytes);
        self.record_peer_result(&peer, &result);
        match result {
            Ok(written) => {
                self.update_silence_clock(&peer, message);
                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
            }
            Err(error) => {
                self.counters.delivery_failure();
                // Targeted Stop/Pause/Start has the same authority as a
                // broadcast. A missed targeted control must revoke this
                // listener's UDP eligibility, not just report an error.
                Self::retire_on_failed_playback_delivery(&peer, message);
                Err(error)
            }
        }
    }

    fn replay_cached_playback_to_peer(
        &self,
        device_id: &DeviceId,
    ) -> Result<TransportDelivery, TransportError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(shutting_down_error());
        }
        // A live boundary, Stop, or same-ID reconnect cannot interleave
        // with the requested recovery snapshot or change its target route.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let peer = self.peer_for_device(device_id)?;
        let bytes = self.send_cached_playback_controls(&peer, true)?;
        if bytes == 0 {
            // A stopped/idle host has no active playback to replay.
            TransportDelivery::new(0, 0, 0, 0)
        } else {
            TransportDelivery::new(1, 1, 0, bytes)
        }
    }

    fn broadcast_control(
        &self,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if message.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound control message belongs to a different session",
            ));
        }
        // Legacy v3 has no verified peer identity. No host-facing send API
        // may turn a claimed listener ID into durable trusted identity.
        if matches!(
            message,
            ControlMessage::JoinApproval(approval) if approval.trusted_for_future
        ) {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "legacy v3 cannot grant durable trust without verified cryptographic identity",
            ));
        }
        // Admission decisions and targeted terminal controls have exactly one
        // recipient. Broadcasting one leaks a different listener's decision.
        if addressed_listener(message).is_some() {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "listener-specific control requires a targeted TCP route",
            ));
        }
        // Serialize with late-join authorization and its cached replay. Without
        // this guard, a newly registered peer can receive a live boundary
        // before its cached StreamStart, or receive the boundary twice.
        let _delivery_guard = self.control_delivery.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "control delivery ordering lock is poisoned",
            )
        })?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        // Keep live broadcasts consistent with late-join replay. A stale
        // Pause, boundary or Stop must not reach active listeners after the
        // authoritative cache has rejected it.
        let accepted = self
            .playback_controls
            .lock()
            .map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "playback control cache is poisoned",
                )
            })?
            .update(message);
        if !accepted {
            return TransportDelivery::new(0, 0, 0, 0);
        }
        let peers: Vec<Arc<PeerState>> = self
            .authorized_routes()?
            .into_iter()
            .map(|(_, route)| route.peer)
            .collect();
        let intended = u32::try_from(peers.len()).map_err(|_| {
            TransportError::new(
                TransportErrorKind::Delivery,
                TransportChannel::Control,
                "peer count exceeds delivery accounting range",
            )
        })?;
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut sent_bytes = 0_u64;
        for peer in peers {
            let result = peer.sender.send(bytes.clone());
            self.record_peer_result(&peer, &result);
            match result {
                Ok(written) => {
                    self.update_silence_clock(&peer, message);
                    successful = successful.saturating_add(1);
                    sent_bytes =
                        sent_bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
                }
                Err(_) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                    Self::retire_on_failed_playback_delivery(&peer, message);
                }
            }
        }
        TransportDelivery::new(intended, successful, failed, sent_bytes)
    }

    fn broadcast_sync(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.broadcast_datagram(TransportChannel::Synchronization, frame)
    }

    fn send_sync_to_peer(
        &self,
        device_id: &DeviceId,
        frame: &ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        self.send_targeted_sync(device_id, frame)
    }

    fn broadcast_audio(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.broadcast_datagram(TransportChannel::Audio, frame)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let started = Instant::now();
        loop {
            // Drain older accepted/control events before a terminal whose
            // reader has already exited. Preserve per-peer causal order.
            if let Ok(event) = self.event_receiver.try_recv() {
                return Ok(event);
            }
            if let Some(event) = self.terminal_mailbox.pop() {
                return Ok(event);
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return recv_event(&self.event_receiver, Duration::ZERO);
            }
            // A terminal may arrive while the application blocks waiting for
            // a control event. Recheck the mailbox at a bounded cadence.
            match recv_event(&self.event_receiver, remaining.min(Duration::from_millis(10))) {
                Ok(event) => return Ok(event),
                Err(error) if error.kind == TransportErrorKind::Timeout => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn counters(&self) -> TransportCounters {
        self.counters.snapshot()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.stop.store(true, Ordering::Release);
        let mut shutdown_error = None;
        match self.peers.lock() {
            Ok(peers) => {
                for peer in peers.values() {
                    if let Err(error) = peer.close() {
                        shutdown_error.get_or_insert(error);
                    }
                }
            }
            Err(_) => {
                shutdown_error = Some(TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Runtime,
                    "peer registry is poisoned during shutdown",
                ));
            }
        }
        let join_result = join_workers(&self.workers);
        self.shutdown_complete = true;
        // A finished worker was already joined by the live accept-loop
        // reaper. Its panic must still fail the explicit owner shutdown.
        if self.counters.has_worker_panic() {
            return Err(TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "a reaped transport worker panicked",
            ));
        }
        shutdown_error.map_or(join_result, Err)
    }
}
