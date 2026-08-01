use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use silent_disco_core::domain::DeviceId;
use silent_disco_core::protocol::{ControlMessage, DeviceIdentity, Disconnect, JoinRequest};
use silent_disco_core::transport::{
    DEFAULT_IO_TIMEOUT, DEFAULT_OPERATION_TIMEOUT, DEFAULT_TRANSPORT_EVENT_CAPACITY,
    DEFAULT_TRANSPORT_QUEUE_CAPACITY, ListenerTransportConfig, ListenerTransportNode,
    ManualHostEndpoint, SystemTransportClock, TransportChannel, TransportClock, TransportEvent,
    TransportFactory, production_transport_factory,
};

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
    inner: Mutex<Option<Inner>>,
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
        let transport = production_transport_factory().connect_listener(config, clock)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(Inner {
                transport,
                session_id,
                device_id,
            })),
        }))
    }

    /// Sends the shared Rust `JoinRequest` for this session.
    pub fn send_join_request(
        &self,
        display_name: String,
        invite_code: Option<String>,
    ) -> Result<(), FfiListenerTransportError> {
        self.with_transport(|inner| {
            let message = ControlMessage::JoinRequest(JoinRequest {
                session_id: inner.session_id.clone(),
                device: DeviceIdentity {
                    device_id: inner.device_id.clone(),
                    display_name,
                },
                invite_code,
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

    /// Waits up to `timeout_ms` for the next transport event.
    ///
    /// Returns `None` when the bounded wait elapses without an event.
    pub fn poll_event(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<FfiListenerTransportEvent>, FfiListenerTransportError> {
        let mut guard = self.lock()?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| FfiListenerTransportError::Closed("transport is closed".to_owned()))?;
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
        let mut guard = self.lock()?;
        let Some(mut inner) = guard.take() else {
            return Ok(());
        };
        inner.transport.shutdown().map_err(Into::into)
    }
}

impl FfiListenerTransportHandle {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Inner>>, FfiListenerTransportError> {
        self.inner.lock().map_err(|_| {
            FfiListenerTransportError::Closed("transport lock was poisoned".to_owned())
        })
    }

    fn with_transport<T>(
        &self,
        action: impl FnOnce(&mut Inner) -> Result<T, FfiListenerTransportError>,
    ) -> Result<T, FfiListenerTransportError> {
        let mut guard = self.lock()?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| FfiListenerTransportError::Closed("transport is closed".to_owned()))?;
        action(inner)
    }
}

impl Drop for FfiListenerTransportHandle {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(mut inner) = guard.take()
        {
            drop(inner.transport.shutdown());
        }
    }
}

fn map_event(event: TransportEvent) -> Option<FfiListenerTransportEvent> {
    match event {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame,
            ..
        } => map_control_frame(frame),
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
        ControlMessage::StreamStart(_) => Some(FfiListenerTransportEvent::StreamStarted),
        ControlMessage::Pause(_) => Some(FfiListenerTransportEvent::Paused),
        ControlMessage::Stop(_) => Some(FfiListenerTransportEvent::Stopped),
        ControlMessage::JoinRequest(_)
        | ControlMessage::Heartbeat(_)
        | ControlMessage::ResyncNotice(_) => None,
    }
}
