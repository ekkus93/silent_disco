//! TLS-admitted control peers.
//!
//! An admitted listener becomes a peer exactly like a plaintext one: it gets a
//! peer-table entry, a `PeerAccepted` event, and the shared inbound handling in
//! `peer_inbound`. A single pump thread owns the session. It writes queued
//! control frames and reads inbound ones, so the TLS channel is never shared
//! across threads.
//!
//! Scope: this wires the control path into the host actor. Datagram routes for
//! the listener and the listener's own join flow are separate steps.

use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Instant;

use super::super::secure_admission::AdmittedListener;
use super::super::secure_control_session::SecureControlSession;
use super::super::{TransportChannel, TransportError, TransportErrorKind, TransportEvent};
use super::host::PeerState;
use super::host_workers::startup::rollback_failed_peer_start;
use super::host_workers::worker_registry_error;
use super::peer_inbound::PeerInbound;
use super::peer_registry::PeerRegistry;
use super::secure_control_io::{drain_writes, receive_outcome};
use super::shared::{ControlWrite, ReadControlOutcome, send_authoritative_event_bounded};

/// Registers an admitted listener as a control peer and starts its pump.
///
/// `shutdown` is a second handle to the same socket, so the host can close the
/// connection. `admitted.channel` is dropped, which closes it, on any refusal.
///
/// # Errors
/// Returns `QueueFull` at the peer limit or when the actor cannot take the
/// `PeerAccepted` event, and an I/O error if the pump thread cannot start.
pub(super) fn register_secure_peer(
    registry: &PeerRegistry,
    admitted: AdmittedListener,
    shutdown: TcpStream,
    remote: SocketAddr,
) -> Result<(), TransportError> {
    if registry.is_at_capacity() {
        registry.counters.busy_admission_refusal();
        registry.counters.delivery_failure();
        return Err(TransportError::new(
            TransportErrorKind::QueueFull,
            TransportChannel::Control,
            "host is at its peer limit; secure listener refused",
        ));
    }
    let number = registry.next_peer.fetch_add(1, Ordering::AcqRel);
    let (write_sender, write_receiver) = registry.writer_queue();
    let verified = Some(admitted.verified);
    let peer = registry.new_peer(number, remote, write_sender, shutdown, verified);
    registry
        .peers
        .lock()
        .map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer registry is poisoned",
            )
        })?
        .insert(number, peer.clone());
    registry.counters.accepted();
    // The actor must observe the peer before its join can be processed.
    if !send_authoritative_event_bounded(
        &registry.event_sender,
        &registry.stop,
        &registry.counters,
        TransportEvent::PeerAccepted {
            peer: peer.transport_peer(),
            received_at: registry.clock.now(),
        },
        registry.config.operation_timeout,
    ) {
        drop(peer.close());
        if let Ok(mut peers) = registry.peers.lock() {
            peers.remove(&number);
        }
        registry.counters.closed();
        return Err(TransportError::new(
            TransportErrorKind::QueueFull,
            TransportChannel::Control,
            "host event queue could not accept the new peer; connection refused",
        ));
    }

    let session = SecureControlSession::new(admitted.channel, registry.config.session_id.clone());
    let pump_peer = peer.clone();
    let pump_registry = registry.clone();
    let spawned = thread::Builder::new()
        .name(format!("silent-disco-secure-control-{number}"))
        .spawn(move || run_pump(session, write_receiver, pump_peer, pump_registry));
    match spawned {
        Ok(handle) => {
            registry
                .workers
                .lock()
                .map_err(worker_registry_error)?
                .push(handle);
            Ok(())
        }
        Err(error) => {
            let error = TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to spawn secure control pump",
                &error,
            );
            rollback_failed_peer_start(
                &peer,
                &registry.peers,
                &registry.event_sender,
                &registry.stop,
                &registry.counters,
                registry.clock.now(),
                registry.config.operation_timeout,
                &error,
            );
            Err(error)
        }
    }
}

/// Owns one admitted session until it ends, then runs the shared teardown.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the pump owns its session, queue and handles for its whole life"
)]
fn run_pump(
    mut session: SecureControlSession,
    writes: Receiver<ControlWrite>,
    peer: Arc<PeerState>,
    registry: PeerRegistry,
) {
    let inbound = PeerInbound::new(peer.clone(), registry.clone());
    let mut terminal_error = None;
    let join_started = Instant::now();
    while !peer.stop.load(Ordering::Acquire) {
        if let Err(error) = drain_writes(&session, &writes, &registry.counters) {
            terminal_error = Some(error);
            break;
        }
        // An unjoined peer must not hold a slot past the admission deadline.
        if !peer.join_forwarded.load(Ordering::Acquire)
            && join_started.elapsed() >= registry.config.operation_timeout
        {
            inbound.handle(
                ReadControlOutcome::Rejected(TransportError::new(
                    TransportErrorKind::Timeout,
                    TransportChannel::Control,
                    "secure peer did not submit a join request before the admission deadline",
                )),
                &mut terminal_error,
            );
            break;
        }
        let Some(outcome) = receive_outcome(&mut session) else {
            continue;
        };
        if !inbound.handle(outcome, &mut terminal_error) {
            break;
        }
    }
    // Close the socket first so the channel worker's pending writes fail fast.
    // Dropping the session then only stops the worker.
    inbound.finish(terminal_error);
    drop(session);
}
