use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use silent_disco_core::domain::{DeviceId, MonotonicMillis, SyncConfidence};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, Disconnect, JoinRequest, ProtocolFrame, SyncRequest,
    SynchronizationReport,
};
use silent_disco_core::transport::{
    DEFAULT_IO_TIMEOUT, DEFAULT_OPERATION_TIMEOUT, DEFAULT_TRANSPORT_EVENT_CAPACITY,
    DEFAULT_TRANSPORT_QUEUE_CAPACITY, ListenerTransportConfig, ListenerTransportNode,
    ManualHostEndpoint, SystemTransportClock, TransportChannel, TransportClock, TransportEvent,
    TransportFactory, production_transport_factory,
};

use crate::listener_playback::{FfiListenerPlaybackHandle, FfiSyncConfidence};

use super::types::{
    FfiListenerDatagramRoutes, FfiListenerTransportCounters, FfiListenerTransportError,
    FfiListenerTransportEvent, FfiManualHostEndpoint,
};

/// Parses and validates a copy-pasted desktop connection payload without connecting.
///
/// Intended for live UI validation as the user types or pastes; call
/// [`FfiListenerTransportHandle::connect`] with the same raw payload to
/// actually open the transport.
///
/// # Errors
///
/// Returns [`FfiListenerTransportError::InvalidEndpoint`] or
/// [`FfiListenerTransportError::UnsupportedProtocolVersion`] for a malformed,
/// oversized, expired, or version-mismatched payload.
#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned String parameters at the foreign boundary"
)]
#[uniffi::export]
pub fn parse_manual_host_endpoint(
    raw: String,
    now_wall_clock_ms: u64,
) -> Result<FfiManualHostEndpoint, FfiListenerTransportError> {
    let endpoint = ManualHostEndpoint::parse(&raw, now_wall_clock_ms)?;
    Ok(FfiManualHostEndpoint {
        host_address: endpoint.endpoint.address.to_string(),
        control_port: endpoint.endpoint.control_port,
        sync_port: endpoint.endpoint.sync_port,
        audio_port: endpoint.endpoint.audio_port,
        session_id: endpoint.session_id.as_str().to_owned(),
        protocol_version: endpoint.protocol_version,
        invite_code_required: endpoint.invite_code_required,
        expires_at_ms: endpoint.expires_at_ms,
    })
}

/// Opaque, bounded handle around the shared Rust `SocketListenerTransport`.
///
/// This handle is intentionally narrow and transport-oriented: it does not
/// route through the authoritative `CoreActorRuntime`. The shared actor's
/// listener-role commands (`SelectSession`, `SubmitJoin`, ...) model
/// discovery-driven joins and do not yet have a corresponding event path for
/// receiving `JoinApproval`/`JoinRejection` from a connected transport. Kotlin
/// observes the typed events this handle exposes and projects them into local
/// UI state; it performs no protocol parsing or domain-legality decisions of
/// its own.
#[derive(uniffi::Object)]
pub struct FfiListenerTransportHandle {
    /// A read/write lock, not a mutex, and that distinction is the point:
    /// `poll_event` blocks inside its receive while holding a **read**
    /// guard, and the send methods take read guards too, so a probe can be
    /// sent while a poll is parked. Only teardown needs the write guard.
    inner: RwLock<Option<Inner>>,
    /// When set, received audio datagrams are submitted straight to this
    /// runtime inside [`Self::poll_event`] instead of being surfaced to the
    /// foreign caller. See [`Self::attach_playback`].
    playback: Mutex<Option<Arc<FfiListenerPlaybackHandle>>>,
    /// Audio datagrams forwarded directly to playback, preserving the
    /// received-packet count the foreign caller used to derive by counting
    /// the per-packet events it no longer sees.
    forwarded_audio: AtomicU64,
}

impl std::fmt::Debug for FfiListenerTransportHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FfiListenerTransportHandle")
    }
}

struct Inner {
    transport: Box<dyn ListenerTransportNode>,
    session_id: silent_disco_core::domain::SessionId,
    device_id: DeviceId,
    /// The exact clock instance passed to `connect_listener`, retained so
    /// `FfiListenerTransportHandle::now_ms` can be queried on the same
    /// timeline `TransportEvent::FrameReceived::received_at` is stamped on.
    /// See `now_ms`'s doc comment for why that matters.
    clock: Arc<dyn TransportClock>,
}

#[allow(
    clippy::missing_errors_doc,
    reason = "all exported methods use the uniform validated FfiListenerTransportError contract"
)]
#[uniffi::export]
impl FfiListenerTransportHandle {
    /// Parses the raw connection payload and opens a real listener transport.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "uniffi::export requires owned String parameters at the foreign boundary"
    )]
    #[uniffi::constructor]
    pub fn connect(
        raw_endpoint: String,
        now_wall_clock_ms: u64,
        local_device_id: String,
        local_address: String,
    ) -> Result<Arc<Self>, FfiListenerTransportError> {
        let endpoint = ManualHostEndpoint::parse(&raw_endpoint, now_wall_clock_ms)?;
        let device_id = DeviceId::new(local_device_id)
            .map_err(|error| FfiListenerTransportError::InvalidEndpoint(error.to_string()))?;
        let local_address: IpAddr = local_address.parse().map_err(|_| {
            FfiListenerTransportError::InvalidEndpoint("local bind address is malformed".to_owned())
        })?;
        let session_id = endpoint.session_id.clone();
        let config = ListenerTransportConfig {
            session_id: session_id.clone(),
            device_id: device_id.clone(),
            endpoint: endpoint.endpoint,
            local_address,
            outbound_queue_capacity: DEFAULT_TRANSPORT_QUEUE_CAPACITY,
            event_queue_capacity: DEFAULT_TRANSPORT_EVENT_CAPACITY,
            io_timeout: DEFAULT_IO_TIMEOUT,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
        };
        let clock: Arc<dyn TransportClock> = Arc::new(SystemTransportClock::default());
        let transport =
            production_transport_factory().connect_listener(config, Arc::clone(&clock))?;
        Ok(Arc::new(Self {
            playback: Mutex::new(None),
            forwarded_audio: AtomicU64::new(0),
            inner: RwLock::new(Some(Inner {
                transport,
                session_id,
                device_id,
                clock,
            })),
        }))
    }

    /// Sends the shared Rust `JoinRequest` for this session.
    ///
    /// Includes this listener's own bound synchronization/audio UDP ports so
    /// the host can authorize datagram routing for this peer if it approves
    /// the join -- Kotlin never needs to know these ports exist.
    pub fn send_join_request(
        &self,
        display_name: String,
        invite_code: Option<String>,
    ) -> Result<(), FfiListenerTransportError> {
        self.with_transport(|inner| {
            let routes = inner.transport.local_routes();
            let message = ControlMessage::JoinRequest(JoinRequest {
                session_id: inner.session_id.clone(),
                device: DeviceIdentity {
                    device_id: inner.device_id.clone(),
                    display_name,
                },
                invite_code,
                sync_port: routes.synchronization.port(),
                audio_port: routes.audio.port(),
            });
            inner.transport.send_control(&message)?;
            Ok(())
        })
    }

    /// Sends an explicit listener-initiated disconnect before shutdown.
    pub fn send_disconnect(&self, reason: String) -> Result<(), FfiListenerTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::Disconnect(Disconnect {
                session_id: inner.session_id.clone(),
                listener_id: inner.device_id.clone(),
                reason,
            });
            inner.transport.send_control(&message)?;
            Ok(())
        })
    }

    /// Sends a clock-sync probe carrying this listener's own send timestamp.
    ///
    /// `correlation_id` is caller-chosen and echoed back verbatim on the
    /// matching `SyncResponseReceived` event so the caller can match a
    /// response to the probe that produced it.
    pub fn send_sync_request(
        &self,
        correlation_id: u64,
        local_send_elapsed_ms: u64,
    ) -> Result<(), FfiListenerTransportError> {
        self.with_transport(|inner| {
            let request = SyncRequest {
                session_id: inner.session_id.clone(),
                correlation_id,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(local_send_elapsed_ms),
            };
            inner.transport.send_sync_request(&request)?;
            Ok(())
        })
    }

    /// Reports this listener's own locally-computed synchronization quality
    /// (from its `ClockSyncEstimator`) back to the host, so the host's
    /// per-listener sync diagnostics stop reading "has not yet completed a
    /// sync exchange" forever (D2, `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`).
    /// The host has no way to compute this itself -- see
    /// `SynchronizationReport`'s doc comment -- so this is Kotlin
    /// voluntarily relaying what it already knows, not answering a probe.
    ///
    /// # Errors
    ///
    /// Returns a structured error when the underlying send fails.
    pub fn send_synchronization_report(
        &self,
        confidence: FfiSyncConfidence,
        offset_ms: f64,
        round_trip_ms: f64,
        drift_ppm: f64,
    ) -> Result<(), FfiListenerTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::SynchronizationReport(SynchronizationReport {
                session_id: inner.session_id.clone(),
                listener_id: inner.device_id.clone(),
                confidence: SyncConfidence::from(confidence),
                offset_ms,
                round_trip_ms,
                drift_ppm,
            });
            inner.transport.send_control(&message)?;
            Ok(())
        })
    }

    /// Attaches a playback runtime so received audio bypasses the foreign
    /// binding entirely.
    ///
    /// While attached, [`Self::poll_event`] submits each arriving audio
    /// datagram straight into `playback` and keeps polling, so audio frames
    /// never occupy the foreign caller's event stream. That stream otherwise
    /// carries one event per audio packet -- 200/second at the current
    /// packet duration -- and a foreign caller draining it one event at a
    /// time, copying each payload out and immediately back in, becomes the
    /// bottleneck. Measured on a real device: clock-sync responses queued
    /// behind that audio backlog and were timestamped ~140ms late against a
    /// true network round trip of 7.7ms, which had 94% of sync samples
    /// rejected and biased the offset estimate -- the reason packets read as
    /// late and forced the rebuffering heard as dropouts.
    pub fn attach_playback(&self, playback: Arc<FfiListenerPlaybackHandle>) {
        if let Ok(mut guard) = self.playback.lock() {
            *guard = Some(playback);
        }
    }

    /// Stops forwarding to a previously attached runtime. Idempotent, and
    /// must be called before that runtime is stopped so no datagram is
    /// submitted to a runtime that is shutting down.
    pub fn detach_playback(&self) {
        if let Ok(mut guard) = self.playback.lock() {
            *guard = None;
        }
    }

    /// Audio datagrams forwarded straight to playback since connect.
    #[must_use]
    pub fn forwarded_audio_count(&self) -> u64 {
        self.forwarded_audio.load(Ordering::Relaxed)
    }

    /// Waits up to `timeout_ms` for the next transport event the foreign
    /// caller needs to see.
    ///
    /// Returns `None` when the bounded wait elapses without such an event.
    /// Audio datagrams are consumed internally while a playback runtime is
    /// attached (see [`Self::attach_playback`]) and are never returned here,
    /// so this can legitimately return `None` while a great deal of audio is
    /// flowing.
    pub fn poll_event(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<FfiListenerTransportEvent>, FfiListenerTransportError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let event = {
                let guard = self.read()?;
                let inner = guard.as_ref().ok_or_else(|| {
                    FfiListenerTransportError::Closed("transport is closed".to_owned())
                })?;
                match inner.transport.recv_event(remaining) {
                    Ok(event) => event,
                    Err(error)
                        if error.kind
                            == silent_disco_core::transport::TransportErrorKind::Timeout =>
                    {
                        return Ok(None);
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            // Forwarded audio is consumed here rather than returned, so the
            // caller's loop keeps pace with control traffic no matter how
            // much audio is arriving.
            if let Some(event) = self.forward_audio(event)?
                && let Some(mapped) = map_event(event)
            {
                return Ok(Some(mapped));
            }
        }
    }

    /// This transport's own clock, read now.
    ///
    /// On a *different* timeline than the playback runtime's `now_ms` --
    /// each is `Instant::now()` at its own construction, and the transport
    /// connects before a stream's runtime exists. To translate a
    /// `SyncResponseReceived.received_at_elapsed_ms` onto the runtime's
    /// timeline, read both clocks back-to-back at runtime creation and
    /// subtract: since both wrap the same underlying monotonic source, that
    /// one-time delta stays exact for the rest of the process (Instant
    /// deltas are not subject to drift the way wall-clock deltas are).
    pub fn now_ms(&self) -> Result<u64, FfiListenerTransportError> {
        self.with_transport(|inner| Ok(inner.clock.now().get()))
    }

    pub fn local_routes(&self) -> Result<FfiListenerDatagramRoutes, FfiListenerTransportError> {
        self.with_transport(|inner| {
            let routes = inner.transport.local_routes();
            Ok(FfiListenerDatagramRoutes {
                local_synchronization_port: routes.synchronization.port(),
                local_audio_port: routes.audio.port(),
            })
        })
    }

    pub fn counters(&self) -> Result<FfiListenerTransportCounters, FfiListenerTransportError> {
        self.with_transport(|inner| {
            let counters = inner.transport.counters();
            Ok(FfiListenerTransportCounters {
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
    pub fn shutdown(&self) -> Result<(), FfiListenerTransportError> {
        let mut guard = self.write()?;
        let Some(mut inner) = guard.take() else {
            return Ok(());
        };
        inner.transport.shutdown().map_err(Into::into)
    }
}

impl FfiListenerTransportHandle {
    fn read(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, Option<Inner>>, FfiListenerTransportError> {
        self.inner.read().map_err(|_| {
            FfiListenerTransportError::Closed("transport lock was poisoned".to_owned())
        })
    }

    fn write(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, Option<Inner>>, FfiListenerTransportError> {
        self.inner.write().map_err(|_| {
            FfiListenerTransportError::Closed("transport lock was poisoned".to_owned())
        })
    }

    fn with_transport<T>(
        &self,
        action: impl FnOnce(&Inner) -> Result<T, FfiListenerTransportError>,
    ) -> Result<T, FfiListenerTransportError> {
        let guard = self.read()?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| FfiListenerTransportError::Closed("transport is closed".to_owned()))?;
        action(inner)
    }
}

impl Drop for FfiListenerTransportHandle {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.write()
            && let Some(mut inner) = guard.take()
        {
            drop(inner.transport.shutdown());
        }
    }
}

impl FfiListenerTransportHandle {
    /// Submits `event` straight to an attached playback runtime when it is
    /// an audio datagram, returning `None` to mean "consumed". Any other
    /// event, or audio with nothing attached, is handed back unchanged for
    /// normal mapping.
    fn forward_audio(
        &self,
        event: TransportEvent,
    ) -> Result<Option<TransportEvent>, FfiListenerTransportError> {
        let is_audio = matches!(
            &event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(_),
                ..
            }
        );
        if !is_audio {
            return Ok(Some(event));
        }
        let attached = self.playback.lock().ok().and_then(|guard| guard.clone());
        let Some(playback) = attached else {
            return Ok(Some(event));
        };
        let TransportEvent::FrameReceived {
            frame: ProtocolFrame::Audio(datagram),
            ..
        } = event
        else {
            unreachable!("checked to be an audio frame above")
        };
        playback.submit_core_datagram(datagram).map_err(|error| {
            FfiListenerTransportError::Io(format!(
                "attached listener playback failed while forwarding audio: {error}"
            ))
        })?;
        self.forwarded_audio.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }
}

fn map_event(event: TransportEvent) -> Option<FfiListenerTransportEvent> {
    use silent_disco_core::protocol::ProtocolFrame;
    match event {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame,
            ..
        } => map_control_frame(frame),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization,
            frame: ProtocolFrame::SyncResponse(response),
            received_at,
            ..
        } => Some(FfiListenerTransportEvent::SyncResponseReceived {
            correlation_id: response.correlation_id,
            t1_listener_send_elapsed_ms: response.t1_listener_send_elapsed_ms.get(),
            t2_host_receive_elapsed_ms: response.t2_host_receive_elapsed_ms.get(),
            t3_host_send_elapsed_ms: response.t3_host_send_elapsed_ms.get(),
            received_at_elapsed_ms: received_at.get(),
        }),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            frame: ProtocolFrame::Audio(datagram),
            ..
        } => Some(FfiListenerTransportEvent::AudioReceived {
            stream_id: datagram.stream_id.into_string(),
            sequence: datagram.sequence.get(),
            sample_rate: datagram.sample_rate,
            channels: datagram.channels,
            samples_per_packet: datagram.samples_per_packet,
            first_sample_index: datagram.first_sample_index.get(),
            host_presentation_time_ms: datagram.host_presentation_time_ms.get(),
            payload: datagram.payload,
        }),
        TransportEvent::PeerDisconnected { error, .. } => {
            Some(FfiListenerTransportEvent::ConnectionClosed {
                message: error.map(|error| error.to_string()),
            })
        }
        TransportEvent::Rejected { error, .. } => Some(FfiListenerTransportEvent::Rejected {
            message: error.to_string(),
        }),
        TransportEvent::FrameReceived { .. }
        | TransportEvent::PeerAccepted { .. }
        | TransportEvent::PeerAuthorized { .. } => None,
    }
}

fn map_control_frame(
    frame: silent_disco_core::protocol::ProtocolFrame,
) -> Option<FfiListenerTransportEvent> {
    use silent_disco_core::protocol::ProtocolFrame;
    let ProtocolFrame::Control(message) = frame else {
        return None;
    };
    match message {
        ControlMessage::Hello(value) => Some(FfiListenerTransportEvent::Hello {
            session_name: value.session_name,
            host_name: value.host_name,
            approval_required: value.approval_required,
        }),
        ControlMessage::JoinApproval(value) => Some(FfiListenerTransportEvent::JoinApproved {
            trusted_for_future: value.trusted_for_future,
        }),
        ControlMessage::JoinRejection(value) => Some(FfiListenerTransportEvent::JoinRejected {
            reason: value.reason,
        }),
        ControlMessage::Disconnect(value) => Some(FfiListenerTransportEvent::HostDisconnected {
            reason: value.reason,
        }),
        ControlMessage::StreamStart(value) => Some(FfiListenerTransportEvent::StreamStarted {
            stream_id: value.stream_id.into_string(),
            host_start_time_ms: value.host_start_time_ms.get(),
            sample_rate: value.sample_rate,
            channels: value.channels,
            samples_per_packet: value.samples_per_packet,
        }),
        ControlMessage::Pause(value) => Some(FfiListenerTransportEvent::Paused {
            stream_id: value.stream_id.into_string(),
            host_pause_time_ms: value.host_pause_time_ms.get(),
        }),
        ControlMessage::Stop(value) => Some(FfiListenerTransportEvent::Stopped {
            stream_id: value.stream_id.into_string(),
            host_stop_time_ms: value.host_stop_time_ms.get(),
        }),
        ControlMessage::JoinRequest(_)
        | ControlMessage::Heartbeat(_)
        | ControlMessage::ResyncNotice(_)
        | ControlMessage::SynchronizationReport(_) => None,
    }
}

#[cfg(test)]
mod audio_forwarding_tests {
    use super::{FfiListenerTransportHandle, ProtocolFrame};
    use crate::audio_abi::registry_test_guard;
    use crate::listener_playback::{FfiListenerPlaybackConfig, FfiListenerPlaybackHandle};
    use silent_disco_core::domain::{
        MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
    };
    use silent_disco_core::protocol::{AudioCodec, AudioDatagram, ControlMessage, Disconnect};
    use silent_disco_core::transport::{TransportChannel, TransportEvent, TransportPeer};
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex, RwLock};

    const SAMPLES_PER_PACKET: u32 = 240;

    /// A handle with no transport: `forward_audio` only consults the
    /// attached runtime, so this exercises the routing decision on its own,
    /// without sockets or timing.
    fn detached_handle() -> FfiListenerTransportHandle {
        FfiListenerTransportHandle {
            inner: RwLock::new(None),
            playback: Mutex::new(None),
            forwarded_audio: AtomicU64::new(0),
        }
    }

    fn playback() -> Arc<FfiListenerPlaybackHandle> {
        FfiListenerPlaybackHandle::open(FfiListenerPlaybackConfig {
            session_id: "session-forwarding".to_owned(),
            stream_id: "stream-forwarding".to_owned(),
            sample_rate: 48_000,
            host_start_time_ms: 0,
            samples_per_packet: SAMPLES_PER_PACKET,
            channels: 2,
            startup_buffer_target_ms: 400,
            rebuffer_target_ms: 400,
            ring_capacity_frames: 48_000,
            ring_target_fill_frames: 19_200,
            write_lead_ms: 400,
            max_prefill_ms: 800,
            volume: 1.0,
        })
        .expect("playback runtime opens")
    }

    fn audio_event(sequence: u64) -> TransportEvent {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            peer: TransportPeer {
                device_id: None,
                control_address: "127.0.0.1:1".parse().expect("addr"),
            },
            frame: ProtocolFrame::Audio(AudioDatagram {
                session_id: SessionId::new("session-forwarding").expect("session"),
                stream_id: StreamId::new("stream-forwarding").expect("stream"),
                sequence: PacketSequence::new(sequence),
                codec: AudioCodec::PcmS16Le,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: SAMPLES_PER_PACKET,
                first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
                host_presentation_time_ms: MonotonicMillis::new(sequence * 5),
                payload: vec![0_u8; SAMPLES_PER_PACKET as usize * 2 * 2],
            }),
            received_at: MonotonicMillis::new(sequence * 5),
        }
    }

    fn control_event() -> TransportEvent {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            peer: TransportPeer {
                device_id: None,
                control_address: "127.0.0.1:1".parse().expect("addr"),
            },
            frame: ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
                session_id: SessionId::new("session-forwarding").expect("session"),
                listener_id: silent_disco_core::domain::DeviceId::new("d").expect("device"),
                reason: "bye".to_owned(),
            })),
            received_at: MonotonicMillis::new(0),
        }
    }

    /// The whole point of attaching: audio stops occupying the foreign
    /// caller's event stream. If these frames were still returned, the
    /// caller would keep paying one event plus two payload copies per
    /// packet, which is what delayed control traffic behind audio.
    #[test]
    fn attached_audio_is_consumed_and_never_surfaced() {
        let _guard = registry_test_guard();
        let handle = detached_handle();
        handle.attach_playback(playback());

        for sequence in 0..5 {
            assert!(
                handle
                    .forward_audio(audio_event(sequence))
                    .expect("forward succeeds")
                    .is_none(),
                "audio must be consumed while a runtime is attached"
            );
        }
        assert_eq!(handle.forwarded_audio_count(), 5);
    }

    #[test]
    fn attached_playback_failure_is_returned_instead_of_counted_as_forwarded_audio() {
        let _guard = registry_test_guard();
        let handle = detached_handle();
        let playback = playback();
        playback.stop().expect("playback stops");
        handle.attach_playback(Arc::clone(&playback));

        let error = handle
            .forward_audio(audio_event(0))
            .expect_err("a dead attached runtime must make transport forwarding fail");
        assert!(matches!(error, FfiListenerTransportError::Io(_)));
        assert_eq!(handle.forwarded_audio_count(), 0);
    }

    /// Control traffic must be unaffected -- it is precisely what was being
    /// delayed, so it has to keep reaching the caller promptly.
    #[test]
    fn control_events_are_never_consumed() {
        let _guard = registry_test_guard();
        let handle = detached_handle();
        handle.attach_playback(playback());

        assert!(
            handle
                .forward_audio(control_event())
                .expect("forward succeeds")
                .is_some(),
            "control frames must still reach the caller"
        );
        assert_eq!(handle.forwarded_audio_count(), 0);
    }

    /// With nothing attached the previous behaviour must be preserved, so a
    /// caller that never attaches still receives audio events.
    #[test]
    fn unattached_audio_still_surfaces_to_the_caller() {
        let _guard = registry_test_guard();
        let handle = detached_handle();
        assert!(
            handle
                .forward_audio(audio_event(0))
                .expect("forward succeeds")
                .is_some(),
            "audio must surface when no runtime is attached"
        );
        assert_eq!(handle.forwarded_audio_count(), 0);
    }

    /// Detaching must stop forwarding immediately: submitting into a
    /// runtime that is being stopped is exactly what the detach ordering in
    /// the Kotlin controller exists to prevent.
    #[test]
    fn detaching_returns_audio_to_the_caller() {
        let _guard = registry_test_guard();
        let handle = detached_handle();
        handle.attach_playback(playback());
        assert!(
            handle
                .forward_audio(audio_event(0))
                .expect("forward succeeds")
                .is_none()
        );

        handle.detach_playback();
        assert!(
            handle
                .forward_audio(audio_event(1))
                .expect("forward succeeds")
                .is_some(),
            "audio must surface again once detached"
        );
        handle.detach_playback();
        assert_eq!(handle.forwarded_audio_count(), 1);
    }
}
