#![allow(clippy::too_many_lines)]
use std::collections::HashMap;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{MAX_AUDIO_DATAGRAM_BYTES, ProtocolDecoder, ProtocolFrame};

use super::super::{
    HostTransportConfig, TransportChannel, TransportClock, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
use super::host::{PeerRoute, PeerState, terminal_mailbox::TerminalEventMailbox};
#[path = "host_capacity_refusal.rs"]
mod capacity_refusal;
#[path = "host_join_delivery.rs"]
pub(super) mod join_delivery;
#[path = "host_worker_startup.rs"]
pub(super) mod startup;
use super::peer_inbound::PeerInbound;
use super::peer_registry::PeerRegistry;
use super::shared::{
    CounterState, classify_rejection, read_control_loop, reap_finished_workers,
    send_authoritative_event_bounded, send_event, spawn_control_writer,
};
use startup::rollback_failed_peer_start;

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_accept_loop(
    listener: TcpListener,
    config: HostTransportConfig,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    peers: Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    terminal_mailbox: Arc<TerminalEventMailbox>,
    routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    next_peer: Arc<AtomicU64>,
) -> Result<thread::JoinHandle<()>, TransportError> {
    thread::Builder::new()
        .name("silent-disco-transport-accept".to_owned())
        .spawn(move || {
            let refusal_gate = capacity_refusal::CapacityRefusalGate::default();
            let secure_in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let registry = PeerRegistry {
                config: config.clone(),
                stop: stop.clone(),
                counters: counters.clone(),
                peers: peers.clone(),
                terminal_mailbox: terminal_mailbox.clone(),
                routes: routes.clone(),
                workers: workers.clone(),
                event_sender: event_sender.clone(),
                clock: clock.clone(),
                next_peer: next_peer.clone(),
            };
            while !stop.load(Ordering::Acquire) {
                // Reap finished peer threads even when no new connections
                // arrive, rather than retaining two handles per reconnect.
                if let Err(error) = reap_finished_workers(&workers) {
                    counters.mark_worker_panicked();
                    stop.store(true, Ordering::Release);
                    send_event(
                        &event_sender,
                        &counters,
                        TransportEvent::Rejected {
                            channel: TransportChannel::Runtime,
                            source: SocketAddr::new(config.bind_address, config.control_port),
                            error,
                            received_at: clock.now(),
                        },
                    );
                    break;
                }
                match listener.accept() {
                    Ok((stream, remote)) => {
                        if stop.load(Ordering::Acquire) {
                            drop(stream.shutdown(Shutdown::Both));
                            break;
                        }
                        // Retained terminals consume admission slots until collected.
                        if registry.is_at_capacity() {
                            counters.busy_admission_refusal();
                            counters.delivery_failure();
                            refusal_gate.spawn(stream, &config, &stop, &counters);
                            continue;
                        }
                        let stream = match super::control_dispatch::route_control_connection(
                            stream,
                            remote,
                            &registry,
                            &secure_in_flight,
                        ) {
                            super::control_dispatch::Routed::Plaintext(stream) => stream,
                            super::control_dispatch::Routed::Handled => continue,
                        };
                        if let Err(error) = register_peer(stream, remote, &registry) {
                            send_event(
                                &event_sender,
                                &counters,
                                TransportEvent::Rejected {
                                    channel: TransportChannel::Control,
                                    source: remote,
                                    error,
                                    received_at: clock.now(),
                                },
                            );
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => {
                        send_event(
                            &event_sender,
                            &counters,
                            TransportEvent::Rejected {
                                channel: TransportChannel::Control,
                                source: SocketAddr::new(config.bind_address, config.control_port),
                                error: TransportError::io(
                                    TransportErrorKind::Listen,
                                    TransportChannel::Control,
                                    "TCP control accept failed",
                                    &error,
                                ),
                                received_at: clock.now(),
                            },
                        );
                        thread::sleep(config.io_timeout);
                    }
                }
            }
        })
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to spawn TCP control accept worker",
                &error,
            )
        })
}

fn register_peer(
    stream: TcpStream,
    remote: SocketAddr,
    registry: &PeerRegistry,
) -> Result<(), TransportError> {
    let writer_stream = stream.try_clone().map_err(|error| {
        TransportError::io(
            TransportErrorKind::Listen,
            TransportChannel::Control,
            "failed to clone accepted control stream for writer",
            &error,
        )
    })?;
    let shutdown_copy = stream.try_clone().map_err(|error| {
        TransportError::io(
            TransportErrorKind::Listen,
            TransportChannel::Control,
            "failed to clone accepted control stream for shutdown",
            &error,
        )
    })?;
    let number = registry.next_peer.fetch_add(1, Ordering::AcqRel);
    let (write_sender, write_receiver) = registry.writer_queue();
    let peer = registry.new_peer(number, remote, write_sender, shutdown_copy);
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
    // The actor must observe a new TCP peer before it can process its join.
    // Only the accept worker waits, never the decoder or audio callback.
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
        // No reader/writer has started. Close the unobservable socket and
        // remove its provisional registry entry after the bounded deadline.
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

    let writer = spawn_control_writer(
        writer_stream,
        write_receiver,
        peer.stop.clone(),
        registry.counters.clone(),
        registry.config.io_timeout,
        format!("silent-disco-control-writer-{number}"),
    )
    .inspect_err(|error| {
        rollback_failed_peer_start(
            &peer,
            &registry.peers,
            &registry.event_sender,
            &registry.stop,
            &registry.counters,
            registry.clock.now(),
            registry.config.operation_timeout,
            error,
        );
    })?;
    registry
        .workers
        .lock()
        .map_err(worker_registry_error)?
        .push(writer);

    let io_timeout = registry.config.io_timeout;
    let join_timeout = registry.config.operation_timeout;
    let session_id = registry.config.session_id.clone();
    let inbound = PeerInbound::new(peer.clone(), registry.clone());
    let reader = thread::Builder::new()
        .name(format!("silent-disco-control-reader-{number}"))
        .spawn(move || {
            let mut terminal_error = None;
            read_control_loop(
                stream,
                &inbound.peer.stop,
                io_timeout,
                join_timeout,
                &session_id,
                Some((&inbound.peer.join_forwarded, join_timeout)),
                |outcome| inbound.handle(outcome, &mut terminal_error),
            );
            inbound.finish(terminal_error);
        })
        .map_err(|error| {
            let error = TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to spawn control reader",
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
            error
        })?;
    registry
        .workers
        .lock()
        .map_err(worker_registry_error)?
        .push(reader);
    if registry.stop.load(Ordering::Acquire) {
        drop(peer.close());
    }
    Ok(())
}

/// Retire only the route owned by the reader that is actually exiting.
/// Device identity alone is not sufficient: the same device may reconnect
/// before the old TCP reader finishes its asynchronous shutdown.
pub(super) fn remove_route_if_current(
    routes: &Mutex<HashMap<DeviceId, PeerRoute>>,
    device_id: &DeviceId,
    peer_number: u64,
) -> bool {
    let Ok(mut routes) = routes.lock() else {
        return false;
    };
    if routes
        .get(device_id)
        .is_some_and(|route| route.peer.number == peer_number)
    {
        routes.remove(device_id);
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_udp_receiver(
    socket: Arc<UdpSocket>,
    channel: TransportChannel,
    session_id: SessionId,
    stop: Arc<AtomicBool>,

    counters: Arc<CounterState>,
    routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    io_timeout: Duration,
) -> Result<thread::JoinHandle<()>, TransportError> {
    let name = format!("silent-disco-{}-receiver", channel.stable_name());
    thread::Builder::new()
        .name(name)
        .spawn(move || {
            let capacity = match channel {
                TransportChannel::Audio => MAX_AUDIO_DATAGRAM_BYTES,
                TransportChannel::Synchronization => 4_096,
                TransportChannel::Control | TransportChannel::Runtime => return,
            };
            let mut buffer = vec![0_u8; capacity.saturating_add(1)];
            let mut decoder = ProtocolDecoder::default();
            while !stop.load(Ordering::Acquire) {
                match socket.recv_from(&mut buffer) {
                    Ok((length, source)) => {
                        if length > capacity {
                            counters.oversized();
                            send_event(
                                &event_sender,
                                &counters,
                                TransportEvent::Rejected {
                                    channel,
                                    source,
                                    error: TransportError::new(
                                        TransportErrorKind::Protocol,
                                        channel,
                                        "datagram exceeded the maximum supported size",
                                    ),
                                    received_at: clock.now(),
                                },
                            );
                            continue;
                        }
                        let route = routes.lock().ok().and_then(|routes| {
                            routes.iter().find_map(|(device_id, route)| {
                                let expected = match channel {
                                    TransportChannel::Synchronization => {
                                        route.routes.synchronization
                                    }
                                    TransportChannel::Audio => route.routes.audio,
                                    TransportChannel::Control | TransportChannel::Runtime => {
                                        return None;
                                    }
                                };
                                (expected == source
                                    && route.peer.active.load(Ordering::Acquire)
                                    && route.peer.authorized.load(Ordering::Acquire))
                                .then(|| (device_id.clone(), route.peer.clone()))
                            })
                        });
                        let Some((device_id, peer)) = route else {
                            counters.unauthorized();
                            send_event(
                                &event_sender,
                                &counters,
                                TransportEvent::Rejected {
                                    channel,
                                    source,
                                    error: TransportError::new(
                                        TransportErrorKind::Unauthorized,
                                        channel,
                                        "datagram source is not an authorized peer route",
                                    ),
                                    received_at: clock.now(),
                                },
                            );
                            continue;
                        };
                        let result = decoder.decode(
                            &buffer[..length],
                            crate::protocol::DecodePolicy {
                                expected_session_id: Some(&session_id),
                                minimum_audio_sequence: None,
                            },
                        );
                        match result {
                            Ok(frame) if datagram_matches_channel(channel, &frame) => {
                                counters.datagram_received(channel, length);
                                let received_at = clock.now();
                                peer.mark_inbound_datagram(received_at);
                                send_event(
                                    &event_sender,
                                    &counters,
                                    TransportEvent::FrameReceived {
                                        channel,
                                        peer: TransportPeer {
                                            device_id: Some(device_id),
                                            control_address: peer.remote,
                                        },
                                        frame,
                                        received_at,
                                    },
                                );
                            }
                            Ok(_) => {
                                counters.malformed();
                                send_event(
                                    &event_sender,
                                    &counters,
                                    TransportEvent::Rejected {
                                        channel,
                                        source,
                                        error: TransportError::new(
                                            TransportErrorKind::Protocol,
                                            channel,
                                            "protocol frame arrived on the wrong datagram channel",
                                        ),
                                        received_at: clock.now(),
                                    },
                                );
                            }
                            Err(error) => {
                                let error = TransportError::protocol(channel, &error);
                                classify_rejection(&counters, &error);
                                send_event(
                                    &event_sender,
                                    &counters,
                                    TransportEvent::Rejected {
                                        channel,
                                        source,
                                        error,
                                        received_at: clock.now(),
                                    },
                                );
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        thread::sleep(io_timeout.min(Duration::from_millis(5)));
                    }
                    Err(error) => {
                        send_event(
                            &event_sender,
                            &counters,
                            TransportEvent::Rejected {
                                channel,
                                source: socket
                                    .local_addr()
                                    .unwrap_or_else(|_| SocketAddr::new(session_id_address(), 0)),
                                error: TransportError::io(
                                    TransportErrorKind::Io,
                                    channel,
                                    "failed to receive datagram",
                                    &error,
                                ),
                                received_at: clock.now(),
                            },
                        );
                    }
                }
            }
        })
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                channel,
                "failed to spawn datagram receiver",
                &error,
            )
        })
}

fn datagram_matches_channel(channel: TransportChannel, frame: &ProtocolFrame) -> bool {
    matches!(
        (channel, frame),
        (
            TransportChannel::Synchronization,
            ProtocolFrame::SyncRequest(_)
        ) | (TransportChannel::Audio, ProtocolFrame::Audio(_))
    )
}

pub(super) fn worker_registry_error<T>(_: std::sync::PoisonError<T>) -> TransportError {
    TransportError::new(
        TransportErrorKind::WorkerPanicked,
        TransportChannel::Runtime,
        "transport worker registry is poisoned",
    )
}

pub(super) fn shutting_down_error() -> TransportError {
    TransportError::new(
        TransportErrorKind::ShuttingDown,
        TransportChannel::Runtime,
        "transport runtime is shutting down",
    )
}

const fn session_id_address() -> std::net::IpAddr {
    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
}

#[cfg(test)]
#[path = "host_worker_admission_pressure_tests.rs"]
mod admission_pressure_tests;
