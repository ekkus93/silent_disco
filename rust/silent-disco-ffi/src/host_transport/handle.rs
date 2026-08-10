use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use silent_disco_core::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, Disconnect, JoinApproval, JoinRejection, Pause,
    ProtocolFrame, Stop, StreamStart, SyncResponse,
};
use silent_disco_core::transport::{
    DEFAULT_IO_TIMEOUT, DEFAULT_MAX_CONSECUTIVE_FAILURES, DEFAULT_MAX_TRANSPORT_PEERS,
    DEFAULT_OPERATION_TIMEOUT, DEFAULT_PEER_INBOUND_SILENCE_TIMEOUT,
    DEFAULT_TRANSPORT_EVENT_CAPACITY, DEFAULT_TRANSPORT_QUEUE_CAPACITY, HostTransportConfig,
    HostTransportNode, SystemTransportClock, TransportChannel, TransportClock, TransportDelivery,
    TransportEvent, TransportFactory, production_transport_factory,
};

use super::types::{
    FfiHostTransportCounters, FfiHostTransportDelivery, FfiHostTransportError,
    FfiHostTransportEvent,
};

/// Opaque, bounded handle around the shared Rust `SocketHostTransport`.
///
/// Mirrors `FfiListenerTransportHandle`'s scope and doc-comment intent: it
/// does not route through the authoritative `CoreActorRuntime` -- Kotlin
/// observes the typed events this handle exposes and bridges them into the
/// existing `FfiCoreHandle` actor bridge itself (`submit_join_request`,
/// `submit_listener_disconnected`, ...). Synchronization and audio are now
/// surfaced (`SyncRequestReceived`, `send_sync_response`, `broadcast_audio`).
#[derive(uniffi::Object)]
pub struct FfiHostTransportHandle {
    inner: Mutex<Option<Inner>>,
}

impl std::fmt::Debug for FfiHostTransportHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FfiHostTransportHandle")
    }
}

struct Inner {
    transport: Box<dyn HostTransportNode>,
    session_id: SessionId,
}

#[allow(
    clippy::missing_errors_doc,
    reason = "all exported methods use the uniform validated FfiHostTransportError contract"
)]
#[uniffi::export]
impl FfiHostTransportHandle {
    /// Binds the shared production socket runtime for a hosted session.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned String parameters at the foreign boundary"
    )]
    #[uniffi::constructor]
    pub fn bind(
        local_address: String,
        control_port: u16,
        sync_port: u16,
        audio_port: u16,
        session_id: String,
    ) -> Result<Arc<Self>, FfiHostTransportError> {
        let bind_address: IpAddr = local_address.parse().map_err(|_| {
            FfiHostTransportError::InvalidConfiguration(
                "local bind address is malformed".to_owned(),
            )
        })?;
        let session_id = SessionId::new(session_id)
            .map_err(|error| FfiHostTransportError::InvalidConfiguration(error.to_string()))?;
        let config = HostTransportConfig {
            session_id: session_id.clone(),
            bind_address,
            control_port,
            sync_port,
            audio_port,
            outbound_queue_capacity: DEFAULT_TRANSPORT_QUEUE_CAPACITY,
            event_queue_capacity: DEFAULT_TRANSPORT_EVENT_CAPACITY,
            max_peers: DEFAULT_MAX_TRANSPORT_PEERS,
            io_timeout: DEFAULT_IO_TIMEOUT,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
            max_consecutive_failures: DEFAULT_MAX_CONSECUTIVE_FAILURES,
            peer_inbound_silence_timeout: DEFAULT_PEER_INBOUND_SILENCE_TIMEOUT,
        };
        let clock: Arc<dyn TransportClock> = Arc::new(SystemTransportClock::default());
        let transport = production_transport_factory().bind_host(config, clock)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(Inner {
                transport,
                session_id,
            })),
        }))
    }

    /// Sends a join approval to one identified listener.
    pub fn send_join_approval(
        &self,
        listener_id: String,
        trusted_for_future: bool,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let message = ControlMessage::JoinApproval(JoinApproval {
                session_id: inner.session_id.clone(),
                listener_id: listener_id.clone(),
                trusted_for_future,
            });
            let delivery = inner
                .transport
                .send_pending_control(&listener_id, &message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Sends a join rejection to one identified listener.
    pub fn send_join_rejection(
        &self,
        listener_id: String,
        reason: String,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let message = ControlMessage::JoinRejection(JoinRejection {
                session_id: inner.session_id.clone(),
                listener_id: listener_id.clone(),
                reason,
            });
            let delivery = inner
                .transport
                .send_pending_control(&listener_id, &message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Sends an explicit disconnect to one identified listener.
    ///
    /// Does not additionally force-close the TCP connection: this peer was
    /// never authorized (control-plane only, see the type doc comment), so
    /// the lower-level `disconnect_peer` operates on a different, authorized-
    /// peer registry and can't find it. The listener closes its own end on
    /// receiving this message; the host's accept loop detects that closure
    /// and reports it as an ordinary `PeerDisconnected` event.
    pub fn disconnect_peer(
        &self,
        listener_id: String,
        reason: String,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let message = ControlMessage::Disconnect(Disconnect {
                session_id: inner.session_id.clone(),
                listener_id: listener_id.clone(),
                reason,
            });
            let delivery = inner
                .transport
                .send_pending_control(&listener_id, &message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Broadcasts a stream-start announcement to every connected listener.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the wire message's own field count"
    )]
    pub fn broadcast_stream_start(
        &self,
        stream_id: String,
        host_start_time_ms: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::StreamStart(StreamStart {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                host_start_time_ms: MonotonicMillis::new(host_start_time_ms),
                sample_rate,
                channels,
                samples_per_packet,
            });
            let delivery = inner.transport.broadcast_control(&message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Broadcasts a pause announcement to every connected listener.
    pub fn broadcast_pause(
        &self,
        stream_id: String,
        host_pause_time_ms: u64,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::Pause(Pause {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                host_pause_time_ms: MonotonicMillis::new(host_pause_time_ms),
            });
            let delivery = inner.transport.broadcast_control(&message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Broadcasts a stop announcement to every connected listener.
    pub fn broadcast_stop(
        &self,
        stream_id: String,
        host_stop_time_ms: u64,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::Stop(Stop {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                host_stop_time_ms: MonotonicMillis::new(host_stop_time_ms),
            });
            let delivery = inner.transport.broadcast_control(&message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Authorizes one listener for synchronization/audio datagram routing.
    ///
    /// Call this once a join is approved, using the `sync_port`/`audio_port`
    /// carried on that listener's `JoinRequestReceived` event -- until this
    /// is called for a listener, the host will not accept sync requests from
    /// it or route audio/sync datagrams to it, regardless of approval state.
    pub fn authorize_listener(
        &self,
        listener_id: String,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            inner
                .transport
                .authorize_peer_ports(&listener_id, sync_port, audio_port)?;
            Ok(())
        })
    }

    /// Broadcasts a clock-sync response echoing one listener's probe.
    ///
    /// The host transport currently only broadcasts sync responses to every
    /// authorized peer (there is no targeted per-listener datagram send) --
    /// every listener other than the one whose `correlation_id` this echoes
    /// discards it as a correlation mismatch, so this is correct, if
    /// wasteful with many listeners.
    pub fn send_sync_response(
        &self,
        correlation_id: u64,
        t1_listener_send_elapsed_ms: u64,
        t2_host_receive_elapsed_ms: u64,
        t3_host_send_elapsed_ms: u64,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let frame = ProtocolFrame::SyncResponse(SyncResponse {
                session_id: inner.session_id.clone(),
                correlation_id,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(t1_listener_send_elapsed_ms),
                t2_host_receive_elapsed_ms: MonotonicMillis::new(t2_host_receive_elapsed_ms),
                t3_host_send_elapsed_ms: MonotonicMillis::new(t3_host_send_elapsed_ms),
            });
            let delivery = inner.transport.broadcast_sync(&frame)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Broadcasts one audio datagram to every connected listener.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the wire datagram's own field count"
    )]
    pub fn broadcast_audio(
        &self,
        stream_id: String,
        sequence: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        first_sample_index: u64,
        host_presentation_time_ms: u64,
        payload: Vec<u8>,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let frame = ProtocolFrame::Audio(AudioDatagram {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                sequence: PacketSequence::new(sequence),
                codec: AudioCodec::PcmS16Le,
                sample_rate,
                channels,
                samples_per_packet,
                first_sample_index: SampleIndex::new(first_sample_index),
                host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
                payload,
            });
            let delivery = inner.transport.broadcast_audio(&frame)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Waits up to `timeout_ms` for the next transport event.
    ///
    /// Returns `None` when the bounded wait elapses without an event.
    pub fn poll_event(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<FfiHostTransportEvent>, FfiHostTransportError> {
        let mut guard = self.lock()?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| FfiHostTransportError::Closed("transport is closed".to_owned()))?;
        match inner
            .transport
            .recv_event(Duration::from_millis(timeout_ms))
        {
            Ok(event) => Ok(map_event(event)),
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn counters(&self) -> Result<FfiHostTransportCounters, FfiHostTransportError> {
        self.with_transport(|inner| {
            let counters = inner.transport.counters();
            Ok(FfiHostTransportCounters {
                accepted_control_connections: counters.accepted_control_connections,
                closed_control_connections: counters.closed_control_connections,
                control_frames_sent: counters.control_frames_sent,
                control_frames_received: counters.control_frames_received,
                sync_datagrams_sent: counters.sync_datagrams_sent,
                sync_datagrams_received: counters.sync_datagrams_received,
                audio_datagrams_sent: counters.audio_datagrams_sent,
                audio_datagrams_received: counters.audio_datagrams_received,
                bytes_sent: counters.bytes_sent,
                bytes_received: counters.bytes_received,
                malformed_frames: counters.malformed_frames,
                oversized_frames: counters.oversized_frames,
                unauthorized_frames: counters.unauthorized_frames,
                queue_overflows: counters.queue_overflows,
                delivery_failures: counters.delivery_failures,
            })
        })
    }

    /// Idempotently shuts down and joins every transport worker.
    pub fn shutdown(&self) -> Result<(), FfiHostTransportError> {
        let mut guard = self.lock()?;
        let Some(mut inner) = guard.take() else {
            return Ok(());
        };
        inner.transport.shutdown().map_err(Into::into)
    }
}

impl FfiHostTransportHandle {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Inner>>, FfiHostTransportError> {
        self.inner
            .lock()
            .map_err(|_| FfiHostTransportError::Closed("transport lock was poisoned".to_owned()))
    }

    fn with_transport<T>(
        &self,
        action: impl FnOnce(&mut Inner) -> Result<T, FfiHostTransportError>,
    ) -> Result<T, FfiHostTransportError> {
        let mut guard = self.lock()?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| FfiHostTransportError::Closed("transport is closed".to_owned()))?;
        action(inner)
    }
}

impl Drop for FfiHostTransportHandle {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(mut inner) = guard.take()
        {
            drop(inner.transport.shutdown());
        }
    }
}

fn device_id(value: String) -> Result<DeviceId, FfiHostTransportError> {
    DeviceId::new(value)
        .map_err(|error| FfiHostTransportError::InvalidConfiguration(error.to_string()))
}

fn parse_stream_id(value: String) -> Result<StreamId, FfiHostTransportError> {
    StreamId::new(value)
        .map_err(|error| FfiHostTransportError::InvalidConfiguration(error.to_string()))
}

fn delivery_from(delivery: TransportDelivery) -> FfiHostTransportDelivery {
    FfiHostTransportDelivery {
        intended_peers: delivery.report.intended_peers,
        successful_peers: delivery.report.successful_peers,
        failed_peers: delivery.report.failed_peers,
        bytes_sent: delivery.bytes_sent,
    }
}

fn map_event(event: TransportEvent) -> Option<FfiHostTransportEvent> {
    match event {
        TransportEvent::PeerAccepted { peer, .. } => Some(FfiHostTransportEvent::PeerAccepted {
            control_address: peer.control_address.to_string(),
        }),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame,
            ..
        } => map_control_frame(frame),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization,
            frame: ProtocolFrame::SyncRequest(request),
            peer,
            ..
        } => Some(FfiHostTransportEvent::SyncRequestReceived {
            listener_id: peer.device_id.map(DeviceId::into_string),
            correlation_id: request.correlation_id,
            t1_listener_send_elapsed_ms: request.t1_listener_send_elapsed_ms.get(),
        }),
        TransportEvent::PeerDisconnected { peer, error, .. } => {
            Some(FfiHostTransportEvent::PeerDisconnected {
                listener_id: peer.device_id.map(DeviceId::into_string),
                message: error.map(|error| error.to_string()),
            })
        }
        TransportEvent::Rejected { error, .. } => Some(FfiHostTransportEvent::Rejected {
            message: error.to_string(),
        }),
        TransportEvent::FrameReceived { .. } | TransportEvent::PeerAuthorized { .. } => None,
    }
}

fn map_control_frame(
    frame: silent_disco_core::protocol::ProtocolFrame,
) -> Option<FfiHostTransportEvent> {
    use silent_disco_core::protocol::ProtocolFrame;
    let ProtocolFrame::Control(message) = frame else {
        return None;
    };
    match message {
        ControlMessage::JoinRequest(value) => Some(FfiHostTransportEvent::JoinRequestReceived {
            device_id: value.device.device_id.into_string(),
            display_name: value.device.display_name,
            invite_code: value.invite_code,
            sync_port: value.sync_port,
            audio_port: value.audio_port,
        }),
        ControlMessage::Heartbeat(value) => Some(FfiHostTransportEvent::Heartbeat {
            listener_id: value.listener_id.into_string(),
        }),
        ControlMessage::ResyncNotice(value) => Some(FfiHostTransportEvent::ResyncNotice {
            listener_id: value.listener_id.into_string(),
            reason: value.reason,
        }),
        // `SynchronizationReport` is deliberately not surfaced here: this FFI
        // handle is the Android-as-host path, and D2 (2026-08-10) only wired
        // the desktop host's own native processor
        // (`host_transport_events.rs`) to turn this into
        // `AudioEvent::SynchronizationUpdated`. Wiring an Android host's
        // per-listener sync diagnostics the same way is real follow-up work,
        // not done here -- see `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`.
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::Disconnect(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_)
        | ControlMessage::SynchronizationReport(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::FfiHostTransportHandle;
    use std::net::{TcpListener, UdpSocket};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    fn free_tcp_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .expect("bind ephemeral tcp port")
            .local_addr()
            .expect("local addr")
            .port()
    }

    fn free_udp_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .expect("bind ephemeral udp port")
            .local_addr()
            .expect("local addr")
            .port()
    }

    /// Block 24: races `shutdown` against a thread continuously issuing real
    /// transport operations -- broadcasts over real loopback sockets and
    /// event polling -- against a bound host with no connected peer. This is
    /// exactly the moment the accept-loop, outbound, and event-queue worker
    /// threads are all live and busy, which is what "shutdown during network
    /// load" means for a host transport that owns its own worker threads
    /// rather than borrowing a shared runtime. No operation may panic,
    /// `shutdown` itself must complete in bounded time regardless of the
    /// concurrent load, and a second shutdown afterward must be an explicit
    /// no-op rather than reaching into torn-down worker state.
    #[test]
    fn shutdown_races_with_concurrent_broadcasts_and_never_panics_or_hangs() {
        for iteration in 0_u32..5 {
            let handle = FfiHostTransportHandle::bind(
                "127.0.0.1".to_owned(),
                free_tcp_port(),
                free_udp_port(),
                free_udp_port(),
                format!("session-shutdown-race-{iteration}"),
            )
            .expect("host transport binds on loopback");

            let running = Arc::new(AtomicBool::new(true));
            let load_flag = Arc::clone(&running);
            let load_handle = Arc::clone(&handle);
            let load_thread = thread::spawn(move || {
                let mut sequence = 0_u64;
                while load_flag.load(Ordering::SeqCst) {
                    let _ = load_handle.broadcast_audio(
                        "stream-shutdown-race".to_owned(),
                        sequence,
                        48_000,
                        2,
                        960,
                        sequence * 960,
                        sequence * 20,
                        vec![0_u8; 64],
                    );
                    let _ = load_handle.poll_event(0);
                    sequence += 1;
                }
            });

            // Let real broadcasts and polls actually run before racing the
            // shutdown.
            thread::sleep(Duration::from_millis(5));
            let started = Instant::now();
            handle
                .shutdown()
                .expect("shutdown succeeds under concurrent network load");
            let elapsed = started.elapsed();
            assert!(
                elapsed < Duration::from_secs(5),
                "shutdown took {elapsed:?} while racing concurrent broadcasts/polls"
            );

            running.store(false, Ordering::SeqCst);
            load_thread.join().expect("load thread must not panic");

            handle
                .shutdown()
                .expect("a repeated shutdown is a no-op, not an error");
        }
    }
}
