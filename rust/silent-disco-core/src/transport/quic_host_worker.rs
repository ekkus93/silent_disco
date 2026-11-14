//! v4 host QUIC endpoint on its own thread (ADR phase 2, option 1).
//!
//! One thread owns a current-thread Tokio runtime and the quinn endpoint. The
//! rest of the host talks to it through a synchronous API: commands go in over
//! a bounded queue, events come out over a bounded queue. Each connection must
//! pass the v4 admission (`quic_admission`) before it becomes a peer; after
//! that its control stream carries length-prefixed frames and its QUIC
//! DATAGRAM frames carry sync and audio, both protected by that connection's
//! own TLS 1.3 keys.
//!
//! Queues are bounded. A full event queue pauses reading a peer's control
//! stream rather than dropping a control frame; datagrams are lossy by nature
//! and are dropped and counted instead.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::quic_admission::{QuicControlStream, quic_host_admit};
use super::quic_datagram::quic_server_config;
use super::quic_framing::{read_frame, write_frame};
use super::secure_control_tls::HostTlsIdentity;
use super::secure_identity::VerifiedListenerKey;
use crate::domain::SessionId;

/// Host worker settings.
#[derive(Debug, Clone)]
pub struct QuicHostConfig {
    /// Local UDP address to bind (port 0 picks one).
    pub bind: SocketAddr,
    /// Session listeners must prove for.
    pub session: SessionId,
    /// Session generation listeners must prove for.
    pub generation: u64,
    /// Most peers admitted or being admitted at once.
    pub max_peers: usize,
    /// Capacity of the event queue towards the host.
    pub event_capacity: usize,
}

/// Something that happened on the endpoint.
#[derive(Debug)]
pub enum QuicHostEvent {
    /// A connection passed v4 admission and is now a peer.
    Admitted {
        peer: u64,
        remote: SocketAddr,
        listener: VerifiedListenerKey,
    },
    /// A connection failed admission or was refused for capacity.
    Refused {
        remote: SocketAddr,
        reason: String,
        /// `true` when the host was at its peer limit, `false` when the
        /// connection failed the handshake or the v4 proof.
        at_capacity: bool,
    },
    /// One control frame from a peer, in order.
    Control { peer: u64, frame: Vec<u8> },
    /// One datagram from a peer.
    Datagram { peer: u64, datagram: Vec<u8> },
    /// A peer's connection ended.
    Closed { peer: u64, reason: String },
}

/// Why a host worker call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuicHostError {
    /// The endpoint could not be started.
    Start(String),
    /// No admitted peer has that number.
    UnknownPeer(u64),
    /// Sending to the peer failed.
    Send(String),
    /// The worker has stopped.
    Stopped,
}

impl std::fmt::Display for QuicHostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(reason) => write!(formatter, "QUIC host failed to start: {reason}"),
            Self::UnknownPeer(peer) => write!(formatter, "no admitted QUIC peer {peer}"),
            Self::Send(reason) => write!(formatter, "QUIC send failed: {reason}"),
            Self::Stopped => formatter.write_str("QUIC host worker has stopped"),
        }
    }
}

impl std::error::Error for QuicHostError {}

type Reply<T> = oneshot::Sender<Result<T, QuicHostError>>;

/// Per-peer outcome of a datagram broadcast.
pub type BroadcastResults = Vec<(u64, Result<(), QuicHostError>)>;

enum Command {
    SendControl {
        peer: u64,
        frame: Vec<u8>,
        reply: Reply<()>,
    },
    SendDatagram {
        peer: u64,
        datagram: Vec<u8>,
        reply: Reply<()>,
    },
    BroadcastDatagram {
        /// `None` sends to every admitted peer.
        peers: Option<Vec<u64>>,
        datagram: Vec<u8>,
        reply: oneshot::Sender<BroadcastResults>,
    },
    Close {
        peer: u64,
        reason: String,
    },
    Shutdown,
}

/// One item on a peer's control writer queue.
enum ControlWrite {
    Frame(Vec<u8>, Reply<()>),
    /// Write everything queued before this, then close the connection.
    Close(String),
}

struct PeerEntry {
    connection: quinn::Connection,
    control: mpsc::Sender<ControlWrite>,
}

#[derive(Default)]
struct Shared {
    peers: Mutex<HashMap<u64, PeerEntry>>,
    pending: AtomicU64,
    next_peer: AtomicU64,
    dropped_datagrams: AtomicU64,
    /// Refusals at the peer limit that could not be reported because the
    /// event queue was full.
    dropped_refusals: AtomicU64,
}

impl Shared {
    fn peers(&self) -> std::sync::MutexGuard<'_, HashMap<u64, PeerEntry>> {
        self.peers.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Capacity of each peer's control write queue and of the command queue.
const QUEUE_CAPACITY: usize = 256;
/// Retry interval while the event queue is full for a control frame.
const EVENT_BACKOFF: Duration = Duration::from_millis(2);
/// How long a closing peer may take to acknowledge its last control frames
/// (for example a `JoinRejection` or `Disconnect`) before the connection
/// is closed anyway. Closing a QUIC connection discards unacknowledged
/// stream data, so the close waits for the acknowledgement first.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Longest one control frame may wait to be written. A peer that stops
/// reading its control stream fills its flow-control window, and an
/// unbounded write would hold every caller (a Stop broadcast included).
pub(super) const CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Why a peer that stopped reading its control stream was disconnected.
const STALLED_REASON: &str = "listener stopped reading its control stream";
/// Extra time on top of [`CLOSE_GRACE`] for the writers to run before a
/// shutdown closes whatever is still open.
const SHUTDOWN_SLACK: Duration = Duration::from_millis(500);

/// Handle to a running host QUIC endpoint.
pub struct QuicHostWorker {
    local_addr: SocketAddr,
    commands: mpsc::Sender<Command>,
    events: Receiver<QuicHostEvent>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for QuicHostWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuicHostWorker")
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

impl QuicHostWorker {
    /// Binds the endpoint and starts accepting connections.
    ///
    /// # Errors
    /// Returns [`QuicHostError::Start`] if the configuration, runtime,
    /// socket or thread cannot be set up.
    pub fn start(
        config: QuicHostConfig,
        identity: &HostTlsIdentity,
    ) -> Result<Self, QuicHostError> {
        let server_config = quic_server_config(identity)
            .map_err(|error| QuicHostError::Start(error.to_string()))?;
        let host_spki = identity.spki.clone();
        let (event_tx, events) = sync_channel(config.event_capacity.max(1));
        let (command_tx, command_rx) = mpsc::channel(QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = sync_channel(1);
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("silent-disco-quic-host".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let endpoint = match quinn::Endpoint::server(server_config, config.bind) {
                        Ok(endpoint) => endpoint,
                        Err(error) => {
                            let _ = ready_tx.send(Err(error.to_string()));
                            return;
                        }
                    };
                    let bound = endpoint.local_addr().map_err(|error| error.to_string());
                    let _ = ready_tx.send(bound);
                    run_host(
                        endpoint,
                        config,
                        host_spki,
                        worker_shared,
                        event_tx,
                        command_rx,
                    )
                    .await;
                });
            })
            .map_err(|error| QuicHostError::Start(error.to_string()))?;
        let local_addr = ready_rx
            .recv()
            .map_err(|_| QuicHostError::Start("worker ended before binding".to_owned()))?
            .map_err(QuicHostError::Start)?;
        Ok(Self {
            local_addr,
            commands: command_tx,
            events,
            shared,
            thread: Some(thread),
        })
    }

    /// The bound UDP address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Datagrams dropped because the event queue was full.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.shared.dropped_datagrams.load(Ordering::Relaxed)
    }

    /// Peer-limit refusals not reported because the event queue was full.
    /// The connection is refused either way.
    #[must_use]
    pub fn dropped_refusals(&self) -> u64 {
        self.shared.dropped_refusals.load(Ordering::Relaxed)
    }

    /// Waits up to `timeout` for the next event; `None` on timeout.
    ///
    /// # Errors
    /// Returns [`QuicHostError::Stopped`] once the worker has ended and every
    /// queued event has been read.
    pub fn recv_event(&self, timeout: Duration) -> Result<Option<QuicHostEvent>, QuicHostError> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(QuicHostError::Stopped),
        }
    }

    fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<T>) -> Command,
    ) -> Result<T, QuicHostError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .blocking_send(build(reply))
            .map_err(|_| QuicHostError::Stopped)?;
        response.blocking_recv().map_err(|_| QuicHostError::Stopped)
    }

    /// Sends one control frame to a peer and waits until it is written.
    ///
    /// # Errors
    /// Returns an error for an unknown peer, a failed write or a stopped worker.
    pub fn send_control(&self, peer: u64, frame: Vec<u8>) -> Result<(), QuicHostError> {
        self.request(|reply| Command::SendControl { peer, frame, reply })?
    }

    /// Sends one datagram to a peer.
    ///
    /// # Errors
    /// Returns an error for an unknown peer, a datagram the connection cannot
    /// carry, or a stopped worker.
    pub fn send_datagram(&self, peer: u64, datagram: Vec<u8>) -> Result<(), QuicHostError> {
        self.request(|reply| Command::SendDatagram {
            peer,
            datagram,
            reply,
        })?
    }

    /// Sends one datagram to every admitted peer, each on its own connection.
    ///
    /// # Errors
    /// Returns [`QuicHostError::Stopped`] if the worker has stopped.
    pub fn broadcast_datagram(&self, datagram: Vec<u8>) -> Result<BroadcastResults, QuicHostError> {
        self.request(|reply| Command::BroadcastDatagram {
            peers: None,
            datagram,
            reply,
        })
    }

    /// Sends one datagram to each listed peer, each on its own connection.
    /// A listed peer that is no longer admitted reports
    /// [`QuicHostError::UnknownPeer`].
    ///
    /// # Errors
    /// Returns [`QuicHostError::Stopped`] if the worker has stopped.
    pub fn send_datagram_to(
        &self,
        peers: Vec<u64>,
        datagram: Vec<u8>,
    ) -> Result<BroadcastResults, QuicHostError> {
        self.request(|reply| Command::BroadcastDatagram {
            peers: Some(peers),
            datagram,
            reply,
        })
    }

    /// Closes one peer's connection after its already queued control frames
    /// have been written and acknowledged (bounded by a short grace period).
    /// The peer stops accepting new sends at once.
    ///
    /// # Errors
    /// Returns [`QuicHostError::Stopped`] if the worker has stopped.
    pub fn close_peer(&self, peer: u64, reason: &str) -> Result<(), QuicHostError> {
        self.commands
            .blocking_send(Command::Close {
                peer,
                reason: reason.to_owned(),
            })
            .map_err(|_| QuicHostError::Stopped)
    }

    /// Closes every connection and joins the worker thread. Idempotent.
    pub fn shutdown(&mut self) {
        let _ = self.commands.blocking_send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for QuicHostWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run_host(
    endpoint: quinn::Endpoint,
    config: QuicHostConfig,
    host_spki: Vec<u8>,
    shared: Arc<Shared>,
    events: SyncSender<QuicHostEvent>,
    mut commands: mpsc::Receiver<Command>,
) {
    let events = Events::start(events);
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let remote = incoming.remote_address();
                let in_use = shared.peers().len()
                    + usize::try_from(shared.pending.load(Ordering::Acquire)).unwrap_or(usize::MAX);
                if in_use >= config.max_peers {
                    incoming.refuse();
                    // Never wait here: this loop also handles commands (a
                    // Stop broadcast included), and a stalled event consumer
                    // would freeze them all. The connection is refused
                    // either way; an unreported refusal is counted.
                    let refused = QuicHostEvent::Refused {
                        remote,
                        reason: "host is at its peer limit".to_owned(),
                        at_capacity: true,
                    };
                    if events.lossy.try_send(refused).is_err() {
                        shared.dropped_refusals.fetch_add(1, Ordering::Relaxed);
                    }
                    continue;
                }
                shared.pending.fetch_add(1, Ordering::AcqRel);
                tokio::spawn(admit_peer(
                    incoming,
                    config.session.clone(),
                    config.generation,
                    host_spki.clone(),
                    Arc::clone(&shared),
                    events.clone(),
                ));
            }
            command = commands.recv() => {
                match command {
                    None | Some(Command::Shutdown) => break,
                    Some(command) => handle_command(command, &shared),
                }
            }
        }
    }
    // Closing a QUIC connection discards control data the peer has not yet
    // acknowledged, so a Stop sent just before shutdown could be lost and
    // the listener left waiting for the idle timeout. Close every peer the
    // way `close_peer` does: let its writer finish what is queued and wait
    // (bounded) for the acknowledgement, all peers at once.
    let peers: Vec<PeerEntry> = shared.peers().drain().map(|(_, peer)| peer).collect();
    for peer in &peers {
        if peer
            .control
            .try_send(ControlWrite::Close("host shutting down".to_owned()))
            .is_err()
        {
            peer.connection.close(0_u32.into(), b"host shutting down");
        }
    }
    let _ = tokio::time::timeout(CLOSE_GRACE + SHUTDOWN_SLACK, async {
        for peer in &peers {
            peer.connection.closed().await;
        }
    })
    .await;
    endpoint.close(0_u32.into(), b"host shutting down");
    endpoint.wait_idle().await;
}

fn handle_command(command: Command, shared: &Shared) {
    match command {
        Command::SendControl { peer, frame, reply } => {
            let entry = shared
                .peers()
                .get(&peer)
                .map(|entry| (entry.control.clone(), entry.connection.clone()));
            let Some((sender, connection)) = entry else {
                let _ = reply.send(Err(QuicHostError::UnknownPeer(peer)));
                return;
            };
            // Never wait on one peer's queue here: this loop serves every
            // peer. A full queue means the peer has stopped reading.
            match sender.try_send(ControlWrite::Frame(frame, reply)) {
                // Only a frame is queued here; a `Close` cannot come back.
                Ok(())
                | Err(
                    mpsc::error::TrySendError::Full(ControlWrite::Close(_))
                    | mpsc::error::TrySendError::Closed(ControlWrite::Close(_)),
                ) => {}
                Err(mpsc::error::TrySendError::Full(ControlWrite::Frame(_, reply))) => {
                    connection.close(1_u32.into(), STALLED_REASON.as_bytes());
                    let _ = reply.send(Err(QuicHostError::Send(STALLED_REASON.to_owned())));
                }
                Err(mpsc::error::TrySendError::Closed(ControlWrite::Frame(_, reply))) => {
                    let _ = reply.send(Err(QuicHostError::UnknownPeer(peer)));
                }
            }
        }
        Command::SendDatagram {
            peer,
            datagram,
            reply,
        } => {
            let connection = shared
                .peers()
                .get(&peer)
                .map(|entry| entry.connection.clone());
            let result = match connection {
                Some(connection) => connection
                    .send_datagram(datagram.into())
                    .map_err(|error| QuicHostError::Send(error.to_string())),
                None => Err(QuicHostError::UnknownPeer(peer)),
            };
            let _ = reply.send(result);
        }
        Command::BroadcastDatagram {
            peers,
            datagram,
            reply,
        } => {
            let connections: Vec<(u64, Option<quinn::Connection>)> = {
                let table = shared.peers();
                match peers {
                    None => table
                        .iter()
                        .map(|(peer, entry)| (*peer, Some(entry.connection.clone())))
                        .collect(),
                    Some(peers) => peers
                        .into_iter()
                        .map(|peer| (peer, table.get(&peer).map(|e| e.connection.clone())))
                        .collect(),
                }
            };
            let bytes = bytes::Bytes::from(datagram);
            let results = connections
                .into_iter()
                .map(|(peer, connection)| {
                    let result = connection.map_or(Err(QuicHostError::UnknownPeer(peer)), |c| {
                        c.send_datagram(bytes.clone())
                            .map_err(|error| QuicHostError::Send(error.to_string()))
                    });
                    (peer, result)
                })
                .collect();
            let _ = reply.send(results);
        }
        Command::Close { peer, reason } => {
            let entry = shared.peers().remove(&peer);
            if let Some(entry) = entry {
                // The writer drains earlier frames, then closes. If its queue
                // is full (the peer stopped reading) or gone (the writer
                // already failed), close directly rather than wait here.
                if entry
                    .control
                    .try_send(ControlWrite::Close(reason.clone()))
                    .is_err()
                {
                    entry.connection.close(0_u32.into(), reason.as_bytes());
                }
            }
        }
        Command::Shutdown => {}
    }
}

async fn admit_peer(
    incoming: quinn::Incoming,
    session: SessionId,
    generation: u64,
    host_spki: Vec<u8>,
    shared: Arc<Shared>,
    events: Events,
) {
    let remote = incoming.remote_address();
    let admitted = async {
        let connection = incoming.await.map_err(|error| error.to_string())?;
        let (listener, stream) = quic_host_admit(&connection, &session, generation, &host_spki)
            .await
            .map_err(|error| {
                connection.close(1_u32.into(), b"admission refused");
                error.to_string()
            })?;
        Ok::<_, String>((connection, listener, stream))
    }
    .await;
    shared.pending.fetch_sub(1, Ordering::AcqRel);
    let (connection, listener, stream) = match admitted {
        Ok(admitted) => admitted,
        Err(reason) => {
            events
                .lossless(QuicHostEvent::Refused {
                    remote,
                    reason,
                    at_capacity: false,
                })
                .await;
            return;
        }
    };
    let peer = shared.next_peer.fetch_add(1, Ordering::AcqRel) + 1;
    let QuicControlStream { send, recv } = stream;
    let (control_tx, control_rx) = mpsc::channel(QUEUE_CAPACITY);
    shared.peers().insert(
        peer,
        PeerEntry {
            connection: connection.clone(),
            control: control_tx,
        },
    );
    events
        .lossless(QuicHostEvent::Admitted {
            peer,
            remote,
            listener,
        })
        .await;
    tokio::spawn(write_control(send, connection.clone(), control_rx));
    tokio::spawn(read_datagrams(
        peer,
        connection.clone(),
        Arc::clone(&shared),
        events.lossy.clone(),
    ));
    let reason = read_control(peer, recv, &events).await;
    connection.close(0_u32.into(), b"control stream ended");
    shared.peers().remove(&peer);
    events
        .lossless(QuicHostEvent::Closed { peer, reason })
        .await;
}

async fn write_control(
    mut send: quinn::SendStream,
    connection: quinn::Connection,
    mut queue: mpsc::Receiver<ControlWrite>,
) {
    while let Some(write) = queue.recv().await {
        match write {
            ControlWrite::Frame(frame, reply) => {
                let write =
                    tokio::time::timeout(CONTROL_WRITE_TIMEOUT, write_frame(&mut send, &frame));
                let result = if let Ok(written) = write.await {
                    written.map_err(|error| QuicHostError::Send(error.to_string()))
                } else {
                    connection.close(1_u32.into(), STALLED_REASON.as_bytes());
                    Err(QuicHostError::Send(STALLED_REASON.to_owned()))
                };
                let failed = result.is_err();
                let _ = reply.send(result);
                if failed {
                    break;
                }
            }
            ControlWrite::Close(reason) => {
                let _ = send.finish();
                // Bounded: a peer that never acknowledges is closed anyway.
                let _ = tokio::time::timeout(CLOSE_GRACE, send.stopped()).await;
                connection.close(0_u32.into(), reason.as_bytes());
                return;
            }
        }
    }
    let _ = send.finish();
}

async fn read_control(peer: u64, mut recv: quinn::RecvStream, events: &Events) -> String {
    loop {
        match read_frame(&mut recv).await {
            Ok(frame) => {
                events
                    .lossless(QuicHostEvent::Control { peer, frame })
                    .await;
            }
            Err(error) => return error.to_string(),
        }
    }
}

async fn read_datagrams(
    peer: u64,
    connection: quinn::Connection,
    shared: Arc<Shared>,
    events: SyncSender<QuicHostEvent>,
) {
    while let Ok(datagram) = connection.read_datagram().await {
        let event = QuicHostEvent::Datagram {
            peer,
            datagram: datagram.to_vec(),
        };
        if events.try_send(event).is_err() {
            shared.dropped_datagrams.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// How worker tasks hand events to the host side.
///
/// Events that must not be lost go through one bounded tokio queue whose
/// waiting senders are served first in, first out, and a single forwarder
/// moves them into the host's queue in that order. So when the host's
/// consumer stalls, connections waiting to report a join are still reported
/// in the order they arrived (A54 run, phase 4: per-task retries used to
/// shuffle them). Lossy events (datagrams, refusals at the peer limit) skip
/// the queue and are counted when dropped; the main loop never waits on
/// either lane.
#[derive(Clone)]
struct Events {
    lossless: mpsc::Sender<QuicHostEvent>,
    lossy: SyncSender<QuicHostEvent>,
}

impl Events {
    fn start(host: SyncSender<QuicHostEvent>) -> Self {
        let (lossless, mut ordered) = mpsc::channel(QUEUE_CAPACITY);
        let forward_to = host.clone();
        tokio::spawn(async move {
            while let Some(event) = ordered.recv().await {
                send_event_lossless(&forward_to, event).await;
            }
        });
        Self {
            lossless,
            lossy: host,
        }
    }

    /// Waits for room in the ordered queue. Gives up only when the
    /// forwarder has ended (the worker is shutting down).
    async fn lossless(&self, event: QuicHostEvent) {
        let _ = self.lossless.send(event).await;
    }
}

/// Delivers an event that must not be lost, waiting while the queue is full.
/// Gives up only when the host side has dropped the receiver.
async fn send_event_lossless(events: &SyncSender<QuicHostEvent>, mut event: QuicHostEvent) {
    loop {
        match events.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => return,
            Err(TrySendError::Full(returned)) => {
                event = returned;
                tokio::time::sleep(EVENT_BACKOFF).await;
            }
        }
    }
}
