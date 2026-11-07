//! Accept-time routing of a new control connection.
//!
//! Only a connection that opens with a TLS handshake record is routed to secure
//! admission, on its own thread. Every other connection, including silent,
//! partial or malformed ones, goes to the existing plaintext path unchanged, so
//! the host's own timers and decoder keep their current behavior.
//!
//! Classification blocks the accept loop for at most `CLASSIFY_DEADLINE`. A TLS
//! `ClientHello` arrives at once; a client that sends nothing is handed to the
//! plaintext path after the deadline, so its delay is bounded.

use std::io::ErrorKind;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::super::control_protocol::{ControlProtocol, classify_control_prefix};
use super::super::secure_admission::{SECURE_ADMISSION_GENERATION, admit_listener};
use super::super::secure_listener_proof::ProofExchangeError;
use super::super::{TransportChannel, TransportError, TransportErrorKind, TransportEvent};
use super::peer_registry::PeerRegistry;
use super::secure_peer::register_secure_peer;
use super::shared::send_event;

/// Longest a new connection may take to reveal its protocol.
const CLASSIFY_DEADLINE: Duration = Duration::from_millis(250);
const CLASSIFY_POLL: Duration = Duration::from_millis(5);

/// What the accept loop should do with a routed socket.
pub(super) enum Routed {
    /// Continue on the existing plaintext registration path.
    Plaintext(TcpStream),
    /// The connection was handled here (admitted or refused).
    Handled,
}

/// Routes one accepted socket. `secure_in_flight` bounds concurrent TLS
/// admissions by `config.max_peers`.
pub(super) fn route_control_connection(
    stream: TcpStream,
    remote: SocketAddr,
    registry: &PeerRegistry,
    secure_in_flight: &Arc<AtomicUsize>,
) -> Routed {
    // A failed peek is not a TLS handshake, so it takes the plaintext path,
    // which reports the socket error the way it always has.
    let protocol = classify_peer(&stream).unwrap_or(ControlProtocol::NeedMore);
    match protocol {
        ControlProtocol::Plaintext | ControlProtocol::NeedMore | ControlProtocol::Unknown => {
            if stream.set_read_timeout(None).is_err() {
                refuse(
                    stream,
                    remote,
                    TransportErrorKind::Io,
                    "could not restore blocking mode for the plaintext control connection",
                    registry,
                );
                return Routed::Handled;
            }
            Routed::Plaintext(stream)
        }
        ControlProtocol::Tls => {
            let Some(identity) = registry.config.secure_control.clone() else {
                refuse(
                    stream,
                    remote,
                    TransportErrorKind::Protocol,
                    "secure control is not configured on this host; TLS connection refused",
                    registry,
                );
                return Routed::Handled;
            };
            if !try_reserve(secure_in_flight, registry.config.max_peers) {
                refuse(
                    stream,
                    remote,
                    TransportErrorKind::QueueFull,
                    "too many secure admissions are already in progress",
                    registry,
                );
                return Routed::Handled;
            }
            spawn_secure_admission(stream, remote, identity, registry.clone(), secure_in_flight);
            Routed::Handled
        }
    }
}

/// Reads the first bytes without consuming them, within the classify deadline.
fn classify_peer(stream: &TcpStream) -> std::io::Result<ControlProtocol> {
    let deadline = Instant::now() + CLASSIFY_DEADLINE;
    stream.set_read_timeout(Some(CLASSIFY_DEADLINE))?;
    let mut prefix = [0_u8; 4];
    loop {
        let read = match stream.peek(&mut prefix) {
            Ok(read) => read,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(ControlProtocol::NeedMore);
            }
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(ControlProtocol::Unknown);
        }
        match classify_control_prefix(&prefix[..read]) {
            ControlProtocol::NeedMore if Instant::now() < deadline => thread::sleep(CLASSIFY_POLL),
            decided => return Ok(decided),
        }
    }
}

fn try_reserve(in_flight: &Arc<AtomicUsize>, limit: usize) -> bool {
    in_flight
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < limit).then_some(active + 1)
        })
        .is_ok()
}

struct ReservedSlot(Arc<AtomicUsize>);

impl Drop for ReservedSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the stream and registry are moved into the admission thread"
)]
fn spawn_secure_admission(
    stream: TcpStream,
    remote: SocketAddr,
    identity: Arc<super::super::secure_control_tls::HostTlsIdentity>,
    registry: PeerRegistry,
    in_flight: &Arc<AtomicUsize>,
) {
    // The slot was reserved by the caller; release it when this thread ends.
    let slot = ReservedSlot(in_flight.clone());
    let failure_counters = registry.counters.clone();
    let spawned = thread::Builder::new()
        .name("silent-disco-secure-admission".to_owned())
        .spawn(move || {
            let _slot = slot;
            // A second handle to the socket lets the host close the admitted
            // connection later; admission consumes the first one.
            let Ok(shutdown_handle) = stream.try_clone() else {
                send_rejection(
                    &registry,
                    remote,
                    TransportError::new(
                        TransportErrorKind::Io,
                        TransportChannel::Control,
                        "could not clone the admitted TLS control socket",
                    ),
                );
                return;
            };
            let session = registry.config.session_id.clone();
            let outcome = admit_listener(stream, &identity, &session, SECURE_ADMISSION_GENERATION);
            let failure = match outcome {
                Ok(admitted) => {
                    register_secure_peer(&registry, admitted, shutdown_handle, remote).err()
                }
                Err(error) => {
                    let kind = match error {
                        ProofExchangeError::Proof(_) => TransportErrorKind::Unauthorized,
                        _ => TransportErrorKind::Protocol,
                    };
                    Some(TransportError::new(
                        kind,
                        TransportChannel::Control,
                        error.to_string(),
                    ))
                }
            };
            if let Some(error) = failure {
                send_rejection(&registry, remote, error);
            }
        });
    if spawned.is_err() {
        // The closure (and with it the stream and slot) was dropped; the slot
        // is released by its Drop and the socket is closed.
        failure_counters.delivery_failure();
    }
}

fn send_rejection(registry: &PeerRegistry, remote: SocketAddr, error: TransportError) {
    send_event(
        &registry.event_sender,
        &registry.counters,
        TransportEvent::Rejected {
            channel: TransportChannel::Control,
            source: remote,
            error,
            received_at: registry.clock.now(),
        },
    );
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "taking the socket by value closes the refused connection when it is dropped"
)]
fn refuse(
    stream: TcpStream,
    remote: SocketAddr,
    kind: TransportErrorKind,
    message: &str,
    registry: &PeerRegistry,
) {
    let _ = stream.shutdown(Shutdown::Both);
    send_rejection(
        registry,
        remote,
        TransportError::new(kind, TransportChannel::Control, message),
    );
}
