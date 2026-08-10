#![allow(clippy::too_many_lines)]
use std::collections::HashMap;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, MAX_AUDIO_DATAGRAM_BYTES, ProtocolDecoder, ProtocolFrame};

use super::super::{
    HostTransportConfig, TransportChannel, TransportClock, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
use super::host::{PeerRoute, PeerState};
use super::shared::{
    ControlSender, CounterState, ReadControlOutcome, classify_rejection, read_control_loop,
    send_event, shutdown_stream, spawn_control_writer,
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
        active: AtomicBool::new(true),
        consecutive_failures: AtomicU32::new(0),
        sender: ControlSender::new(write_sender, config.operation_timeout, counters.clone()),
        stop: peer_stop.clone(),
        shutdown_stream: Mutex::new(shutdown_copy),
        // Registration time, not zero -- a freshly authorized peer that
        // hasn't had a chance to send its first sync probe yet must not
        // read as already stale (see the field's own doc comment).
        last_inbound_millis: AtomicU64::new(clock.now().get()),
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
    send_event(
        event_sender,
        counters,
        TransportEvent::PeerAccepted {
            peer: peer.transport_peer(),
            received_at: clock.now(),
        },
    );

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
                                send_event(
                                    &event_for_reader,
                                    &counters_for_reader,
                                    TransportEvent::FrameReceived {
                                        channel: TransportChannel::Control,
                                        peer: reader_peer.transport_peer(),
                                        frame,
                                        received_at,
                                    },
                                );
                                true
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
                drop(
                    routes_for_reader
                        .lock()
                        .ok()
                        .and_then(|mut routes| routes.remove(&device_id)),
                );
            }
            counters_for_reader.closed();
            send_event(
                &event_for_reader,
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
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
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
                                peer.mark_inbound_activity(received_at);
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
