//! v4 listener QUIC connection on its own thread (ADR phase 2, option 1).
//!
//! The listener-side counterpart of `quic_host_worker`: one thread owns a
//! current-thread Tokio runtime and a single connection to the host, pinned to
//! the host's invitation key. The connection is usable only after the
//! listener has proved its key over it (`quic_admission`). Control frames and
//! datagrams then cross through bounded queues with the same rules as the
//! host: control frames are never dropped, datagrams may be and are counted.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::quic_admission::{QuicControlStream, quic_listener_prove};
use super::quic_datagram::quic_client_config;
use super::quic_framing::{read_frame, write_frame};
use super::secure_control_tls::HOST_SERVER_NAME;
use super::secure_listener_proof::ListenerProofSigner;
use crate::domain::SessionId;

/// Listener connection settings.
#[derive(Debug, Clone)]
pub struct QuicListenerConfig {
    /// The host's UDP address.
    pub host: SocketAddr,
    /// The host TLS key pinned from its invitation.
    pub host_spki: Vec<u8>,
    /// Session to prove for.
    pub session: SessionId,
    /// Session generation to prove for.
    pub generation: u64,
    /// Capacity of the event queue towards the listener.
    pub event_capacity: usize,
}

/// Something that happened on the connection.
#[derive(Debug)]
pub enum QuicListenerEvent {
    /// One control frame from the host, in order.
    Control(Vec<u8>),
    /// One datagram from the host.
    Datagram(Vec<u8>),
    /// The connection ended.
    Closed(String),
}

/// Why a listener worker call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuicListenerError {
    /// Connecting or proving failed.
    Connect(String),
    /// Sending failed.
    Send(String),
    /// The worker has stopped.
    Stopped,
}

impl std::fmt::Display for QuicListenerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(reason) => {
                write!(formatter, "QUIC connection to the host failed: {reason}")
            }
            Self::Send(reason) => write!(formatter, "QUIC send failed: {reason}"),
            Self::Stopped => formatter.write_str("QUIC listener worker has stopped"),
        }
    }
}

impl std::error::Error for QuicListenerError {}

type Reply = oneshot::Sender<Result<(), QuicListenerError>>;

enum Command {
    SendControl {
        frame: Vec<u8>,
        reply: Reply,
    },
    SendDatagram {
        datagram: Vec<u8>,
        reply: Reply,
    },
    /// Write what is queued, wait (bounded) for it to be acknowledged, then
    /// close. Used for both `close` and `shutdown`.
    Close {
        reason: String,
    },
}

const QUEUE_CAPACITY: usize = 256;
const EVENT_BACKOFF: Duration = Duration::from_millis(2);
/// How long a closing listener waits for the host to acknowledge its last
/// control frames (such as a `Disconnect`). Closing a QUIC connection
/// discards unacknowledged stream data, so the close waits first.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
/// Longest wait for the host to answer the QUIC handshake. QUIC runs over
/// UDP and quinn ignores ICMP "port unreachable", so a closed or v3-only
/// host is otherwise noticed only at the 8 s idle timeout. This matches the
/// v3 TCP connect timeout (`DEFAULT_OPERATION_TIMEOUT`), so a failed v4 join
/// is reported as quickly as a failed v3 one. Established connections keep
/// the 8 s idle timeout.
pub const QUIC_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Handle to a connected, admitted listener connection.
pub struct QuicListenerWorker {
    commands: mpsc::Sender<Command>,
    /// Locked only while receiving, never by the send methods, so a send
    /// (such as a clock probe) never waits behind a blocked receive. The
    /// lock is what makes the handle `Sync`.
    events: Mutex<Receiver<QuicListenerEvent>>,
    dropped_datagrams: Arc<AtomicU64>,
    max_datagram_size: Option<usize>,
    local_addr: SocketAddr,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for QuicListenerWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuicListenerWorker")
            .field("max_datagram_size", &self.max_datagram_size)
            .finish_non_exhaustive()
    }
}

impl QuicListenerWorker {
    /// Connects to the host, pins its key, proves the listener key, and
    /// returns once the connection is admitted.
    ///
    /// # Errors
    /// Returns [`QuicListenerError::Connect`] if the runtime, socket,
    /// handshake (including a host that does not hold the pinned key) or the
    /// proof exchange fails.
    pub fn connect(
        config: QuicListenerConfig,
        signer: Arc<dyn ListenerProofSigner + Send + Sync>,
    ) -> Result<Self, QuicListenerError> {
        let client_config = quic_client_config(&config.host_spki)
            .map_err(|error| QuicListenerError::Connect(error.to_string()))?;
        let (event_tx, events) = sync_channel(config.event_capacity.max(1));
        let (command_tx, command_rx) = mpsc::channel(QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let worker_dropped = Arc::clone(&dropped);
        let thread = thread::Builder::new()
            .name("silent-disco-quic-listener".to_owned())
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
                    let connected =
                        connect_and_prove(&config, client_config, signer.as_ref()).await;
                    let (endpoint, connection, stream) = match connected {
                        Ok(connected) => connected,
                        Err(reason) => {
                            let _ = ready_tx.send(Err(reason));
                            return;
                        }
                    };
                    let local = endpoint.local_addr().map_err(|error| error.to_string());
                    let ready = local.map(|local| (connection.max_datagram_size(), local));
                    let failed = ready.is_err();
                    let _ = ready_tx.send(ready);
                    if failed {
                        connection.close(1_u32.into(), b"no local address");
                        return;
                    }
                    run_listener(connection, stream, worker_dropped, event_tx, command_rx).await;
                    endpoint.wait_idle().await;
                });
            })
            .map_err(|error| QuicListenerError::Connect(error.to_string()))?;
        let (max_datagram_size, local_addr) = ready_rx
            .recv()
            .map_err(|_| QuicListenerError::Connect("worker ended before connecting".to_owned()))?
            .map_err(QuicListenerError::Connect)?;
        Ok(Self {
            commands: command_tx,
            events: Mutex::new(events),
            dropped_datagrams: dropped,
            max_datagram_size,
            local_addr,
            thread: Some(thread),
        })
    }

    /// Largest datagram the connection can carry, as negotiated at connect.
    #[must_use]
    pub const fn max_datagram_size(&self) -> Option<usize> {
        self.max_datagram_size
    }

    /// The local UDP address of this connection (unspecified IP, real port).
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Datagrams dropped because the event queue was full.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.dropped_datagrams.load(Ordering::Relaxed)
    }

    /// Waits up to `timeout` for the next event; `None` on timeout.
    ///
    /// # Errors
    /// Returns [`QuicListenerError::Stopped`] once the worker has ended and
    /// every queued event has been read.
    pub fn recv_event(
        &self,
        timeout: Duration,
    ) -> Result<Option<QuicListenerEvent>, QuicListenerError> {
        let events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        match events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(QuicListenerError::Stopped),
        }
    }

    fn request(&self, build: impl FnOnce(Reply) -> Command) -> Result<(), QuicListenerError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .blocking_send(build(reply))
            .map_err(|_| QuicListenerError::Stopped)?;
        response
            .blocking_recv()
            .map_err(|_| QuicListenerError::Stopped)?
    }

    /// Sends one control frame and waits until it is written.
    ///
    /// # Errors
    /// Returns an error for a failed write or a stopped worker.
    pub fn send_control(&self, frame: Vec<u8>) -> Result<(), QuicListenerError> {
        self.request(|reply| Command::SendControl { frame, reply })
    }

    /// Sends one datagram to the host.
    ///
    /// # Errors
    /// Returns an error for a datagram the connection cannot carry or a
    /// stopped worker.
    pub fn send_datagram(&self, datagram: Vec<u8>) -> Result<(), QuicListenerError> {
        self.request(|reply| Command::SendDatagram { datagram, reply })
    }

    /// Closes the connection after already sent control frames are
    /// acknowledged (bounded by a short grace period). Sends fail from then
    /// on, and a `Closed` event follows. Does not join the thread.
    ///
    /// # Errors
    /// Returns [`QuicListenerError::Stopped`] if the worker has stopped.
    pub fn close(&self, reason: &str) -> Result<(), QuicListenerError> {
        self.commands
            .blocking_send(Command::Close {
                reason: reason.to_owned(),
            })
            .map_err(|_| QuicListenerError::Stopped)
    }

    /// Closes the connection as [`Self::close`] does and joins the worker
    /// thread. Idempotent.
    pub fn shutdown(&mut self) {
        let _ = self.commands.blocking_send(Command::Close {
            reason: "listener shut down".to_owned(),
        });
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for QuicListenerWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn connect_and_prove(
    config: &QuicListenerConfig,
    client_config: quinn::ClientConfig,
    signer: &dyn ListenerProofSigner,
) -> Result<(quinn::Endpoint, quinn::Connection, QuicControlStream), String> {
    let local = if config.host.is_ipv4() {
        SocketAddr::from(([0, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0_u16; 8], 0))
    };
    let mut endpoint = quinn::Endpoint::client(local).map_err(|error| error.to_string())?;
    endpoint.set_default_client_config(client_config);
    let connecting = endpoint
        .connect(config.host, HOST_SERVER_NAME)
        .map_err(|error| error.to_string())?;
    let connection = tokio::time::timeout(QUIC_CONNECT_TIMEOUT, connecting)
        .await
        .map_err(|_| {
            format!(
                "no QUIC answer from {} within {} s; is the host running and using secure transport (v4)?",
                config.host,
                QUIC_CONNECT_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| error.to_string())?;
    let stream = quic_listener_prove(
        &connection,
        &config.session,
        config.generation,
        &config.host_spki,
        signer,
    )
    .await
    .map_err(|error| {
        connection.close(1_u32.into(), b"proof failed");
        error.to_string()
    })?;
    Ok((endpoint, connection, stream))
}

async fn run_listener(
    connection: quinn::Connection,
    stream: QuicControlStream,
    dropped: Arc<AtomicU64>,
    events: SyncSender<QuicListenerEvent>,
    mut commands: mpsc::Receiver<Command>,
) {
    let QuicControlStream { mut send, mut recv } = stream;
    let datagram_task = tokio::spawn(read_datagrams(connection.clone(), dropped, events.clone()));
    let control_events = events.clone();
    let mut control_task = tokio::spawn(async move {
        loop {
            match read_frame(&mut recv).await {
                Ok(frame) => {
                    send_event_lossless(&control_events, QuicListenerEvent::Control(frame)).await;
                }
                Err(error) => return error.to_string(),
            }
        }
    });
    let reason = loop {
        tokio::select! {
            ended = &mut control_task => {
                break ended.unwrap_or_else(|error| error.to_string());
            }
            command = commands.recv() => match command {
                None => break "listener shut down".to_owned(),
                Some(Command::Close { reason }) => {
                    let _ = send.finish();
                    // Bounded: a host that never acknowledges is closed anyway.
                    let _ = tokio::time::timeout(CLOSE_GRACE, send.stopped()).await;
                    break reason;
                }
                Some(Command::SendControl { frame, reply }) => {
                    let result = write_frame(&mut send, &frame)
                        .await
                        .map_err(|error| QuicListenerError::Send(error.to_string()));
                    let _ = reply.send(result);
                }
                Some(Command::SendDatagram { datagram, reply }) => {
                    let result = connection
                        .send_datagram(datagram.into())
                        .map_err(|error| QuicListenerError::Send(error.to_string()));
                    let _ = reply.send(result);
                }
            },
        }
    };
    let _ = send.finish();
    connection.close(0_u32.into(), b"listener closing");
    control_task.abort();
    datagram_task.abort();
    send_event_lossless(&events, QuicListenerEvent::Closed(reason)).await;
}

async fn read_datagrams(
    connection: quinn::Connection,
    dropped: Arc<AtomicU64>,
    events: SyncSender<QuicListenerEvent>,
) {
    while let Ok(datagram) = connection.read_datagram().await {
        if events
            .try_send(QuicListenerEvent::Datagram(datagram.to_vec()))
            .is_err()
        {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn send_event_lossless(events: &SyncSender<QuicListenerEvent>, mut event: QuicListenerEvent) {
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
