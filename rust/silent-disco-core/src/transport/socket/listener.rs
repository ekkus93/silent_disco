#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::vec_init_then_push
)]
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::domain::DeviceId;
use crate::protocol::{
    ControlMessage, MAX_AUDIO_DATAGRAM_BYTES, ProtocolDecoder, ProtocolFrame, SyncRequest,
    encode_frame,
};

use super::super::secure_admission::{SECURE_ADMISSION_GENERATION, prove_over_tcp};
use super::super::secure_control_session::SecureControlSession;
use super::super::secure_frame_channel::SecureFrameChannel;
use super::super::secure_listener_proof::ProofExchangeError;
use super::super::{
    ListenerDatagramRoutes, ListenerTransportConfig, ListenerTransportNode, TransportChannel,
    TransportClock, TransportCounters, TransportDelivery, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
use super::listener_inbound::ListenerInbound;
use super::secure_control_io::{drain_writes, receive_outcome};
use super::shared::{
    ControlSender, ControlWrite, CounterState, classify_rejection, fail_loud_on_implicit_shutdown,
    join_workers, read_control_loop, recv_event, send_event, shutdown_stream, spawn_control_writer,
};

pub struct SocketListenerTransport {
    config: ListenerTransportConfig,
    local_routes: ListenerDatagramRoutes,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    control_sender: ControlSender,
    control_stream: Mutex<TcpStream>,
    sync_socket: Arc<UdpSocket>,
    /// The receiver has its own lock, held only for the duration of a
    /// receive. It is deliberately *not* the lock the send methods use:
    /// `recv_event` takes `&self` so a poll parked here cannot delay a
    /// concurrent send, which is what previously made a clock-sync probe's
    /// measured round trip include the poll's own wait.
    event_receiver: Mutex<Receiver<TransportEvent>>,
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    shutdown_complete: bool,
}

impl SocketListenerTransport {
    pub fn connect(
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, TransportError> {
        config.validate()?;
        let control_address =
            SocketAddr::new(config.endpoint.address, config.endpoint.control_port);
        let stream = TcpStream::connect_timeout(&control_address, config.operation_timeout)
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Connect,
                    TransportChannel::Control,
                    "failed to connect TCP control channel",
                    &error,
                )
            })?;
        stream.set_nodelay(true).map_err(|error| {
            TransportError::io(
                TransportErrorKind::Connect,
                TransportChannel::Control,
                "failed to configure TCP control channel",
                &error,
            )
        })?;
        let link = match config.secure_host.clone() {
            Some(target) => {
                let shutdown = stream.try_clone().map_err(|error| {
                    TransportError::io(
                        TransportErrorKind::Connect,
                        TransportChannel::Control,
                        "failed to clone listener control stream for shutdown",
                        &error,
                    )
                })?;
                let channel = prove_over_tcp(
                    stream,
                    &target.host_spki,
                    &config.session_id,
                    SECURE_ADMISSION_GENERATION,
                    target.signer.as_ref(),
                )
                .map_err(|error| proof_failure(&error))?;
                ControlLink::Secure { channel, shutdown }
            }
            None => ControlLink::Plain(stream),
        };
        let shutdown_copy = match &link {
            ControlLink::Secure { shutdown, .. } => shutdown.try_clone(),
            ControlLink::Plain(stream) => stream.try_clone(),
        }
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Connect,
                TransportChannel::Control,
                "failed to clone listener control stream for shutdown",
                &error,
            )
        })?;
        let sync_socket = Arc::new(
            UdpSocket::bind(SocketAddr::new(config.local_address, 0)).map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Bind,
                    TransportChannel::Synchronization,
                    "failed to bind listener synchronization socket",
                    &error,
                )
            })?,
        );
        let audio_socket = Arc::new(
            UdpSocket::bind(SocketAddr::new(config.local_address, 0)).map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Bind,
                    TransportChannel::Audio,
                    "failed to bind listener audio socket",
                    &error,
                )
            })?,
        );
        sync_socket
            .set_read_timeout(Some(config.io_timeout))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Synchronization,
                    "failed to configure listener synchronization timeout",
                    &error,
                )
            })?;
        audio_socket
            .set_read_timeout(Some(config.io_timeout))
            .map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Io,
                    TransportChannel::Audio,
                    "failed to configure listener audio timeout",
                    &error,
                )
            })?;
        let local_routes = ListenerDatagramRoutes {
            synchronization: sync_socket.local_addr().map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Bind,
                    TransportChannel::Synchronization,
                    "failed to query listener synchronization route",
                    &error,
                )
            })?,
            audio: audio_socket.local_addr().map_err(|error| {
                TransportError::io(
                    TransportErrorKind::Bind,
                    TransportChannel::Audio,
                    "failed to query listener audio route",
                    &error,
                )
            })?,
        };

        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(CounterState::default());
        counters.accepted();
        let (event_sender, event_receiver) = sync_channel(config.event_queue_capacity);
        let (write_sender, write_receiver) = sync_channel(config.outbound_queue_capacity);
        let control_sender =
            ControlSender::new(write_sender, config.operation_timeout, counters.clone());
        let mut workers = Vec::new();
        match link {
            ControlLink::Secure { channel, .. } => {
                let session = SecureControlSession::new(channel, config.session_id.clone());
                workers.push(spawn_secure_control_pump(
                    session,
                    write_receiver,
                    ListenerInbound::new(
                        config.clone(),
                        stop.clone(),
                        counters.clone(),
                        event_sender.clone(),
                        clock.clone(),
                    ),
                )?);
            }
            ControlLink::Plain(stream) => {
                let writer_stream = stream.try_clone().map_err(|error| {
                    TransportError::io(
                        TransportErrorKind::Connect,
                        TransportChannel::Control,
                        "failed to clone listener control stream for writer",
                        &error,
                    )
                })?;
                workers.push(spawn_control_writer(
                    writer_stream,
                    write_receiver,
                    stop.clone(),
                    counters.clone(),
                    config.io_timeout,
                    "silent-disco-listener-control-writer".to_owned(),
                )?);
                workers.push(spawn_control_reader(
                    stream,
                    config.clone(),
                    stop.clone(),
                    counters.clone(),
                    event_sender.clone(),
                    clock.clone(),
                )?);
            }
        }
        workers.push(spawn_listener_udp_receiver(
            sync_socket.clone(),
            TransportChannel::Synchronization,
            config.clone(),
            stop.clone(),
            counters.clone(),
            event_sender.clone(),
            clock.clone(),
        )?);
        workers.push(spawn_listener_udp_receiver(
            audio_socket.clone(),
            TransportChannel::Audio,
            config.clone(),
            stop.clone(),
            counters.clone(),
            event_sender,
            clock,
        )?);

        Ok(Self {
            config,
            local_routes,
            stop,
            counters,
            control_sender,
            control_stream: Mutex::new(shutdown_copy),
            sync_socket,
            event_receiver: Mutex::new(event_receiver),
            workers: Mutex::new(workers),
            shutdown_complete: false,
        })
    }
}

/// The control connection after dialing: plaintext, or TLS already proven to
/// the pinned host, with a second handle to the same socket.
enum ControlLink {
    Plain(TcpStream),
    Secure {
        channel: SecureFrameChannel,
        shutdown: TcpStream,
    },
}

fn proof_failure(error: &ProofExchangeError) -> TransportError {
    let kind = match error {
        ProofExchangeError::Proof(_) => TransportErrorKind::Unauthorized,
        _ => TransportErrorKind::Connect,
    };
    TransportError::new(kind, TransportChannel::Control, error.to_string())
}

/// Starts the TLS control pump. It owns the session and the inbound handler,
/// so it runs the same validation and ordered disconnect as the plaintext reader.
fn spawn_secure_control_pump(
    session: SecureControlSession,
    writes: Receiver<ControlWrite>,
    inbound: ListenerInbound,
) -> Result<thread::JoinHandle<()>, TransportError> {
    thread::Builder::new()
        .name("silent-disco-listener-secure-control".to_owned())
        .spawn(move || run_listener_pump(session, writes, inbound))
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Connect,
                TransportChannel::Control,
                "failed to spawn listener secure control pump",
                &error,
            )
        })
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the pump owns its session, queue and handler for its whole life"
)]
fn run_listener_pump(
    mut session: SecureControlSession,
    writes: Receiver<ControlWrite>,
    inbound: ListenerInbound,
) {
    let mut terminal_error = None;
    while !inbound.stop.load(Ordering::Acquire) {
        if let Err(error) = drain_writes(&session, &writes, &inbound.counters) {
            terminal_error = Some(error);
            break;
        }
        let Some(outcome) = receive_outcome(&mut session) else {
            continue;
        };
        if !inbound.handle(outcome, &mut terminal_error) {
            break;
        }
    }
    inbound.finish(terminal_error);
    drop(session);
}

impl ListenerTransportNode for SocketListenerTransport {
    fn local_routes(&self) -> ListenerDatagramRoutes {
        self.local_routes
    }

    fn send_control(&self, message: &ControlMessage) -> Result<TransportDelivery, TransportError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(TransportError::new(
                TransportErrorKind::ShuttingDown,
                TransportChannel::Control,
                "listener control connection is retired",
            ));
        }
        if message.session_id() != &self.config.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Control,
                "outbound control message belongs to a different session",
            ));
        }
        validate_listener_outbound(&self.config.device_id, message)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;
        match self.control_sender.send(bytes) {
            Ok(written) => {
                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
            }
            Err(error) => {
                // A timed-out control may already be in the kernel writer.
                // Retire this TCP generation rather than allowing a late Join,
                // Disconnect or Resync to arrive after the caller saw failure.
                // A fresh listener transport is required for retry.
                self.counters.delivery_failure();
                self.stop.store(true, Ordering::Release);
                drop(shutdown_stream(&self.control_stream));
                Err(error)
            }
        }
    }

    fn send_sync_request(
        &self,
        request: &SyncRequest,
    ) -> Result<TransportDelivery, TransportError> {
        // UDP clock probes must not escape a retired TCP admission generation.
        if self.stop.load(Ordering::Acquire) {
            return Err(TransportError::new(
                TransportErrorKind::ShuttingDown,
                TransportChannel::Synchronization,
                "listener control connection is retired",
            ));
        }
        if request.session_id != self.config.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                TransportChannel::Synchronization,
                "outbound synchronization request belongs to a different session",
            ));
        }
        let frame = ProtocolFrame::SyncRequest(request.clone());
        let bytes = encode_frame(&frame)
            .map_err(|error| TransportError::protocol(TransportChannel::Synchronization, &error))?;
        let destination =
            SocketAddr::new(self.config.endpoint.address, self.config.endpoint.sync_port);
        match self.sync_socket.send_to(&bytes, destination) {
            Ok(written) if written == bytes.len() => {
                self.counters
                    .datagram_sent(TransportChannel::Synchronization, written);
                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))
            }
            Ok(_) => {
                self.counters.delivery_failure();
                Err(TransportError::new(
                    TransportErrorKind::Delivery,
                    TransportChannel::Synchronization,
                    "synchronization datagram send reported a partial write",
                ))
            }
            Err(error) => {
                self.counters.delivery_failure();
                Err(TransportError::io(
                    TransportErrorKind::Delivery,
                    TransportChannel::Synchronization,
                    "failed to send synchronization datagram",
                    &error,
                ))
            }
        }
    }

    fn recv_event(&self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let receiver = self
            .event_receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recv_event(&receiver, timeout)
    }

    fn counters(&self) -> TransportCounters {
        self.counters.snapshot()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.stop.store(true, Ordering::Release);
        let stream_result = shutdown_stream(&self.control_stream);
        let join_result = join_workers(&self.workers);
        self.counters.closed();
        self.shutdown_complete = true;
        stream_result.and(join_result)
    }
}

impl Drop for SocketListenerTransport {
    fn drop(&mut self) {
        let result = self.shutdown();
        fail_loud_on_implicit_shutdown(&result, "socket listener transport");
    }
}

fn spawn_control_reader(
    stream: TcpStream,
    config: ListenerTransportConfig,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
) -> Result<thread::JoinHandle<()>, TransportError> {
    let inbound = ListenerInbound::new(config, stop, counters, event_sender, clock);
    thread::Builder::new()
        .name("silent-disco-listener-control-reader".to_owned())
        .spawn(move || {
            let mut terminal_error = None;
            read_control_loop(
                stream,
                &inbound.stop,
                inbound.config.io_timeout,
                inbound.config.operation_timeout,
                &inbound.config.session_id,
                None,
                |outcome| inbound.handle(outcome, &mut terminal_error),
            );
            inbound.finish(terminal_error);
        })
        .map_err(|error| {
            TransportError::io(
                TransportErrorKind::Connect,
                TransportChannel::Control,
                "failed to spawn listener control reader",
                &error,
            )
        })
}

fn spawn_listener_udp_receiver(
    socket: Arc<UdpSocket>,
    channel: TransportChannel,
    config: ListenerTransportConfig,
    stop: Arc<AtomicBool>,
    counters: Arc<CounterState>,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
) -> Result<thread::JoinHandle<()>, TransportError> {
    let expected_source = match channel {
        TransportChannel::Synchronization => {
            SocketAddr::new(config.endpoint.address, config.endpoint.sync_port)
        }
        TransportChannel::Audio => {
            SocketAddr::new(config.endpoint.address, config.endpoint.audio_port)
        }
        TransportChannel::Control | TransportChannel::Runtime => {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                channel,
                "listener UDP receiver requires a datagram channel",
            ));
        }
    };
    let name = format!("silent-disco-listener-{}-receiver", channel.stable_name());
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
            let host_peer = TransportPeer {
                device_id: None,
                control_address: SocketAddr::new(
                    config.endpoint.address,
                    config.endpoint.control_port,
                ),
            };
            while !stop.load(Ordering::Acquire) {
                match socket.recv_from(&mut buffer) {
                    Ok((length, source)) if length > capacity => {
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
                    }
                    Ok((length, source)) if source != expected_source => {
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
                                    "datagram was not sent by the configured host endpoint",
                                ),
                                received_at: clock.now(),
                            },
                        );
                        if length == buffer.len() {
                            counters.oversized();
                        }
                    }
                    Ok((length, source)) => {
                        let result = decoder.decode(
                            &buffer[..length],
                            crate::protocol::DecodePolicy {
                                expected_session_id: Some(&config.session_id),
                                minimum_audio_sequence: None,
                            },
                        );
                        match result {
                            Ok(frame) if listener_datagram_matches(channel, &frame) => {
                                counters.datagram_received(channel, length);
                                send_event(
                                    &event_sender,
                                    &counters,
                                    TransportEvent::FrameReceived {
                                        channel,
                                        peer: host_peer.clone(),
                                        frame,
                                        received_at: clock.now(),
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
                                            "protocol frame arrived on the wrong listener datagram channel",
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
                        ) => {}
                    Err(error) => {
                        send_event(
                            &event_sender,
                            &counters,
                            TransportEvent::Rejected {
                                channel,
                                source: expected_source,
                                error: TransportError::io(
                                    TransportErrorKind::Io,
                                    channel,
                                    "failed to receive listener datagram",
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
                "failed to spawn listener datagram receiver",
                &error,
            )
        })
}

pub(crate) fn validate_listener_outbound(
    device_id: &DeviceId,
    message: &ControlMessage,
) -> Result<(), TransportError> {
    match message {
        ControlMessage::JoinRequest(request) if &request.device.device_id == device_id => Ok(()),
        ControlMessage::Heartbeat(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::Disconnect(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::ResyncNotice(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::SynchronizationReport(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::JoinRequest(_)
        | ControlMessage::Heartbeat(_)
        | ControlMessage::Disconnect(_)
        | ControlMessage::ResyncNotice(_)
        | ControlMessage::SynchronizationReport(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "outbound listener control identity does not match this transport",
        )),
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::TrackBoundary(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "listener attempted to send a host-only control message",
        )),
    }
}

pub(crate) fn validate_listener_inbound(
    device_id: &DeviceId,
    frame: &ProtocolFrame,
) -> Result<(), TransportError> {
    let ProtocolFrame::Control(message) = frame else {
        return Err(TransportError::new(
            TransportErrorKind::Protocol,
            TransportChannel::Control,
            "non-control frame reached listener control validation",
        ));
    };
    match message {
        ControlMessage::JoinApproval(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::JoinRejection(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::Heartbeat(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::Disconnect(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::ResyncNotice(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::Heartbeat(_)
        | ControlMessage::Disconnect(_)
        | ControlMessage::ResyncNotice(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "host control message targets a different listener",
        )),
        ControlMessage::Hello(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::TrackBoundary(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => Ok(()),
        ControlMessage::JoinRequest(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "host sent a listener-only join request",
        )),
        ControlMessage::SynchronizationReport(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "host sent a listener-only synchronization report",
        )),
    }
}

fn listener_datagram_matches(channel: TransportChannel, frame: &ProtocolFrame) -> bool {
    matches!(
        (channel, frame),
        (
            TransportChannel::Synchronization,
            ProtocolFrame::SyncResponse(_)
        ) | (TransportChannel::Audio, ProtocolFrame::Audio(_))
    )
}

#[cfg(test)]
#[path = "listener/failed_control_delivery_tests.rs"]
mod failed_control_delivery_tests;
