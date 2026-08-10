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

use super::super::{
    ListenerDatagramRoutes, ListenerTransportConfig, ListenerTransportNode, TransportChannel,
    TransportClock, TransportCounters, TransportDelivery, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};
use super::shared::{
    ControlSender, CounterState, ReadControlOutcome, classify_rejection, join_workers,
    read_control_loop, recv_event, send_event, shutdown_stream, spawn_control_writer,
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
        let writer_stream = stream.try_clone().map_err(|error| {
            TransportError::io(
                TransportErrorKind::Connect,
                TransportChannel::Control,
                "failed to clone listener control stream for writer",
                &error,
            )
        })?;
        let shutdown_copy = stream.try_clone().map_err(|error| {
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

impl ListenerTransportNode for SocketListenerTransport {
    fn local_routes(&self) -> ListenerDatagramRoutes {
        self.local_routes
    }

    fn send_control(&self, message: &ControlMessage) -> Result<TransportDelivery, TransportError> {
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
                self.counters.delivery_failure();
                Err(error)
            }
        }
    }

    fn send_sync_request(
        &self,
        request: &SyncRequest,
    ) -> Result<TransportDelivery, TransportError> {
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
        drop(self.shutdown());
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
    thread::Builder::new()
        .name("silent-disco-listener-control-reader".to_owned())
        .spawn(move || {
            let mut terminal_error = None;
            let host_peer = TransportPeer {
                device_id: None,
                control_address: SocketAddr::new(
                    config.endpoint.address,
                    config.endpoint.control_port,
                ),
            };
            read_control_loop(
                stream,
                &stop,
                config.io_timeout,
                &config.session_id,
                |outcome| match outcome {
                    ReadControlOutcome::Frame(frame, bytes) => {
                        counters.frame_received(bytes);
                        match validate_listener_inbound(&config.device_id, &frame) {
                            Ok(()) => {
                                send_event(
                                    &event_sender,
                                    &counters,
                                    TransportEvent::FrameReceived {
                                        channel: TransportChannel::Control,
                                        peer: host_peer.clone(),
                                        frame,
                                        received_at: clock.now(),
                                    },
                                );
                                true
                            }
                            Err(error) => {
                                classify_rejection(&counters, &error);
                                send_event(
                                    &event_sender,
                                    &counters,
                                    TransportEvent::Rejected {
                                        channel: TransportChannel::Control,
                                        source: host_peer.control_address,
                                        error,
                                        received_at: clock.now(),
                                    },
                                );
                                false
                            }
                        }
                    }
                    ReadControlOutcome::Closed => false,
                    ReadControlOutcome::Rejected(error) => {
                        terminal_error = Some(error.clone());
                        classify_rejection(&counters, &error);
                        send_event(
                            &event_sender,
                            &counters,
                            TransportEvent::Rejected {
                                channel: TransportChannel::Control,
                                source: host_peer.control_address,
                                error,
                                received_at: clock.now(),
                            },
                        );
                        false
                    }
                },
            );
            stop.store(true, Ordering::Release);
            send_event(
                &event_sender,
                &counters,
                TransportEvent::PeerDisconnected {
                    peer: host_peer,
                    error: terminal_error,
                    received_at: clock.now(),
                },
            );
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

fn validate_listener_outbound(
    device_id: &DeviceId,
    message: &ControlMessage,
) -> Result<(), TransportError> {
    match message {
        ControlMessage::JoinRequest(request) if &request.device.device_id == device_id => Ok(()),
        ControlMessage::Heartbeat(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::Disconnect(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::ResyncNotice(value) if &value.listener_id == device_id => Ok(()),
        ControlMessage::JoinRequest(_)
        | ControlMessage::Heartbeat(_)
        | ControlMessage::Disconnect(_)
        | ControlMessage::ResyncNotice(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "outbound listener control identity does not match this transport",
        )),
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "listener attempted to send a host-only control message",
        )),
    }
}

fn validate_listener_inbound(
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
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => Ok(()),
        ControlMessage::JoinRequest(_) => Err(TransportError::new(
            TransportErrorKind::Unauthorized,
            TransportChannel::Control,
            "host sent a listener-only join request",
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
