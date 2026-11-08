use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use silent_disco_core::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, Disconnect, JoinApproval, JoinRejection, Pause,
    ProtocolFrame, Stop, StreamStart, SyncResponse, TrackBoundary,
};
use silent_disco_core::transport::secure_control_external_key::host_tls_identity_from_external_key;
use silent_disco_core::transport::secure_control_tls::HostTlsIdentity;
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
use crate::listener_proof::{CallbackSigner, FfiListenerProofSigner};

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
        Self::bind_with_host_clock(
            local_address,
            control_port,
            sync_port,
            audio_port,
            session_id,
            0,
        )
    }

    /// Binds using Android's boot-relative elapsedRealtime clock domain.
    /// On Android, a nonzero anchor selects the kernel `CLOCK_BOOTTIME` source
    /// directly: unlike Instant, it includes device suspend and avoids bind
    /// handoff skew. Other targets retain anchored relative time for tests.
    /// The legacy zero-anchor constructor keeps its original relative clock.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned String parameters at the foreign boundary"
    )]
    #[uniffi::constructor]
    pub fn bind_with_host_clock(
        local_address: String,
        control_port: u16,
        sync_port: u16,
        audio_port: u16,
        session_id: String,
        host_now_elapsed_ms: u64,
    ) -> Result<Arc<Self>, FfiHostTransportError> {
        Self::bind_configured(
            local_address,
            control_port,
            sync_port,
            audio_port,
            session_id,
            host_now_elapsed_ms,
            None,
        )
    }

    /// Binds like [`Self::bind_with_host_clock`], presenting a TLS control-channel
    /// identity whose private key stays in the platform (Android Keystore). That
    /// key's SPKI is the `tk` the host's invitations carry, so listeners pin what
    /// their handshake proves. Fails closed: if the platform cannot supply its
    /// public key or certificate, nothing binds.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned String and Arc parameters at the foreign boundary"
    )]
    #[uniffi::constructor]
    pub fn bind_secure_with_host_clock(
        local_address: String,
        control_port: u16,
        sync_port: u16,
        audio_port: u16,
        session_id: String,
        host_now_elapsed_ms: u64,
        tls_signer: Arc<dyn FfiListenerProofSigner>,
    ) -> Result<Arc<Self>, FfiHostTransportError> {
        let identity = host_tls_identity_from_external_key(Arc::new(CallbackSigner(tls_signer)))
            .map_err(|error| {
                FfiHostTransportError::InvalidConfiguration(format!(
                    "host TLS identity unavailable: {error}"
                ))
            })?;
        Self::bind_configured(
            local_address,
            control_port,
            sync_port,
            audio_port,
            session_id,
            host_now_elapsed_ms,
            Some(Arc::new(identity)),
        )
    }

    /// Sends a session-only join approval to one identified listener.
    /// Legacy v3 has no verified key identity, so it cannot grant durable trust.
    pub fn send_join_approval(
        &self,
        listener_id: String,
        trusted_for_future: bool,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        if trusted_for_future {
            return Err(FfiHostTransportError::Unauthorized(
                "Legacy v3 transport cannot remember a listener without verified cryptographic identity"
                    .to_owned(),
            ));
        }
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

    /// Refuses only the pending TCP connection that submitted this join.
    /// A self-declared device ID can be duplicated, so a refusal for a
    /// congested/unprojected join must never be routed by ID alone.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned String parameters at the foreign boundary"
    )]
    pub fn reject_pending_peer_at(
        &self,
        control_address: String,
        listener_id: String,
        reason: String,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        let address: std::net::SocketAddr = control_address.parse().map_err(|_| {
            FfiHostTransportError::InvalidConfiguration(
                "pending listener control address is malformed".to_owned(),
            )
        })?;
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let message = ControlMessage::JoinRejection(JoinRejection {
                session_id: inner.session_id.clone(),
                listener_id: listener_id.clone(),
                reason,
            });
            let delivery =
                inner
                    .transport
                    .reject_pending_peer_at(address, &listener_id, &message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Sends an explicit disconnect to an admitted listener and atomically
    /// retires its audio route. Pending (not-yet-authorized) peers still
    /// receive the rejection through their control-only TCP connection.
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
            let delivery = match inner.transport.send_disconnect(&listener_id, &message) {
                Ok(delivery) => delivery,
                Err(error)
                    if error.kind
                        == silent_disco_core::transport::TransportErrorKind::PeerNotFound =>
                {
                    inner
                        .transport
                        .send_pending_control(&listener_id, &message)?
                }
                Err(error) => return Err(error.into()),
            };
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
        current_entry_id: Option<String>,
        current_display_name: Option<String>,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::StreamStart(StreamStart {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                host_start_time_ms: MonotonicMillis::new(host_start_time_ms),
                sample_rate,
                channels,
                samples_per_packet,
                current_entry_id,
                current_display_name,
            });
            let delivery = inner.transport.broadcast_control(&message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Sends the current stream announcement to one already-authorized listener.
    /// Used when an approved listener joins after playback has started.
    #[allow(clippy::too_many_arguments)]
    pub fn send_stream_start_to_listener(
        &self,
        listener_id: String,
        stream_id: String,
        host_start_time_ms: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        current_entry_id: Option<String>,
        current_display_name: Option<String>,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let message = ControlMessage::StreamStart(StreamStart {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                host_start_time_ms: MonotonicMillis::new(host_start_time_ms),
                sample_rate,
                channels,
                samples_per_packet,
                current_entry_id,
                current_display_name,
            });
            let delivery = inner.transport.send_control(&listener_id, &message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Recover one admitted listener from the host's authoritative playback
    /// cache. No caller-provided `StreamStart` or boundary may override the
    /// transport's current generation, and an idle host sends no replay.
    pub fn replay_cached_playback_to_listener(
        &self,
        listener_id: String,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let delivery = inner
                .transport
                .replay_cached_playback_to_peer(&listener_id)?;
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

    /// Announces the exact sample index where the next queued track begins.
    pub fn broadcast_track_boundary(
        &self,
        stream_id: String,
        first_sample_index: u64,
        entry_id: String,
        display_name: String,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::TrackBoundary(TrackBoundary {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                first_sample_index: SampleIndex::new(first_sample_index),
                entry_id,
                display_name,
            });
            let delivery = inner.transport.broadcast_control(&message)?;
            Ok(delivery_from(delivery))
        })
    }

    /// Sends one pending sample-indexed track boundary to a late-joining listener.
    pub fn send_track_boundary_to_listener(
        &self,
        listener_id: String,
        stream_id: String,
        first_sample_index: u64,
        entry_id: String,
        display_name: String,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let message = ControlMessage::TrackBoundary(TrackBoundary {
                session_id: inner.session_id.clone(),
                stream_id: parse_stream_id(stream_id)?,
                first_sample_index: SampleIndex::new(first_sample_index),
                entry_id,
                display_name,
            });
            let delivery = inner.transport.send_control(&listener_id, &message)?;
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
    /// Retained for compatibility with legacy callers. Production host
    /// event handlers must use `send_sync_response_to_listener` so a peer's
    /// correlation and timestamps are never broadcast to other listeners.
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

    /// Respond to one admitted listener's clock probe on only that listener's
    /// authorized UDP synchronization route. A missing/retired identity fails
    /// closed; the other listeners never receive the correlation or timestamps.
    pub fn send_sync_response_to_listener(
        &self,
        listener_id: String,
        correlation_id: u64,
        t1_listener_send_elapsed_ms: u64,
        t2_host_receive_elapsed_ms: u64,
        t3_host_send_elapsed_ms: u64,
    ) -> Result<FfiHostTransportDelivery, FfiHostTransportError> {
        self.with_transport(|inner| {
            let listener_id = device_id(listener_id)?;
            let frame = ProtocolFrame::SyncResponse(SyncResponse {
                session_id: inner.session_id.clone(),
                correlation_id,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(t1_listener_send_elapsed_ms),
                t2_host_receive_elapsed_ms: MonotonicMillis::new(t2_host_receive_elapsed_ms),
                t3_host_send_elapsed_ms: MonotonicMillis::new(t3_host_send_elapsed_ms),
            });
            let delivery = inner.transport.send_sync_to_peer(&listener_id, &frame)?;
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
        self.with_transport(|inner| Ok(inner.transport.counters().into()))
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
            peer,
            ..
        } => map_control_frame(frame, peer.control_address),
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
            Some(FfiHostTransportEvent::PeerDisconnectedAt {
                control_address: peer.control_address.to_string(),
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
    control_address: std::net::SocketAddr,
) -> Option<FfiHostTransportEvent> {
    use silent_disco_core::protocol::ProtocolFrame;
    let ProtocolFrame::Control(message) = frame else {
        return None;
    };
    match message {
        ControlMessage::JoinRequest(value) => Some(FfiHostTransportEvent::JoinRequestReceived {
            control_address: control_address.to_string(),
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
        | ControlMessage::TrackBoundary(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_)
        | ControlMessage::SynchronizationReport(_) => None,
    }
}

impl FfiHostTransportHandle {
    /// Shared bind path. `secure_control` is the TLS identity presented on the
    /// control port; `None` leaves TLS listeners refused.
    #[allow(
        clippy::too_many_arguments,
        clippy::needless_pass_by_value,
        reason = "mirrors the exported bind constructors one-for-one, including owned String parameters"
    )]
    fn bind_configured(
        local_address: String,
        control_port: u16,
        sync_port: u16,
        audio_port: u16,
        session_id: String,
        host_now_elapsed_ms: u64,
        secure_control: Option<Arc<HostTlsIdentity>>,
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
            secure_control,
        };
        let clock: Arc<dyn TransportClock> = Arc::new(if host_now_elapsed_ms == 0 {
            SystemTransportClock::default()
        } else {
            #[cfg(target_os = "android")]
            {
                SystemTransportClock::android_boot_relative()
            }
            #[cfg(not(target_os = "android"))]
            {
                SystemTransportClock::starting_at(host_now_elapsed_ms)
            }
        });
        let transport = production_transport_factory().bind_host(config, clock)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(Inner {
                transport,
                session_id,
            })),
        }))
    }
}

#[cfg(test)]
#[path = "disconnect_mapping_tests.rs"]
mod disconnect_mapping_tests;
