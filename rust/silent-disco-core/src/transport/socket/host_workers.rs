#![allow(clippy::too_many_lines)]
use std::collections::HashMap;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{
    ControlMessage, JoinRejection, MAX_AUDIO_DATAGRAM_BYTES, ProtocolDecoder, ProtocolFrame,
    encode_frame,
};

use super::super::{
    HostTransportConfig, TransportChannel, TransportClock, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
use super::host::{PeerRoute, PeerState};
use super::shared::{
    ControlSender, CounterState, ReadControlOutcome, classify_rejection, read_control_loop,
    send_authoritative_event, send_authoritative_event_bounded, send_event, shutdown_stream,
    spawn_control_writer,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_accept_loop(
    listener: TcpListener,
    config: HostTransportConfig,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    peers: Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    routes: Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    next_peer: Arc<AtomicU64>,
) -> Result<thread::JoinHandle<()>, TransportError> {
    thread::Builder::new()
        .name("silent-disco-transport-accept".to_owned())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, remote)) => {
                        if stop.load(Ordering::Acquire) {
                            drop(stream.shutdown(Shutdown::Both));
                            break;
                        }
                        let current_count =
                            peers.lock().map_or(config.max_peers, |peers| peers.len());
                        if current_count >= config.max_peers {
                            drop(stream.shutdown(Shutdown::Both));
                            counters.delivery_failure();
                            continue;
                        }
                        if let Err(error) = register_peer(
                            stream,
                            remote,
                            &config,
                            &stop,
                            &counters,
                            &peers,
                            &routes,
                            &workers,
                            &event_sender,
                            &clock,
                            &next_peer,
                        ) {
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

#[allow(clippy::too_many_arguments)]
fn register_peer(
    stream: TcpStream,
    remote: SocketAddr,
    config: &HostTransportConfig,
    global_stop: &Arc<AtomicBool>,
    counters: &Arc<CounterState>,
    peers: &Arc<Mutex<HashMap<u64, Arc<PeerState>>>>,
    routes: &Arc<Mutex<HashMap<DeviceId, PeerRoute>>>,
    workers: &Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    event_sender: &SyncSender<TransportEvent>,
    clock: &Arc<dyn TransportClock>,
    next_peer: &AtomicU64,
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
    let number = next_peer.fetch_add(1, Ordering::AcqRel);
    let peer_stop = Arc::new(AtomicBool::new(false));
    let (write_sender, write_receiver) = sync_channel(config.outbound_queue_capacity);
    let peer = Arc::new(PeerState {
        number,
        remote,
        identity: Mutex::new(None),
        authorized: AtomicBool::new(false),
        join_forwarded: AtomicBool::new(false),
        active: AtomicBool::new(true),
        consecutive_failures: AtomicU32::new(0),
        sender: ControlSender::new(write_sender, config.operation_timeout, counters.clone()),
        stop: peer_stop.clone(),
        shutdown_stream: Mutex::new(shutdown_copy),
        // Registration time, not zero -- a freshly authorized peer that
        // hasn't had a chance to send its first sync probe yet must not
        // read as already stale (see the field's own doc comment).
        last_inbound_millis: AtomicU64::new(clock.now().get()),
        silence_armed: AtomicBool::new(false),
    });
    peers
        .lock()
        .map_err(|_| {
            TransportError::new(
                TransportErrorKind::WorkerPanicked,
                TransportChannel::Runtime,
                "peer registry is poisoned",
            )
        })?
        .insert(number, peer.clone());
    counters.accepted();
    // The actor must observe a new TCP peer before it can process its join.
    // UDP telemetry can saturate this queue; do not silently drop acceptance.
    // Only the accept worker waits, never the decoder or audio callback.
    if !send_authoritative_event_bounded(
        event_sender,
        global_stop,
        counters,
        TransportEvent::PeerAccepted {
            peer: peer.transport_peer(),
            received_at: clock.now(),
        },
        config.operation_timeout,
    ) {
        // No reader/writer has started. Close the unobservable socket and
        // remove its provisional registry entry after the bounded deadline.
        drop(peer.close());
        if let Ok(mut registry) = peers.lock() {
            registry.remove(&number);
        }
        counters.closed();
        return Err(TransportError::new(
            TransportErrorKind::QueueFull,
            TransportChannel::Control,
            "host event queue could not accept the new peer; connection refused",
        ));
    }

    let writer = spawn_control_writer(
        writer_stream,
        write_receiver,
        peer_stop,
        counters.clone(),
        config.io_timeout,
        format!("silent-disco-control-writer-{number}"),
    )?;
    workers.lock().map_err(worker_registry_error)?.push(writer);

    let session_id = config.session_id.clone();
    let counters_for_reader = counters.clone();
    let peers_for_reader = peers.clone();
    let routes_for_reader = routes.clone();
    let event_for_reader = event_sender.clone();
    let clock_for_reader = clock.clone();
    let reader_peer = peer.clone();
    let io_timeout = config.io_timeout;
    let shutdown_for_reader = global_stop.clone();
    let reader = thread::Builder::new()
        .name(format!("silent-disco-control-reader-{number}"))
        .spawn(move || {
            let mut terminal_error = None;
            read_control_loop(
                stream,
                &reader_peer.stop,
                io_timeout,
                &session_id,
                |outcome| match outcome {
                    ReadControlOutcome::Frame(frame, bytes) => {
                        counters_for_reader.frame_received(bytes);
                        match validate_host_frame(&reader_peer, &frame) {
                            Ok(()) => {
                                let received_at = clock_for_reader.now();
                                reader_peer.mark_inbound_activity(received_at);
                                // A join accepted by the TCP reader must
                                // reach the host actor, or receive an explicit
                                // busy rejection. Dropping it from a full
                                // event queue would leave the listener waiting
                                // indefinitely for an approval.
                                let pending_join = match &frame {
                                    ProtocolFrame::Control(ControlMessage::JoinRequest(request)) => {
                                        Some(request.device.device_id.clone())
                                    }
                                    _ => None,
                                };
                                let event = TransportEvent::FrameReceived {
                                    channel: TransportChannel::Control,
                                    peer: reader_peer.transport_peer(),
                                    frame,
                                    received_at,
                                };
                                if pending_join.is_some()
                                    && reader_peer.join_forwarded.swap(true, Ordering::AcqRel)
                                {
                                    // One accepted join per TCP connection. A
                                    // retransmitted JoinRequest must not create
                                    // another host-actor approval or enrollment.
                                    // A rejected first delivery closes this
                                    // connection; a fresh TCP generation may
                                    // submit a new request.
                                    true
                                } else if let Some(listener_id) = pending_join {
                                    forward_join_or_refuse(
                                        &event_for_reader,
                                        &counters_for_reader,
                                        event,
                                        || {
                                            // Bounded native writer; a completed
                                            // reply precedes socket teardown.
                                            let refusal = ProtocolFrame::Control(
                                                ControlMessage::JoinRejection(JoinRejection {
                                                    session_id: session_id.clone(),
                                                    listener_id,
                                                    reason: "Host busy: join queue full; reconnect and retry"
                                                        .to_owned(),
                                                }),
                                            );
                                            if let Ok(bytes) = encode_frame(&refusal) {
                                                drop(reader_peer.sender.send(bytes));
                                            }
                                        },
                                    )
                                } else {
                                    send_authoritative_event(
                                        &event_for_reader,
                                        &reader_peer.stop,
                                        &counters_for_reader,
                                        event,
                                    )
                                }
                            }
                            Err(error) => {
                                classify_rejection(&counters_for_reader, &error);
                                send_event(
                                    &event_for_reader,
                                    &counters_for_reader,
                                    TransportEvent::Rejected {
                                        channel: TransportChannel::Control,
                                        source: reader_peer.remote,
                                        error,
                                        received_at: clock_for_reader.now(),
                                    },
                                );
                                false
                            }
                        }
                    }
                    ReadControlOutcome::Closed => false,
                    ReadControlOutcome::Rejected(error) => {
                        terminal_error = Some(error.clone());
                        classify_rejection(&counters_for_reader, &error);
                        send_event(
                            &event_for_reader,
                            &counters_for_reader,
                            TransportEvent::Rejected {
                                channel: TransportChannel::Control,
                                source: reader_peer.remote,
                                error,
                                received_at: clock_for_reader.now(),
                            },
                        );
                        false
                    }
                },
            );
            reader_peer.active.store(false, Ordering::Release);
            reader_peer.stop.store(true, Ordering::Release);
            drop(shutdown_stream(&reader_peer.shutdown_stream));
            drop(
                peers_for_reader
                    .lock()
                    .ok()
                    .and_then(|mut peers| peers.remove(&reader_peer.number)),
            );
            if let Some(device_id) = reader_peer.device_id() {
                // A replacement socket for the same device may have been
                // authorized while this retired reader was unwinding. Never
                // let its cleanup remove the replacement's live UDP route.
                let _ = remove_route_if_current(&routes_for_reader, &device_id, reader_peer.number);
            }
            counters_for_reader.closed();
            // A disconnect is authoritative: losing it under UDP telemetry
            // pressure leaves a ghost listener in the host UI/actor. Wait for
            // bounded queue capacity unless the entire host is shutting down.
            // The per-peer stop flag is already set during reader cleanup.
            send_authoritative_event(
                &event_for_reader,
                &shutdown_for_reader,
                &counters_for_reader,
                TransportEvent::PeerDisconnected {
                    peer: reader_peer.transport_peer(),
                    error: terminal_error,
                    received_at: clock_for_reader.now(),
                },
            );
        })
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Listen,
                TransportChannel::Control,
                "failed to spawn control reader",
                &error,
            )
        })?;
    workers.lock().map_err(worker_registry_error)?.push(reader);
    if global_stop.load(Ordering::Acquire) {
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

/**
 * Preserve the host actor's bounded event queue without silently accepting
 * an admission request the actor will never see. Only join requests use this
 * path; other control/telemetry events retain their existing classification.
 * The refusal callback runs on the TCP worker, never the audio callback.
 */
fn forward_join_or_refuse<T>(
    sender: &SyncSender<T>,
    counters: &CounterState,
    event: T,
    refuse: impl FnOnce(),
) -> bool {
    match sender.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            counters.queue_overflow();
            counters.delivery_failure();
            refuse();
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn validate_host_frame(peer: &PeerState, frame: &ProtocolFrame) -> Result<(), TransportError> {
    let ProtocolFrame::Control(message) = frame else {
        return Err(TransportError::new(
            TransportErrorKind::Protocol,
            TransportChannel::Control,
            "non-control frame reached host control validation",
        ));
    };
    match message {
        ControlMessage::JoinRequest(request) => {
            let mut identity = peer.identity.lock().map_err(|_| {
                TransportError::new(
                    TransportErrorKind::WorkerPanicked,
                    TransportChannel::Control,
                    "peer identity is poisoned",
                )
            })?;
            if identity
                .as_ref()
                .is_some_and(|current| current != &request.device.device_id)
            {
                return Err(TransportError::new(
                    TransportErrorKind::Unauthorized,
                    TransportChannel::Control,
                    "control peer attempted to change device identity",
                ));
            }
            *identity = Some(request.device.device_id.clone());
            Ok(())
        }
        ControlMessage::Heartbeat(value) => require_authorized_identity(peer, &value.listener_id),
        ControlMessage::Disconnect(value) => require_authorized_identity(peer, &value.listener_id),
        ControlMessage::ResyncNotice(value) => {
            require_authorized_identity(peer, &value.listener_id)
        }
        ControlMessage::SynchronizationReport(value) => {
            require_authorized_identity(peer, &value.listener_id)
        }
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::TrackBoundary(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "listener sent a host-only control message",
        )),
    }
}

fn require_authorized_identity(
    peer: &PeerState,
    device_id: &DeviceId,
) -> Result<(), TransportError> {
    if !peer.authorized.load(Ordering::Acquire) || peer.device_id().as_ref() != Some(device_id) {
        return Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "control frame was not sent by the authorized peer identity",
        ));
    }
    Ok(())
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
            ProtocolFrame::SyncRequest(_) | ProtocolFrame::SyncResponse(_)
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
mod admission_pressure_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn saturated_join_queue_is_refused_without_silent_acceptance() {
        let (sender, receiver) = sync_channel(1);
        let counters = CounterState::default();
        sender.send(1_u8).expect("fill queue");
        let refused = AtomicBool::new(false);
        assert!(!forward_join_or_refuse(&sender, &counters, 2_u8, || {
            refused.store(true, Ordering::Release);
        }));
        assert!(refused.load(Ordering::Acquire));
        assert_eq!(receiver.try_recv().expect("original event"), 1);
        assert!(receiver.try_recv().is_err());
        assert_eq!(counters.snapshot().queue_overflows, 1);
        assert_eq!(counters.snapshot().delivery_failures, 1);
    }

    #[test]
    fn open_join_queue_delivers_once_without_refusing() {
        let (sender, receiver) = sync_channel(1);
        let counters = CounterState::default();
        assert!(forward_join_or_refuse(&sender, &counters, 17_u8, || {
            panic!("join was delivered, must not send busy response");
        }));
        assert_eq!(receiver.try_recv().expect("delivered join"), 17);
        assert_eq!(counters.snapshot().queue_overflows, 0);
        assert_eq!(counters.snapshot().delivery_failures, 0);
    }
}
