//! `HostTransportNode` for [`QuicHostTransport`]: outbound control, the
//! authorization replay, and DATAGRAM delivery. Every rule mirrors
//! `SocketHostTransport` unless the module docs of `quic_host_transport`
//! say otherwise.

use std::net::SocketAddr;
use std::time::Duration;

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::quic_host_transport::{
    Admission, QuicHostTransport, encode_control, poisoned, worker_error,
};
use super::{
    HostTransportNode, ListenerDatagramRoutes, TransportChannel, TransportCounters,
    TransportDelivery, TransportError, TransportErrorKind, TransportEvent,
};

/// Controls addressed to exactly one listener. They may never be broadcast or
/// delivered to a different peer.
fn addressed_listener(message: &ControlMessage) -> Option<&DeviceId> {
    match message {
        ControlMessage::JoinApproval(value) => Some(&value.listener_id),
        ControlMessage::JoinRejection(value) => Some(&value.listener_id),
        ControlMessage::Disconnect(value) => Some(&value.listener_id),
        ControlMessage::ResyncNotice(value) => Some(&value.listener_id),
        _ => None,
    }
}

fn check_recipient(device_id: &DeviceId, message: &ControlMessage) -> Result<(), TransportError> {
    if addressed_listener(message).is_some_and(|recipient| recipient != device_id) {
        return Err(unauthorized(
            "listener-specific control is addressed to a different device",
        ));
    }
    Ok(())
}

fn unauthorized(message: &str) -> TransportError {
    TransportError::new(
        TransportErrorKind::Unauthorized,
        TransportChannel::Control,
        message,
    )
}

fn not_found(channel: TransportChannel, message: &str) -> TransportError {
    TransportError::new(TransportErrorKind::PeerNotFound, channel, message)
}

fn shutting_down() -> TransportError {
    TransportError::new(
        TransportErrorKind::ShuttingDown,
        TransportChannel::Runtime,
        "QUIC host transport is shutting down",
    )
}

/// A missed admission, recovery or playback control leaves the listener on a
/// timeline it was not told about, so the peer is retired rather than left
/// receiving audio. Heartbeats and telemetry stay lossy.
const fn retires_on_failure(message: &ControlMessage) -> bool {
    matches!(
        message,
        ControlMessage::Hello(_)
            | ControlMessage::JoinApproval(_)
            | ControlMessage::JoinRejection(_)
            | ControlMessage::Disconnect(_)
            | ControlMessage::ResyncNotice(_)
            | ControlMessage::StreamStart(_)
            | ControlMessage::TrackBoundary(_)
            | ControlMessage::Pause(_)
            | ControlMessage::Stop(_)
    )
}

fn delivered(bytes: u64) -> Result<TransportDelivery, TransportError> {
    TransportDelivery::new(1, 1, 0, bytes)
}

impl QuicHostTransport {
    fn check_running(&self) -> Result<(), TransportError> {
        if self.state()?.stopping {
            return Err(shutting_down());
        }
        Ok(())
    }

    fn check_session(&self, message: &ControlMessage) -> Result<(), TransportError> {
        if message.session_id() != &self.session_id {
            return Err(unauthorized(
                "outbound control message belongs to a different session",
            ));
        }
        Ok(())
    }

    /// The single pending connection, or the newest one, proven to hold this
    /// device's key. Unlike v3, two pending connections for one device are
    /// the same key holder reconnecting, so the newest is not ambiguous.
    fn pending_peer(&self, device_id: &DeviceId) -> Result<u64, TransportError> {
        self.state()?
            .peers
            .iter()
            .filter(|(_, peer)| {
                peer.active
                    && peer.admission == Admission::Pending
                    && peer.identity.as_ref() == Some(device_id)
            })
            .map(|(number, _)| *number)
            .max()
            .ok_or_else(|| {
                not_found(
                    TransportChannel::Control,
                    "identified pending control peer is not connected",
                )
            })
    }

    fn authorized_peer(
        &self,
        device_id: &DeviceId,
        channel: TransportChannel,
    ) -> Result<u64, TransportError> {
        self.state()?
            .peers
            .iter()
            .find(|(_, peer)| {
                peer.active
                    && peer.admission == Admission::Authorized
                    && peer.identity.as_ref() == Some(device_id)
            })
            .map(|(number, _)| *number)
            .ok_or_else(|| not_found(channel, "authorized peer is not connected"))
    }

    fn authorized_peers(&self) -> Result<Vec<u64>, TransportError> {
        let mut peers: Vec<u64> = self
            .state()?
            .peers
            .iter()
            .filter(|(_, peer)| peer.active && peer.admission == Admission::Authorized)
            .map(|(number, _)| *number)
            .collect();
        peers.sort_unstable();
        Ok(peers)
    }

    /// Retires a peer: no further sends, no later approval, and its
    /// connection closes after already queued frames are acknowledged.
    fn retire(&self, peer: u64, reason: &str) {
        if let Ok(mut state) = self.state()
            && let Some(entry) = state.peers.get_mut(&peer)
        {
            entry.active = false;
            entry.admission = Admission::Denied;
        }
        let _ = self.worker.close_peer(peer, reason);
    }

    /// Writes one control frame and waits until the worker has written it.
    fn send_to(&self, peer: u64, message: &ControlMessage) -> Result<u64, TransportError> {
        let bytes = encode_control(message)?;
        let length = bytes.len() as u64;
        match self.worker.send_control(peer, bytes) {
            Ok(()) => {
                self.count(|c| {
                    c.control_frames_sent += 1;
                    c.bytes_sent += length;
                });
                Ok(length)
            }
            Err(error) => {
                self.count(|c| c.delivery_failures += 1);
                if retires_on_failure(message) {
                    self.retire(peer, "control delivery failed");
                }
                Err(worker_error(TransportChannel::Control, &error))
            }
        }
    }

    fn send_snapshot(&self, peer: u64, messages: &[ControlMessage]) -> Result<u64, TransportError> {
        let mut total = 0_u64;
        for message in messages {
            // A partial replay is not an authorized timeline; `send_to`
            // retires the peer on the first failure.
            total = total.saturating_add(self.send_to(peer, message)?);
        }
        Ok(total)
    }

    fn authorize(&self, device_id: &DeviceId) -> Result<(), TransportError> {
        self.check_running()?;
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self.pending_peer(device_id)?;
        // Replay before the peer can receive live broadcasts, under the same
        // ordering lock, so it never sees a boundary before its StreamStart.
        let snapshot = self
            .playback_controls
            .lock()
            .map_err(|_| poisoned("playback control cache"))?
            .snapshot_at(self.clock.now());
        self.send_snapshot(peer, &snapshot)?;
        let mut events = self
            .local_events
            .lock()
            .map_err(|_| poisoned("QUIC local event queue"))?;
        if events.len() >= self.event_capacity {
            // The actor must observe every authorization it granted. If it
            // cannot, the listener is not admitted.
            drop(events);
            self.count(|c| {
                c.authoritative_event_failures += 1;
                c.delivery_failures += 1;
            });
            self.retire(peer, "authorization could not be reported");
            return Err(TransportError::new(
                TransportErrorKind::QueueFull,
                TransportChannel::Runtime,
                "host admission event could not be delivered; listener authorization rolled back",
            ));
        }
        let (event, replaced) = {
            let mut state = self.state()?;
            let Some(entry) = state.peers.get(&peer).filter(|entry| entry.active) else {
                return Err(not_found(
                    TransportChannel::Control,
                    "pending control peer disconnected during playback replay",
                ));
            };
            let route = entry.remote;
            let transport_peer = entry.transport_peer();
            // A reconnect replaces this device's older authorized connection.
            let replaced: Vec<u64> = state
                .peers
                .iter()
                .filter(|(number, other)| {
                    **number != peer
                        && other.admission == Admission::Authorized
                        && other.identity.as_ref() == Some(device_id)
                })
                .map(|(number, _)| *number)
                .collect();
            if let Some(entry) = state.peers.get_mut(&peer) {
                entry.admission = Admission::Authorized;
            }
            let event = TransportEvent::PeerAuthorized {
                peer: transport_peer,
                routes: ListenerDatagramRoutes {
                    synchronization: route,
                    audio: route,
                },
                received_at: self.clock.now(),
            };
            (event, replaced)
        };
        events.push_back(event);
        drop(events);
        for old in replaced {
            self.retire(old, "replaced by a newer connection");
        }
        Ok(())
    }

    fn send_datagrams(
        &self,
        channel: TransportChannel,
        peers: Vec<u64>,
        bytes: Vec<u8>,
    ) -> Result<TransportDelivery, TransportError> {
        let intended = u32::try_from(peers.len()).map_err(|_| {
            TransportError::new(
                TransportErrorKind::Delivery,
                channel,
                "peer count exceeds delivery accounting range",
            )
        })?;
        if peers.is_empty() {
            return TransportDelivery::new(0, 0, 0, 0);
        }
        let length = bytes.len() as u64;
        let results = self
            .worker
            .send_datagram_to(peers, bytes)
            .map_err(|error| worker_error(channel, &error))?;
        let successful =
            u32::try_from(results.iter().filter(|(_, r)| r.is_ok()).count()).unwrap_or(u32::MAX);
        let failed = intended.saturating_sub(successful);
        let sent = length.saturating_mul(u64::from(successful));
        self.count(|c| {
            match channel {
                TransportChannel::Audio => c.audio_datagrams_sent += u64::from(successful),
                _ => c.sync_datagrams_sent += u64::from(successful),
            }
            c.bytes_sent += sent;
            c.delivery_failures += u64::from(failed);
        });
        TransportDelivery::new(intended, successful, failed, sent)
    }
}

impl HostTransportNode for QuicHostTransport {
    fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    fn authorize_peer(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        let pending = self.pending_peer(device_id)?;
        let remote = self
            .state()?
            .peers
            .get(&pending)
            .map(|peer| peer.remote)
            .ok_or_else(|| not_found(TransportChannel::Control, "pending peer is gone"))?;
        if routes.synchronization.ip() != remote.ip() || routes.audio.ip() != remote.ip() {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Runtime,
                "datagram routes must use the authenticated control peer address",
            ));
        }
        self.authorize(device_id)
    }

    fn authorize_peer_ports(
        &self,
        device_id: &DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError> {
        if sync_port == 0 || audio_port == 0 {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "listener UDP routes must use nonzero ports",
            ));
        }
        // The ports are not used: datagrams ride the peer's QUIC connection.
        self.authorize(device_id)
    }

    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError> {
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self.authorized_peer(device_id, TransportChannel::Control)?;
        self.retire(peer, "disconnected by host");
        Ok(())
    }

    fn send_disconnect(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if !matches!(message, ControlMessage::Disconnect(_)) {
            return Err(unauthorized(
                "targeted disconnect must belong to this session",
            ));
        }
        self.check_session(message)?;
        check_recipient(device_id, message)?;
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self.authorized_peer(device_id, TransportChannel::Control)?;
        let result = self.send_to(peer, message);
        // Kicked either way; the close waits for the Disconnect to be acknowledged.
        self.retire(peer, "disconnected by host");
        delivered(result?)
    }

    fn send_pending_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.check_session(message)?;
        check_recipient(device_id, message)?;
        if !matches!(
            message,
            ControlMessage::Hello(_)
                | ControlMessage::JoinApproval(_)
                | ControlMessage::JoinRejection(_)
                | ControlMessage::Disconnect(_)
        ) {
            return Err(unauthorized(
                "pending peers may receive admission controls only",
            ));
        }
        // The listener's key is proven, so `trusted_for_future` is allowed here.
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self.pending_peer(device_id)?;
        let bytes = self.send_to(peer, message)?;
        if matches!(
            message,
            ControlMessage::JoinRejection(_) | ControlMessage::Disconnect(_)
        ) {
            // A refused connection cannot be approved later.
            if let Some(entry) = self.state()?.peers.get_mut(&peer) {
                entry.admission = Admission::Denied;
            }
        }
        delivered(bytes)
    }

    fn reject_pending_peer_at(
        &self,
        control_address: SocketAddr,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        if !matches!(message, ControlMessage::JoinRejection(_)) {
            return Err(unauthorized(
                "pending refusal requires a same-session JoinRejection",
            ));
        }
        self.check_session(message)?;
        check_recipient(device_id, message)?;
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self
            .state()?
            .peers
            .iter()
            .find(|(_, peer)| {
                peer.remote == control_address
                    && peer.active
                    && peer.admission != Admission::Authorized
                    && peer.identity.as_ref() == Some(device_id)
            })
            .map(|(number, _)| *number)
            .ok_or_else(|| {
                not_found(
                    TransportChannel::Control,
                    "matching pending QUIC connection is not connected",
                )
            })?;
        let result = self.send_to(peer, message);
        self.retire(peer, "join refused");
        delivered(result?)
    }

    fn send_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.check_session(message)?;
        check_recipient(device_id, message)?;
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self.authorized_peer(device_id, TransportChannel::Control)?;
        delivered(self.send_to(peer, message)?)
    }

    fn replay_cached_playback_to_peer(
        &self,
        device_id: &DeviceId,
    ) -> Result<TransportDelivery, TransportError> {
        self.check_running()?;
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        let peer = self.authorized_peer(device_id, TransportChannel::Control)?;
        let snapshot = self
            .playback_controls
            .lock()
            .map_err(|_| poisoned("playback control cache"))?
            .recovery_snapshot_at(self.clock.now());
        let bytes = self.send_snapshot(peer, &snapshot)?;
        if bytes == 0 {
            TransportDelivery::new(0, 0, 0, 0)
        } else {
            delivered(bytes)
        }
    }

    fn broadcast_control(
        &self,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.check_session(message)?;
        if addressed_listener(message).is_some() {
            return Err(unauthorized(
                "listener-specific control requires a targeted connection",
            ));
        }
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("control delivery ordering lock"))?;
        // A stale Pause, boundary or Stop the cache refuses must not reach
        // live listeners either, so late joiners and live peers agree.
        let accepted = self
            .playback_controls
            .lock()
            .map_err(|_| poisoned("playback control cache"))?
            .update(message);
        if !accepted {
            return TransportDelivery::new(0, 0, 0, 0);
        }
        let peers = self.authorized_peers()?;
        let intended = u32::try_from(peers.len()).map_err(|_| {
            TransportError::new(
                TransportErrorKind::Delivery,
                TransportChannel::Control,
                "peer count exceeds delivery accounting range",
            )
        })?;
        let mut successful = 0_u32;
        let mut bytes = 0_u64;
        for peer in peers {
            if let Ok(written) = self.send_to(peer, message) {
                successful += 1;
                bytes = bytes.saturating_add(written);
            }
        }
        TransportDelivery::new(intended, successful, intended - successful, bytes)
    }

    fn broadcast_sync(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        let channel = TransportChannel::Synchronization;
        match frame {
            ProtocolFrame::SyncRequest(_) => {}
            ProtocolFrame::SyncResponse(_) => {
                return Err(TransportError::new(
                    TransportErrorKind::Unauthorized,
                    channel,
                    "host synchronization responses require a targeted authorized route",
                ));
            }
            _ => {
                return Err(TransportError::new(
                    TransportErrorKind::Protocol,
                    channel,
                    "protocol frame does not belong on the requested datagram channel",
                ));
            }
        }
        if frame.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "outbound datagram belongs to a different session",
            ));
        }
        let bytes =
            encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
        self.send_datagrams(channel, self.authorized_peers()?, bytes)
    }

    fn send_sync_to_peer(
        &self,
        device_id: &DeviceId,
        frame: &ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        let channel = TransportChannel::Synchronization;
        if !matches!(frame, ProtocolFrame::SyncResponse(_)) {
            return Err(TransportError::new(
                TransportErrorKind::Protocol,
                channel,
                "targeted synchronization requires a SyncResponse",
            ));
        }
        if frame.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "targeted synchronization belongs to another session",
            ));
        }
        let bytes =
            encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
        let peer = self.authorized_peer(device_id, channel)?;
        self.send_datagrams(channel, vec![peer], bytes)
    }

    fn broadcast_audio(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        let channel = TransportChannel::Audio;
        let ProtocolFrame::Audio(packet) = frame else {
            return Err(TransportError::new(
                TransportErrorKind::Protocol,
                channel,
                "protocol frame does not belong on the requested datagram channel",
            ));
        };
        if frame.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "outbound datagram belongs to a different session",
            ));
        }
        // Audio and Stop/Pause/StreamStart have one observable order.
        let _delivery = self
            .control_delivery
            .lock()
            .map_err(|_| poisoned("playback delivery ordering lock"))?;
        let bytes =
            encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
        let permitted = self
            .playback_controls
            .lock()
            .map_err(|_| poisoned("playback control cache"))?
            .permits_audio(packet);
        if !permitted {
            return TransportDelivery::new(0, 0, 0, 0);
        }
        self.send_datagrams(channel, self.authorized_peers()?, bytes)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        self.next_event(timeout)
    }

    fn counters(&self) -> TransportCounters {
        let mut counters = self
            .counters
            .lock()
            .map_or_else(|_| TransportCounters::default(), |c| *c);
        // Inbound datagrams the worker dropped on a full event queue.
        counters.best_effort_event_drops += self.worker.dropped_datagrams();
        counters
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.shutdown_worker();
        Ok(())
    }
}
