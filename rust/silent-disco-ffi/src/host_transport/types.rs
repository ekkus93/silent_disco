use core::fmt;

use silent_disco_core::transport::{TransportError, TransportErrorKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiHostTransportCounters {
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

/// Delivery accounting for a host-issued send/broadcast, mirroring
/// `FfiDeliveryReport` in shape (kept separate: this one additionally
/// reports bytes actually written).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiHostTransportDelivery {
    pub intended_peers: u32,
    pub successful_peers: u32,
    pub failed_peers: u32,
    pub bytes_sent: u64,
}

/// One typed event surfaced from the shared Rust host transport.
///
/// Mirrors `FfiListenerTransportEvent`'s scope: `SyncRequestReceived` now
/// surfaces the synchronization datagrams the transport already receives
/// internally, so a caller can compute and send a real response.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiHostTransportEvent {
    /// A listener's TCP control connection was accepted, before any
    /// protocol message has been read from it.
    PeerAccepted {
        control_address: String,
    },
    JoinRequestReceived {
        device_id: String,
        display_name: String,
        invite_code: Option<String>,
        /// The listener's bound sync/audio UDP ports -- pass these to
        /// `authorize_listener` if/when the join is approved, so the host
        /// can actually route synchronization and audio datagrams to them.
        sync_port: u16,
        audio_port: u16,
    },
    Heartbeat {
        listener_id: String,
    },
    ResyncNotice {
        listener_id: String,
        reason: String,
    },
    /// A listener's clock-sync probe. `listener_id` is `None` if the
    /// underlying peer's identity isn't yet known at the transport layer.
    SyncRequestReceived {
        listener_id: Option<String>,
        correlation_id: u64,
        t1_listener_send_elapsed_ms: u64,
    },
    /// A listener's control connection closed or failed.
    PeerDisconnected {
        listener_id: Option<String>,
        message: Option<String>,
    },
    /// A malformed or unauthorized frame was rejected by the transport.
    Rejected {
        message: String,
    },
}

/// Explicit, distinguishable failure exposed to the Android binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiHostTransportError {
    /// The requested bind address, ports, or session ID were invalid.
    InvalidConfiguration(String),
    /// The requested peer is not a currently connected control peer.
    Unauthorized(String),
    /// A bounded wait elapsed without completion.
    Timeout(String),
    /// The local socket could not be bound/listened on.
    BindFailed(String),
    /// An I/O failure occurred on an already-open transport.
    Io(String),
    /// The handle is already shut down.
    Closed(String),
}

impl fmt::Display for FfiHostTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Unauthorized(message)
            | Self::Timeout(message)
            | Self::BindFailed(message)
            | Self::Io(message)
            | Self::Closed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FfiHostTransportError {}

impl From<TransportError> for FfiHostTransportError {
    fn from(error: TransportError) -> Self {
        let message = error.to_string();
        match error.kind {
            TransportErrorKind::InvalidConfiguration => Self::InvalidConfiguration(message),
            TransportErrorKind::Unauthorized => Self::Unauthorized(message),
            TransportErrorKind::Timeout => Self::Timeout(message),
            TransportErrorKind::Bind | TransportErrorKind::Listen => Self::BindFailed(message),
            TransportErrorKind::Connect | TransportErrorKind::PeerNotFound => {
                Self::Unauthorized(message)
            }
            TransportErrorKind::Io
            | TransportErrorKind::Protocol
            | TransportErrorKind::QueueFull
            | TransportErrorKind::Delivery
            | TransportErrorKind::WorkerPanicked => Self::Io(message),
            TransportErrorKind::ShuttingDown => Self::Closed(message),
        }
    }
}
