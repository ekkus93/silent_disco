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
    Refused { remote: SocketAddr, reason: String },
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
        datagram: Vec<u8>,
        reply: oneshot::Sender<BroadcastResults>,
    },
    Close {
        peer: u64,
        reason: String,
    },
    Shutdown,
}

struct PeerEntry {
    connection: quinn::Connection,
    control: mpsc::Sender<(Vec<u8>, Reply<()>)>,
}

#[derive(Default)]
struct Shared {
    peers: Mutex<HashMap<u64, PeerEntry>>,
    pending: AtomicU64,
    next_peer: AtomicU64,
    dropped_datagrams: AtomicU64,
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
        self.request(|reply| Command::BroadcastDatagram { datagram, reply })
    }

    /// Closes one peer's connection.
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
    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let remote = incoming.remote_address();
                let in_use = shared.peers().len()
                    + usize::try_from(shared.pending.load(Ordering::Acquire)).unwrap_or(usize::MAX);
                if in_use >= config.max_peers {
                    incoming.refuse();
                    send_event_lossless(&events, QuicHostEvent::Refused {
                        remote,
                        reason: "host is at its peer limit".to_owned(),
                    }).await;
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
                    Some(command) => handle_command(command, &shared).await,
                }
            }
        }
    }
    for (_, peer) in shared.peers().drain() {
        peer.connection.close(0_u32.into(), b"host shutting down");
    }
    endpoint.close(0_u32.into(), b"host shutting down");
    endpoint.wait_idle().await;
}

async fn handle_command(command: Command, shared: &Shared) {
    match command {
        Command::SendControl { peer, frame, reply } => {
            let sender = shared.peers().get(&peer).map(|entry| entry.control.clone());
            match sender {
                Some(sender) => {
                    if let Err(mpsc::error::SendError((_, reply))) =
                        sender.send((frame, reply)).await
                    {
                        let _ = reply.send(Err(QuicHostError::UnknownPeer(peer)));
                    }
                }
                None => {
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
        Command::BroadcastDatagram { datagram, reply } => {
            let connections: Vec<(u64, quinn::Connection)> = shared
                .peers()
                .iter()
                .map(|(peer, entry)| (*peer, entry.connection.clone()))
                .collect();
            let bytes = bytes::Bytes::from(datagram);
            let results = connections
                .into_iter()
                .map(|(peer, connection)| {
                    let result = connection
                        .send_datagram(bytes.clone())
                        .map_err(|error| QuicHostError::Send(error.to_string()));
                    (peer, result)
                })
                .collect();
            let _ = reply.send(results);
        }
        Command::Close { peer, reason } => {
            if let Some(entry) = shared.peers().remove(&peer) {
                entry.connection.close(0_u32.into(), reason.as_bytes());
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
    events: SyncSender<QuicHostEvent>,
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
            send_event_lossless(&events, QuicHostEvent::Refused { remote, reason }).await;
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
    send_event_lossless(
        &events,
        QuicHostEvent::Admitted {
            peer,
            remote,
            listener,
        },
    )
    .await;
    tokio::spawn(write_control(send, control_rx));
    tokio::spawn(read_datagrams(
        peer,
        connection.clone(),
        Arc::clone(&shared),
        events.clone(),
    ));
    let reason = read_control(peer, recv, &events).await;
    connection.close(0_u32.into(), b"control stream ended");
    shared.peers().remove(&peer);
    send_event_lossless(&events, QuicHostEvent::Closed { peer, reason }).await;
}

async fn write_control(
    mut send: quinn::SendStream,
    mut queue: mpsc::Receiver<(Vec<u8>, Reply<()>)>,
) {
    while let Some((frame, reply)) = queue.recv().await {
        let result = write_frame(&mut send, &frame)
            .await
            .map_err(|error| QuicHostError::Send(error.to_string()));
        let failed = result.is_err();
        let _ = reply.send(result);
        if failed {
            break;
        }
    }
    let _ = send.finish();
}

async fn read_control(
    peer: u64,
    mut recv: quinn::RecvStream,
    events: &SyncSender<QuicHostEvent>,
) -> String {
    loop {
        match read_frame(&mut recv).await {
            Ok(frame) => send_event_lossless(events, QuicHostEvent::Control { peer, frame }).await,
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
