//! v4 QUIC host transport (ADR phase 2, option 1): a `HostTransportNode` over
//! [`QuicHostWorker`].
//!
//! It keeps the socket host's contract so the host actor cannot tell the two
//! apart: a peer is accepted, may send exactly one `JoinRequest`, receives only
//! admission controls until the host authorizes it, then receives the cached
//! playback snapshot before any live broadcast, and so on. What differs is
//! only what QUIC makes unnecessary or stronger:
//!
//! - The listener's identity is the key it proved during QUIC admission, so a
//!   join must claim exactly that key's device ID, and the host may grant
//!   durable trust (`trusted_for_future`), which the unauthenticated v3 path
//!   refuses.
//! - Sync and audio travel as DATAGRAM frames on the peer's own connection.
//!   There are no UDP routes to authorize; the routes in `PeerAuthorized` are
//!   the peer's QUIC address.
//! - A vanished listener is detected by the QUIC idle timeout
//!   (`QUIC_IDLE_TIMEOUT`, kept alive by `QUIC_KEEP_ALIVE`), not by
//!   inbound-silence eviction.
//!
//! Inbound events are translated on the caller's thread inside `recv_event`,
//! so the peer table changes in the same order the actor observes events.
//! Locks are taken in the order `control_delivery`, then `state`; neither is
//! held while waiting on the worker except `control_delivery`, which orders
//! deliveries exactly as in the socket host.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, DecodePolicy, ProtocolDecoder, ProtocolFrame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::quic_host_worker::{QuicHostConfig, QuicHostError, QuicHostEvent, QuicHostWorker};
use super::secure_admission::SECURE_ADMISSION_GENERATION;
use super::secure_identity::VerifiedListenerKey;
use super::socket::PlaybackControls;
use super::{
    HostTransportConfig, TransportChannel, TransportClock, TransportCounters, TransportError,
    TransportErrorKind, TransportEvent, TransportPeer,
};

/// Longest single wait on the worker queue inside `recv_event`, so locally
/// generated events (such as `PeerAuthorized`) are not held up behind it.
const POLL_SLICE: Duration = Duration::from_millis(10);

/// Where a connection stands with the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Admission {
    /// Admitted at the QUIC layer; may receive admission controls only.
    Pending,
    /// Approved: receives playback controls, sync and audio.
    Authorized,
    /// Refused or kicked: a later approval may not revive this connection.
    Denied,
}

pub(super) struct QuicPeer {
    pub(super) remote: SocketAddr,
    pub(super) verified: VerifiedListenerKey,
    /// Set by the peer's (single) `JoinRequest`.
    pub(super) identity: Option<DeviceId>,
    pub(super) join_forwarded: bool,
    pub(super) admission: Admission,
    /// Cleared when the host closes the peer; the entry stays until the
    /// worker reports the close, so `PeerDisconnected` can name the peer.
    pub(super) active: bool,
}

impl QuicPeer {
    pub(super) fn transport_peer(&self) -> TransportPeer {
        TransportPeer {
            device_id: self.identity.clone(),
            control_address: self.remote,
        }
    }
}

#[derive(Default)]
pub(super) struct HostState {
    pub(super) peers: HashMap<u64, QuicPeer>,
    pub(super) stopping: bool,
}

/// v4 host transport over QUIC.
pub struct QuicHostTransport {
    pub(super) session_id: SessionId,
    pub(super) endpoint: NetworkEndpoint,
    pub(super) worker: QuicHostWorker,
    pub(super) clock: Arc<dyn TransportClock>,
    pub(super) state: Mutex<HostState>,
    /// Orders every control delivery, authorization replay and audio send,
    /// as `SocketHostTransport::control_delivery` does.
    pub(super) control_delivery: Mutex<()>,
    pub(super) playback_controls: Mutex<PlaybackControls>,
    /// Events the transport raises itself (`PeerAuthorized`), bounded by the
    /// configured event capacity.
    pub(super) local_events: Mutex<VecDeque<TransportEvent>>,
    pub(super) event_capacity: usize,
    pub(super) counters: Mutex<TransportCounters>,
    decoder: ProtocolDecoder,
    shutdown_complete: bool,
}

impl std::fmt::Debug for QuicHostTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuicHostTransport")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

pub(super) fn poisoned(what: &str) -> TransportError {
    TransportError::new(
        TransportErrorKind::WorkerPanicked,
        TransportChannel::Runtime,
        format!("{what} is poisoned"),
    )
}

pub(super) fn worker_error(channel: TransportChannel, error: &QuicHostError) -> TransportError {
    let kind = match error {
        QuicHostError::Start(_) => TransportErrorKind::Bind,
        QuicHostError::UnknownPeer(_) => TransportErrorKind::PeerNotFound,
        QuicHostError::Send(_) => TransportErrorKind::Delivery,
        QuicHostError::Stopped => TransportErrorKind::ShuttingDown,
    };
    TransportError::new(kind, channel, error.to_string())
}

impl QuicHostTransport {
    /// Binds the v4 QUIC endpoint on `bind_address:control_port`. Sync and
    /// audio share that port, so the endpoint reports it for all three.
    ///
    /// # Errors
    /// `InvalidConfiguration` for an invalid config or a missing TLS identity
    /// (v4 cannot run without one); `Bind` if the endpoint cannot start.
    pub fn bind(
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, TransportError> {
        config.validate()?;
        let identity = config.secure_control.clone().ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "the v4 QUIC host requires a TLS identity",
            )
        })?;
        let worker = QuicHostWorker::start(
            QuicHostConfig {
                bind: SocketAddr::new(config.bind_address, config.control_port),
                session: config.session_id.clone(),
                generation: SECURE_ADMISSION_GENERATION,
                max_peers: config.max_peers,
                event_capacity: config.event_queue_capacity,
            },
            &identity,
        )
        .map_err(|error| worker_error(TransportChannel::Runtime, &error))?;
        let port = worker.local_addr().port();
        Ok(Self {
            session_id: config.session_id,
            endpoint: NetworkEndpoint {
                address: config.bind_address,
                control_port: port,
                sync_port: port,
                audio_port: port,
            },
            worker,
            clock,
            state: Mutex::new(HostState::default()),
            control_delivery: Mutex::new(()),
            playback_controls: Mutex::new(PlaybackControls::default()),
            local_events: Mutex::new(VecDeque::new()),
            event_capacity: config.event_queue_capacity,
            counters: Mutex::new(TransportCounters::default()),
            decoder: ProtocolDecoder::default(),
            shutdown_complete: false,
        })
    }

    /// The bound QUIC address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.worker.local_addr()
    }

    pub(super) fn state(&self) -> Result<MutexGuard<'_, HostState>, TransportError> {
        self.state.lock().map_err(|_| poisoned("QUIC peer table"))
    }

    pub(super) fn count(&self, update: impl FnOnce(&mut TransportCounters)) {
        if let Ok(mut counters) = self.counters.lock() {
            update(&mut counters);
        }
    }

    pub(super) fn next_event(
        &mut self,
        timeout: Duration,
    ) -> Result<TransportEvent, TransportError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self
                .local_events
                .lock()
                .map_err(|_| poisoned("QUIC local event queue"))?
                .pop_front()
            {
                return Ok(event);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.worker.recv_event(remaining.min(POLL_SLICE)) {
                Ok(Some(event)) => {
                    if let Some(event) = self.translate(event)? {
                        return Ok(event);
                    }
                }
                Ok(None) if remaining.is_zero() => {
                    return Err(TransportError::timeout(
                        TransportChannel::Runtime,
                        "timed out waiting for a transport event",
                    ));
                }
                Ok(None) => {}
                Err(error) => return Err(worker_error(TransportChannel::Runtime, &error)),
            }
        }
    }

    fn translate(
        &mut self,
        event: QuicHostEvent,
    ) -> Result<Option<TransportEvent>, TransportError> {
        let received_at = self.clock.now();
        match event {
            QuicHostEvent::Admitted {
                peer,
                remote,
                listener,
            } => {
                let mut state = self.state()?;
                if state.stopping {
                    drop(state);
                    let _ = self.worker.close_peer(peer, "host shutting down");
                    return Ok(None);
                }
                let entry = QuicPeer {
                    remote,
                    verified: listener,
                    identity: None,
                    join_forwarded: false,
                    admission: Admission::Pending,
                    active: true,
                };
                let transport_peer = entry.transport_peer();
                state.peers.insert(peer, entry);
                drop(state);
                self.count(|c| c.accepted_control_connections += 1);
                Ok(Some(TransportEvent::PeerAccepted {
                    peer: transport_peer,
                    received_at,
                }))
            }
            QuicHostEvent::Refused {
                remote,
                reason,
                at_capacity,
            } => {
                let kind = if at_capacity {
                    self.count(|c| {
                        c.busy_admission_refusals += 1;
                        c.delivery_failures += 1;
                    });
                    TransportErrorKind::QueueFull
                } else {
                    self.count(|c| c.unauthorized_frames += 1);
                    TransportErrorKind::Unauthorized
                };
                Ok(Some(TransportEvent::Rejected {
                    channel: TransportChannel::Control,
                    source: remote,
                    error: TransportError::new(kind, TransportChannel::Control, reason),
                    received_at,
                }))
            }
            QuicHostEvent::Control { peer, frame } => self.translate_control(peer, &frame),
            QuicHostEvent::Datagram { peer, datagram } => {
                Ok(self.translate_datagram(peer, &datagram))
            }
            QuicHostEvent::Closed { peer, reason } => {
                let Some(entry) = self.state()?.peers.remove(&peer) else {
                    return Ok(None);
                };
                self.count(|c| c.closed_control_connections += 1);
                // A close the host asked for is not a failure of the peer.
                let error = entry.active.then(|| {
                    TransportError::new(TransportErrorKind::Io, TransportChannel::Control, reason)
                });
                Ok(Some(TransportEvent::PeerDisconnected {
                    peer: entry.transport_peer(),
                    error,
                    received_at,
                }))
            }
        }
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<ProtocolFrame, TransportError> {
        self.decoder
            .decode(
                bytes,
                DecodePolicy {
                    expected_session_id: Some(&self.session_id),
                    minimum_audio_sequence: None,
                },
            )
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))
    }

    fn translate_control(
        &mut self,
        peer: u64,
        bytes: &[u8],
    ) -> Result<Option<TransportEvent>, TransportError> {
        let received_at = self.clock.now();
        self.count(|c| {
            c.control_frames_received += 1;
            c.bytes_received += bytes.len() as u64;
        });
        let decoded = self.decode(bytes);
        let mut state = self.state()?;
        let Some(entry) = state.peers.get_mut(&peer) else {
            return Ok(None);
        };
        if !entry.active {
            return Ok(None);
        }
        let outcome = decoded.and_then(|frame| validate_host_frame(entry, frame));
        match outcome {
            Ok(Some(frame)) => Ok(Some(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                peer: entry.transport_peer(),
                frame,
                received_at,
            })),
            Ok(None) => {
                drop(state);
                self.count(|c| c.duplicate_join_suppressions += 1);
                Ok(None)
            }
            Err(error) => {
                // As on the socket path, a peer that sends an invalid control
                // frame is disconnected; the close is reported separately.
                entry.active = false;
                entry.admission = Admission::Denied;
                let source = entry.remote;
                drop(state);
                self.classify(&error);
                let _ = self.worker.close_peer(peer, "invalid control frame");
                Ok(Some(TransportEvent::Rejected {
                    channel: TransportChannel::Control,
                    source,
                    error,
                    received_at,
                }))
            }
        }
    }

    fn translate_datagram(&mut self, peer: u64, bytes: &[u8]) -> Option<TransportEvent> {
        let channel = TransportChannel::Synchronization;
        let received_at = self.clock.now();
        let decoded = self.decode(bytes);
        let (sender, source) = {
            let state = self.state().ok()?;
            let entry = state.peers.get(&peer)?;
            let sender = (entry.active && entry.admission == Admission::Authorized)
                .then(|| entry.transport_peer());
            (sender, entry.remote)
        };
        let rejected = |error: TransportError| TransportEvent::Rejected {
            channel,
            source,
            error,
            received_at,
        };
        let Some(sender) = sender else {
            self.count(|c| c.unauthorized_frames += 1);
            return Some(rejected(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "datagram from a peer that is not authorized",
            )));
        };
        match decoded {
            // Listeners send only clock probes; audio flows host to listener.
            Ok(frame @ ProtocolFrame::SyncRequest(_)) => {
                self.count(|c| {
                    c.sync_datagrams_received += 1;
                    c.bytes_received += bytes.len() as u64;
                });
                Some(TransportEvent::FrameReceived {
                    channel,
                    peer: sender,
                    frame,
                    received_at,
                })
            }
            Ok(_) => {
                self.count(|c| c.malformed_frames += 1);
                Some(rejected(TransportError::new(
                    TransportErrorKind::Protocol,
                    channel,
                    "listener datagram is not a synchronization request",
                )))
            }
            Err(error) => {
                let error = TransportError::new(error.kind, channel, error.message);
                self.classify(&error);
                Some(rejected(error))
            }
        }
    }

    fn classify(&self, error: &TransportError) {
        match error.kind {
            TransportErrorKind::Unauthorized => self.count(|c| c.unauthorized_frames += 1),
            TransportErrorKind::Protocol
                if error.message.contains("large") || error.message.contains("maximum") =>
            {
                self.count(|c| c.oversized_frames += 1);
            }
            TransportErrorKind::Protocol => self.count(|c| c.malformed_frames += 1),
            _ => {}
        }
    }

    pub(super) fn shutdown_worker(&mut self) {
        if self.shutdown_complete {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.stopping = true;
        }
        self.worker.shutdown();
        self.shutdown_complete = true;
    }
}

impl Drop for QuicHostTransport {
    fn drop(&mut self) {
        self.shutdown_worker();
    }
}

/// The socket host's inbound control rules, keyed to the proven listener key.
/// `Ok(None)` is a repeated join, which is suppressed.
fn validate_host_frame(
    peer: &mut QuicPeer,
    frame: ProtocolFrame,
) -> Result<Option<ProtocolFrame>, TransportError> {
    let unauthorized = |message: &str| {
        Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            message,
        ))
    };
    let ProtocolFrame::Control(message) = &frame else {
        return Err(TransportError::new(
            TransportErrorKind::Protocol,
            TransportChannel::Control,
            "only control frames may travel on the control stream",
        ));
    };
    let claimed = match message {
        ControlMessage::JoinRequest(request) => {
            if request.device.device_id != peer.verified.canonical_device_id() {
                return unauthorized("claimed device ID does not match the verified listener key");
            }
            if peer.join_forwarded {
                // One join per connection; a retransmission is not a new join.
                return Ok(None);
            }
            peer.join_forwarded = true;
            peer.identity = Some(request.device.device_id.clone());
            return Ok(Some(frame));
        }
        ControlMessage::Heartbeat(value) => &value.listener_id,
        ControlMessage::Disconnect(value) => &value.listener_id,
        ControlMessage::ResyncNotice(value) => &value.listener_id,
        ControlMessage::SynchronizationReport(value) => &value.listener_id,
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::TrackBoundary(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => {
            return unauthorized("listener sent a host-only control message");
        }
    };
    if peer.admission != Admission::Authorized || peer.identity.as_ref() != Some(claimed) {
        return unauthorized("control frame was not sent by the authorized peer identity");
    }
    Ok(Some(frame))
}

/// Encodes one control message as a protocol frame.
pub(super) fn encode_control(message: &ControlMessage) -> Result<Vec<u8>, TransportError> {
    encode_frame(&ProtocolFrame::Control(message.clone()))
        .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))
}
