use core::fmt;
use std::error::Error;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::domain::{DeviceId, MonotonicMillis, SessionId};
use crate::protocol::{ProtocolError, ProtocolFrame};
use crate::runtime::{DeliveryReport, NetworkEndpoint};

pub const DEFAULT_TRANSPORT_QUEUE_CAPACITY: usize = 64;
pub const DEFAULT_TRANSPORT_EVENT_CAPACITY: usize = 256;
pub const DEFAULT_MAX_TRANSPORT_PEERS: usize = 64;
pub const MAX_TRANSPORT_QUEUE_CAPACITY: usize = 4_096;
pub const MAX_TRANSPORT_EVENT_CAPACITY: usize = 8_192;
pub const MAX_TRANSPORT_PEERS: usize = 1_024;
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_millis(100);
pub const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportChannel {
    Control,
    Synchronization,
    Audio,
    Runtime,
}

impl TransportChannel {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Synchronization => "synchronization",
            Self::Audio => "audio",
            Self::Runtime => "runtime",
        }
    }
}

impl fmt::Display for TransportChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    InvalidConfiguration,
    Bind,
    Listen,
    Connect,
    Io,
    Protocol,
    Unauthorized,
    QueueFull,
    Timeout,
    PeerNotFound,
    Delivery,
    ShuttingDown,
    WorkerPanicked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub channel: TransportChannel,
    pub message: String,
}

impl TransportError {
    pub(crate) fn new(
        kind: TransportErrorKind,
        channel: TransportChannel,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            channel,
            message: message.into(),
        }
    }

    pub(crate) fn io(
        kind: TransportErrorKind,
        channel: TransportChannel,
        context: &str,
        error: &std::io::Error,
    ) -> Self {
        Self::new(kind, channel, format!("{context}: {error}"))
    }

    pub(crate) fn protocol(channel: TransportChannel, error: &ProtocolError) -> Self {
        let kind = match error {
            ProtocolError::UnauthorizedSession => TransportErrorKind::Unauthorized,
            _ => TransportErrorKind::Protocol,
        };
        Self::new(kind, channel, error.to_string())
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} transport {:?}: {}",
            self.channel, self.kind, self.message
        )
    }
}

impl Error for TransportError {}

#[derive(Debug, Clone)]
pub struct HostTransportConfig {
    pub session_id: SessionId,
    pub bind_address: IpAddr,
    pub control_port: u16,
    pub sync_port: u16,
    pub audio_port: u16,
    pub outbound_queue_capacity: usize,
    pub event_queue_capacity: usize,
    pub max_peers: usize,
    pub io_timeout: Duration,
    pub operation_timeout: Duration,
    pub max_consecutive_failures: u32,
}

impl HostTransportConfig {
    #[must_use]
    pub fn loopback(session_id: SessionId) -> Self {
        Self {
            session_id,
            bind_address: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            control_port: 0,
            sync_port: 0,
            audio_port: 0,
            outbound_queue_capacity: DEFAULT_TRANSPORT_QUEUE_CAPACITY,
            event_queue_capacity: DEFAULT_TRANSPORT_EVENT_CAPACITY,
            max_peers: DEFAULT_MAX_TRANSPORT_PEERS,
            io_timeout: DEFAULT_IO_TIMEOUT,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
            max_consecutive_failures: DEFAULT_MAX_CONSECUTIVE_FAILURES,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), TransportError> {
        validate_common(
            self.outbound_queue_capacity,
            self.event_queue_capacity,
            self.io_timeout,
            self.operation_timeout,
        )?;
        if self.max_peers == 0 || self.max_peers > MAX_TRANSPORT_PEERS {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "max_peers is outside the supported range",
            ));
        }
        if self.max_consecutive_failures == 0 {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "max_consecutive_failures must be nonzero",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ListenerTransportConfig {
    pub session_id: SessionId,
    pub device_id: DeviceId,
    pub endpoint: NetworkEndpoint,
    pub local_address: IpAddr,
    pub outbound_queue_capacity: usize,
    pub event_queue_capacity: usize,
    pub io_timeout: Duration,
    pub operation_timeout: Duration,
}

impl ListenerTransportConfig {
    #[must_use]
    pub fn loopback(session_id: SessionId, device_id: DeviceId, endpoint: NetworkEndpoint) -> Self {
        Self {
            session_id,
            device_id,
            endpoint,
            local_address: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            outbound_queue_capacity: DEFAULT_TRANSPORT_QUEUE_CAPACITY,
            event_queue_capacity: DEFAULT_TRANSPORT_EVENT_CAPACITY,
            io_timeout: DEFAULT_IO_TIMEOUT,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), TransportError> {
        validate_common(
            self.outbound_queue_capacity,
            self.event_queue_capacity,
            self.io_timeout,
            self.operation_timeout,
        )
    }
}

fn validate_common(
    outbound_queue_capacity: usize,
    event_queue_capacity: usize,
    io_timeout: Duration,
    operation_timeout: Duration,
) -> Result<(), TransportError> {
    if outbound_queue_capacity == 0 || outbound_queue_capacity > MAX_TRANSPORT_QUEUE_CAPACITY {
        return Err(TransportError::new(
            TransportErrorKind::InvalidConfiguration,
            TransportChannel::Runtime,
            "outbound queue capacity is outside the supported range",
        ));
    }
    if event_queue_capacity == 0 || event_queue_capacity > MAX_TRANSPORT_EVENT_CAPACITY {
        return Err(TransportError::new(
            TransportErrorKind::InvalidConfiguration,
            TransportChannel::Runtime,
            "event queue capacity is outside the supported range",
        ));
    }
    if io_timeout.is_zero() || operation_timeout.is_zero() {
        return Err(TransportError::new(
            TransportErrorKind::InvalidConfiguration,
            TransportChannel::Runtime,
            "transport timeouts must be nonzero",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenerDatagramRoutes {
    pub synchronization: SocketAddr,
    pub audio: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportPeer {
    pub device_id: Option<DeviceId>,
    pub control_address: SocketAddr,
}

#[derive(Clone, PartialEq, Eq)]
pub enum TransportEvent {
    PeerAccepted {
        peer: TransportPeer,
        received_at: MonotonicMillis,
    },
    PeerAuthorized {
        peer: TransportPeer,
        routes: ListenerDatagramRoutes,
        received_at: MonotonicMillis,
    },
    FrameReceived {
        channel: TransportChannel,
        peer: TransportPeer,
        frame: ProtocolFrame,
        received_at: MonotonicMillis,
    },
    PeerDisconnected {
        peer: TransportPeer,
        error: Option<TransportError>,
        received_at: MonotonicMillis,
    },
    Rejected {
        channel: TransportChannel,
        source: SocketAddr,
        error: TransportError,
        received_at: MonotonicMillis,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransportCounters {
    pub accepted_control_connections: u64,
    pub closed_control_connections: u64,
    pub control_frames_sent: u64,
    pub control_frames_received: u64,
    pub sync_datagrams_sent: u64,
    pub sync_datagrams_received: u64,
    pub audio_datagrams_sent: u64,
    pub audio_datagrams_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub malformed_frames: u64,
    pub oversized_frames: u64,
    pub unauthorized_frames: u64,
    pub queue_overflows: u64,
    pub delivery_failures: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportDelivery {
    pub report: DeliveryReport,
    pub bytes_sent: u64,
}

impl TransportDelivery {
    pub(crate) fn new(
        intended: u32,
        successful: u32,
        failed: u32,
        bytes_sent: u64,
    ) -> Result<Self, TransportError> {
        let report = DeliveryReport::new(intended, successful, failed).map_err(|error| {
            TransportError::new(
                TransportErrorKind::Delivery,
                TransportChannel::Runtime,
                error.to_string(),
            )
        })?;
        Ok(Self { report, bytes_sent })
    }
}
