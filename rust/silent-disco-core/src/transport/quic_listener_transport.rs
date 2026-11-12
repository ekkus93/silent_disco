//! v4 QUIC listener transport (ADR phase 2, option 1): a
//! `ListenerTransportNode` over [`QuicListenerWorker`].
//!
//! It keeps the socket listener's contract: the same outbound identity rules,
//! the same inbound rules for host controls, and a host that sends an invalid
//! control is disconnected with the reason in `PeerDisconnected`. Control
//! travels on the connection's stream; clock probes, clock replies and audio
//! travel as DATAGRAM frames on the same connection, so there are no UDP
//! sockets to bind and `local_routes` reports the connection's own port.
//!
//! Freshness of audio after decryption is not re-checked here. Every
//! received audio frame goes to the playback path, whose jitter buffer
//! rejects a wrong session or stream (an earlier stream generation),
//! duplicates, already-played sequences and frames beyond the reorder
//! window, and whose scheduler drops frames past their deadline. Repeating
//! those rules here would make two owners of them. QUIC itself stops anyone
//! but the host from injecting or replaying datagrams.
//!
//! `recv_event` and the send methods all take `&self` and never wait on each
//! other, so a clock probe is never delayed behind a blocked receive.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::protocol::{
    ControlMessage, DecodePolicy, ProtocolDecoder, ProtocolFrame, SyncRequest, encode_frame,
};

use super::quic_listener_worker::{
    QuicListenerConfig, QuicListenerError, QuicListenerEvent, QuicListenerWorker,
};
use super::secure_admission::SECURE_ADMISSION_GENERATION;
use super::socket::{validate_listener_inbound, validate_listener_outbound};
use super::{
    ListenerDatagramRoutes, ListenerTransportConfig, ListenerTransportNode, TransportChannel,
    TransportClock, TransportCounters, TransportDelivery, TransportError, TransportErrorKind,
    TransportEvent, TransportPeer,
};

/// v4 listener transport over QUIC.
pub struct QuicListenerTransport {
    config: ListenerTransportConfig,
    worker: QuicListenerWorker,
    clock: Arc<dyn TransportClock>,
    host_peer: TransportPeer,
    local_routes: ListenerDatagramRoutes,
    /// Set once this connection must not carry anything more: a failed
    /// control send, an invalid host control, or the host closing.
    retired: AtomicBool,
    /// Why the listener closed the connection itself, reported in the
    /// `PeerDisconnected` that follows.
    terminal_error: Mutex<Option<TransportError>>,
    counters: Mutex<TransportCounters>,
    shutdown_complete: bool,
}

impl std::fmt::Debug for QuicListenerTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuicListenerTransport")
            .field("host", &self.host_peer.control_address)
            .finish_non_exhaustive()
    }
}

fn worker_error(channel: TransportChannel, error: &QuicListenerError) -> TransportError {
    let kind = match error {
        QuicListenerError::Connect(_) => TransportErrorKind::Connect,
        QuicListenerError::Send(_) => TransportErrorKind::Delivery,
        QuicListenerError::Stopped => TransportErrorKind::ShuttingDown,
    };
    TransportError::new(kind, channel, error.to_string())
}

fn retired_error(channel: TransportChannel) -> TransportError {
    TransportError::new(
        TransportErrorKind::ShuttingDown,
        channel,
        "listener control connection is retired",
    )
}

impl QuicListenerTransport {
    /// Connects to the host's v4 QUIC endpoint (`endpoint.address` and
    /// `control_port`), pins its key and proves the listener key. Returns once
    /// the host has admitted the connection; joining is a separate
    /// `JoinRequest`.
    ///
    /// # Errors
    /// `InvalidConfiguration` for an invalid config or no `secure_host`
    /// (v4 cannot run without a pinned host and a listener key); `Connect`
    /// if the connection, the pin or the proof fails.
    pub fn connect(
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, TransportError> {
        config.validate()?;
        let target = config.secure_host.clone().ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "the v4 QUIC listener requires a pinned host key and a listener key",
            )
        })?;
        let host = SocketAddr::new(config.endpoint.address, config.endpoint.control_port);
        let worker = QuicListenerWorker::connect(
            QuicListenerConfig {
                host,
                host_spki: target.host_spki.clone(),
                session: config.session_id.clone(),
                generation: SECURE_ADMISSION_GENERATION,
                event_capacity: config.event_queue_capacity,
            },
            target.signer,
        )
        .map_err(|error| worker_error(TransportChannel::Control, &error))?;
        let route = SocketAddr::new(config.local_address, worker.local_addr().port());
        let counters = TransportCounters {
            accepted_control_connections: 1,
            ..TransportCounters::default()
        };
        Ok(Self {
            config,
            worker,
            clock,
            host_peer: TransportPeer {
                device_id: None,
                control_address: host,
            },
            local_routes: ListenerDatagramRoutes {
                synchronization: route,
                audio: route,
            },
            retired: AtomicBool::new(false),
            terminal_error: Mutex::new(None),
            counters: Mutex::new(counters),
            shutdown_complete: false,
        })
    }

    fn count(&self, update: impl FnOnce(&mut TransportCounters)) {
        if let Ok(mut counters) = self.counters.lock() {
            update(&mut counters);
        }
    }

    fn check_live(&self, channel: TransportChannel) -> Result<(), TransportError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(retired_error(channel));
        }
        Ok(())
    }

    /// Stops all further sends and closes the connection once what was
    /// already sent is acknowledged. A fresh transport is needed to retry.
    fn retire(&self, reason: &str, error: Option<TransportError>) {
        self.retired.store(true, Ordering::Release);
        if let Some(error) = error
            && let Ok(mut terminal) = self.terminal_error.lock()
        {
            terminal.get_or_insert(error);
        }
        let _ = self.worker.close(reason);
    }

    fn decode(
        &self,
        bytes: &[u8],
        channel: TransportChannel,
    ) -> Result<ProtocolFrame, TransportError> {
        ProtocolDecoder::default()
            .decode(
                bytes,
                DecodePolicy {
                    expected_session_id: Some(&self.config.session_id),
                    minimum_audio_sequence: None,
                },
            )
            .map_err(|error| TransportError::protocol(channel, &error))
    }

    fn classify(&self, error: &TransportError) {
        match error.kind {
            TransportErrorKind::Unauthorized => self.count(|c| c.unauthorized_frames += 1),
            TransportErrorKind::Protocol
                if error.message.contains("large") || error.message.contains("maximum") =>
            {
                self.count(|c| c.oversized_frames += 1);
            }
            TransportErrorKind::Protocol => self.count(|c| c.malformed_frames += 1),
            _ => {}
        }
    }

    fn rejected(&self, channel: TransportChannel, error: TransportError) -> TransportEvent {
        TransportEvent::Rejected {
            channel,
            source: self.host_peer.control_address,
            error,
            received_at: self.clock.now(),
        }
    }

    fn translate(&self, event: QuicListenerEvent) -> TransportEvent {
        match event {
            QuicListenerEvent::Control(bytes) => self.translate_control(&bytes),
            QuicListenerEvent::Datagram(bytes) => self.translate_datagram(&bytes),
            QuicListenerEvent::Closed(reason) => {
                let closed_by_us = self.retired.swap(true, Ordering::AcqRel);
                self.count(|c| c.closed_control_connections += 1);
                let recorded = self
                    .terminal_error
                    .lock()
                    .ok()
                    .and_then(|mut terminal| terminal.take());
                // A host close is a failure of the session; our own close
                // reports why we closed, or nothing for a deliberate one.
                let error = recorded.or_else(|| {
                    (!closed_by_us).then(|| {
                        TransportError::new(
                            TransportErrorKind::Io,
                            TransportChannel::Control,
                            reason,
                        )
                    })
                });
                TransportEvent::PeerDisconnected {
                    peer: self.host_peer.clone(),
                    error,
                    received_at: self.clock.now(),
                }
            }
        }
    }

    fn translate_control(&self, bytes: &[u8]) -> TransportEvent {
        let channel = TransportChannel::Control;
        self.count(|c| {
            c.control_frames_received += 1;
            c.bytes_received += bytes.len() as u64;
        });
        let checked = self.decode(bytes, channel).and_then(|frame| {
            validate_listener_inbound(&self.config.device_id, &frame).map(|()| frame)
        });
        match checked {
            Ok(frame) => TransportEvent::FrameReceived {
                channel,
                peer: self.host_peer.clone(),
                frame,
                received_at: self.clock.now(),
            },
            Err(error) => {
                // As on the socket path, a host that sends an invalid control
                // is disconnected, and the reason travels with the disconnect.
                self.classify(&error);
                self.retire("invalid host control frame", Some(error.clone()));
                self.rejected(channel, error)
            }
        }
    }

    fn translate_datagram(&self, bytes: &[u8]) -> TransportEvent {
        match self.decode(bytes, TransportChannel::Audio) {
            Ok(frame @ ProtocolFrame::Audio(_)) => {
                self.count(|c| {
                    c.audio_datagrams_received += 1;
                    c.bytes_received += bytes.len() as u64;
                });
                self.frame(TransportChannel::Audio, frame)
            }
            Ok(frame @ ProtocolFrame::SyncResponse(_)) => {
                self.count(|c| {
                    c.sync_datagrams_received += 1;
                    c.bytes_received += bytes.len() as u64;
                });
                self.frame(TransportChannel::Synchronization, frame)
            }
            Ok(_) => {
                self.count(|c| c.malformed_frames += 1);
                self.rejected(
                    TransportChannel::Audio,
                    TransportError::new(
                        TransportErrorKind::Protocol,
                        TransportChannel::Audio,
                        "host datagram is neither audio nor a clock reply",
                    ),
                )
            }
            Err(error) => {
                self.classify(&error);
                self.rejected(TransportChannel::Audio, error)
            }
        }
    }

    fn frame(&self, channel: TransportChannel, frame: ProtocolFrame) -> TransportEvent {
        TransportEvent::FrameReceived {
            channel,
            peer: self.host_peer.clone(),
            frame,
            received_at: self.clock.now(),
        }
    }
}

impl ListenerTransportNode for QuicListenerTransport {
    fn local_routes(&self) -> ListenerDatagramRoutes {
        self.local_routes
    }

    fn send_control(&self, message: &ControlMessage) -> Result<TransportDelivery, TransportError> {
        let channel = TransportChannel::Control;
        self.check_live(channel)?;
        if message.session_id() != &self.config.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "outbound control message belongs to a different session",
            ));
        }
        validate_listener_outbound(&self.config.device_id, message)?;
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))
            .map_err(|error| TransportError::protocol(channel, &error))?;
        let length = bytes.len() as u64;
        match self.worker.send_control(bytes) {
            Ok(()) => {
                self.count(|c| {
                    c.control_frames_sent += 1;
                    c.bytes_sent += length;
                });
                TransportDelivery::new(1, 1, 0, length)
            }
            Err(error) => {
                // The frame may still reach the host. Retire this connection
                // so a late Join, Disconnect or Resync cannot arrive after the
                // caller saw a failure, as the socket listener does.
                self.count(|c| c.delivery_failures += 1);
                let error = worker_error(channel, &error);
                self.retire("control delivery failed", Some(error.clone()));
                Err(error)
            }
        }
    }

    fn send_sync_request(
        &self,
        request: &SyncRequest,
    ) -> Result<TransportDelivery, TransportError> {
        let channel = TransportChannel::Synchronization;
        self.check_live(channel)?;
        if request.session_id != self.config.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "outbound synchronization request belongs to a different session",
            ));
        }
        let bytes = encode_frame(&ProtocolFrame::SyncRequest(request.clone()))
            .map_err(|error| TransportError::protocol(channel, &error))?;
        let length = bytes.len() as u64;
        match self.worker.send_datagram(bytes) {
            Ok(()) => {
                self.count(|c| {
                    c.sync_datagrams_sent += 1;
                    c.bytes_sent += length;
                });
                TransportDelivery::new(1, 1, 0, length)
            }
            Err(error) => {
                self.count(|c| c.delivery_failures += 1);
                Err(worker_error(channel, &error))
            }
        }
    }

    fn recv_event(&self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        match self.worker.recv_event(timeout) {
            Ok(Some(event)) => Ok(self.translate(event)),
            Ok(None) => Err(TransportError::timeout(
                TransportChannel::Runtime,
                "timed out waiting for a transport event",
            )),
            Err(error) => Err(worker_error(TransportChannel::Runtime, &error)),
        }
    }

    fn counters(&self) -> TransportCounters {
        let mut counters = self
            .counters
            .lock()
            .map_or_else(|_| TransportCounters::default(), |c| *c);
        // Inbound datagrams the worker dropped on a full event queue.
        counters.best_effort_event_drops += self.worker.dropped_datagrams();
        counters
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if !self.shutdown_complete {
            self.retired.store(true, Ordering::Release);
            self.worker.shutdown();
            self.shutdown_complete = true;
        }
        Ok(())
    }
}

impl Drop for QuicListenerTransport {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
