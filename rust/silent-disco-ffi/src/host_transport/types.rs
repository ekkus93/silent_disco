use core::fmt;

use silent_disco_core::transport::{TransportCounters, TransportError, TransportErrorKind};

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
    /// Best-effort telemetry or diagnostics dropped on queue pressure/closure.
    pub best_effort_event_drops: u64,
    /// New TCP peers refused with a busy reply (capacity or actor pressure).
    pub busy_admission_refusals: u64,
    /// Critical events that could not reach the actor before timeout/shutdown.
    pub authoritative_event_failures: u64,
}

impl From<TransportCounters> for FfiHostTransportCounters {
    fn from(counters: TransportCounters) -> Self {
        Self {
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
            best_effort_event_drops: counters.best_effort_event_drops,
            busy_admission_refusals: counters.busy_admission_refusals,
            authoritative_event_failures: counters.authoritative_event_failures,
        }
    }
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
        /// Exact TCP source of this join, used only for generation-bound
        /// refusal. It is not a verified listener identity or trust proof.
        control_address: String,
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
    /// A real TCP peer's terminal event, bound to the socket that closed.
    /// The legacy address-less variant remains for local poll failures.
    PeerDisconnectedAt {
        control_address: String,
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

#[cfg(test)]
mod counter_mapping_tests {
    use super::*;

    /// Every core counter must reach the binding under its own name; unique
    /// values make a swapped or dropped field fail loudly.
    #[test]
    fn host_counters_expose_every_pressure_counter_distinctly() {
        let core = TransportCounters {
            accepted_control_connections: 1,
            closed_control_connections: 2,
            control_frames_sent: 3,
            control_frames_received: 4,
            sync_datagrams_sent: 5,
            sync_datagrams_received: 6,
            audio_datagrams_sent: 7,
            audio_datagrams_received: 8,
            bytes_sent: 9,
            bytes_received: 10,
            malformed_frames: 11,
            oversized_frames: 12,
            unauthorized_frames: 13,
            queue_overflows: 14,
            delivery_failures: 15,
            best_effort_event_drops: 16,
            busy_admission_refusals: 17,
            authoritative_event_failures: 18,
        };
        let ffi = FfiHostTransportCounters::from(core);
        assert_eq!(ffi.accepted_control_connections, 1);
        assert_eq!(ffi.closed_control_connections, 2);
        assert_eq!(ffi.control_frames_sent, 3);
        assert_eq!(ffi.control_frames_received, 4);
        assert_eq!(ffi.sync_datagrams_sent, 5);
        assert_eq!(ffi.sync_datagrams_received, 6);
        assert_eq!(ffi.audio_datagrams_sent, 7);
        assert_eq!(ffi.audio_datagrams_received, 8);
        assert_eq!(ffi.bytes_sent, 9);
        assert_eq!(ffi.bytes_received, 10);
        assert_eq!(ffi.malformed_frames, 11);
        assert_eq!(ffi.oversized_frames, 12);
        assert_eq!(ffi.unauthorized_frames, 13);
        assert_eq!(ffi.queue_overflows, 14);
        assert_eq!(ffi.delivery_failures, 15);
        assert_eq!(ffi.best_effort_event_drops, 16);
        assert_eq!(ffi.busy_admission_refusals, 17);
        assert_eq!(ffi.authoritative_event_failures, 18);
    }
}
