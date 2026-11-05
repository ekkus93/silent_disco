use core::fmt;

use silent_disco_core::transport::{TransportCounters, TransportError, TransportErrorKind};

/// Binding-facing preview of a validated desktop manual connection payload.
///
/// Produced by [`super::parse_manual_host_endpoint`] for live UI validation
/// before the listener attempts to connect.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiManualHostEndpoint {
    pub host_address: String,
    pub control_port: u16,
    pub sync_port: u16,
    pub audio_port: u16,
    pub session_id: String,
    pub protocol_version: u16,
    pub invite_code_required: bool,
    pub expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiListenerDatagramRoutes {
    pub local_synchronization_port: u16,
    pub local_audio_port: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiListenerTransportCounters {
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
    /// Critical events that could not reach the actor before timeout/shutdown.
    pub authoritative_event_failures: u64,
}

impl From<TransportCounters> for FfiListenerTransportCounters {
    fn from(counters: TransportCounters) -> Self {
        Self {
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
            authoritative_event_failures: counters.authoritative_event_failures,
        }
    }
}

/// One typed event surfaced from the shared Rust listener transport.
///
/// `StreamStarted`/`Paused`/`Stopped` now carry the real wire fields (stream
/// identity, format, presentation time) so a caller can actually start/stop
/// playback from them; `SyncResponseReceived`/`AudioReceived` surface the
/// synchronization and audio datagrams the transport already validates and
/// receives internally.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FfiListenerTransportEvent {
    Hello {
        session_name: String,
        host_name: String,
        approval_required: bool,
    },
    JoinApproved {
        trusted_for_future: bool,
    },
    JoinRejected {
        reason: String,
    },
    /// The host sent an explicit protocol `Disconnect` message.
    HostDisconnected {
        reason: String,
    },
    /// The control connection closed or failed without an explicit protocol message.
    ConnectionClosed {
        message: Option<String>,
    },
    StreamStarted {
        stream_id: String,
        host_start_time_ms: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        current_entry_id: Option<String>,
        current_display_name: Option<String>,
    },
    TrackBoundary {
        stream_id: String,
        first_sample_index: u64,
        entry_id: String,
        display_name: String,
    },
    Paused {
        stream_id: String,
        host_pause_time_ms: u64,
    },
    Stopped {
        stream_id: String,
        host_stop_time_ms: u64,
    },
    /// A clock-sync response echoing one of this listener's own probes.
    ///
    /// `t4` (local receive time) is not carried on the wire -- the caller
    /// supplies it as the moment this event is observed.
    SyncResponseReceived {
        correlation_id: u64,
        t1_listener_send_elapsed_ms: u64,
        t2_host_receive_elapsed_ms: u64,
        t3_host_send_elapsed_ms: u64,
        /// This transport's own clock reading at the moment the response's
        /// receiver thread pulled it off the socket -- i.e. as close to `t4`
        /// as this layer can get, before any FFI/dispatch delay. On a
        /// *different* timeline than `now_ms` on the playback runtime, so a
        /// caller combining the two must first translate through
        /// `FfiListenerTransportHandle::now_ms`, not use this value
        /// directly as `t4`.
        received_at_elapsed_ms: u64,
    },
    /// One inbound audio datagram, in wire order, not yet scheduled for playback.
    AudioReceived {
        stream_id: String,
        sequence: u64,
        sample_rate: u32,
        channels: u16,
        samples_per_packet: u32,
        first_sample_index: u64,
        host_presentation_time_ms: u64,
        payload: Vec<u8>,
    },
    /// A malformed or unauthorized frame was rejected by the transport.
    Rejected {
        message: String,
    },
}

/// Explicit, distinguishable failure exposed to the Android binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiListenerTransportError {
    /// The manual connection payload itself was malformed, blank, or out of range.
    InvalidEndpoint(String),
    /// The desktop advertised a protocol version this build does not support.
    UnsupportedProtocolVersion(String),
    /// The transport rejected a message as unauthorized for this session/device.
    Unauthorized(String),
    /// A bounded wait elapsed without completion.
    Timeout(String),
    /// The TCP/UDP connection to the host could not be established.
    ConnectionFailed(String),
    /// An I/O failure occurred on an already-open transport.
    Io(String),
    /// The handle is already shut down.
    Closed(String),
}

impl fmt::Display for FfiListenerTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEndpoint(message)
            | Self::UnsupportedProtocolVersion(message)
            | Self::Unauthorized(message)
            | Self::Timeout(message)
            | Self::ConnectionFailed(message)
            | Self::Io(message)
            | Self::Closed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FfiListenerTransportError {}

impl From<TransportError> for FfiListenerTransportError {
    fn from(error: TransportError) -> Self {
        let message = error.to_string();
        match error.kind {
            TransportErrorKind::InvalidConfiguration => Self::InvalidEndpoint(message),
            TransportErrorKind::Protocol => Self::UnsupportedProtocolVersion(message),
            TransportErrorKind::Unauthorized => Self::Unauthorized(message),
            TransportErrorKind::Timeout => Self::Timeout(message),
            TransportErrorKind::Bind
            | TransportErrorKind::Listen
            | TransportErrorKind::Connect
            | TransportErrorKind::PeerNotFound => Self::ConnectionFailed(message),
            TransportErrorKind::Io
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

    #[test]
    fn listener_counters_expose_event_loss_counters_distinctly() {
        let core = TransportCounters {
            control_frames_sent: 1,
            control_frames_received: 2,
            sync_datagrams_sent: 3,
            sync_datagrams_received: 4,
            audio_datagrams_sent: 5,
            audio_datagrams_received: 6,
            bytes_sent: 7,
            bytes_received: 8,
            malformed_frames: 9,
            oversized_frames: 10,
            unauthorized_frames: 11,
            queue_overflows: 12,
            delivery_failures: 13,
            best_effort_event_drops: 14,
            authoritative_event_failures: 15,
            ..TransportCounters::default()
        };
        let ffi = FfiListenerTransportCounters::from(core);
        assert_eq!(ffi.control_frames_sent, 1);
        assert_eq!(ffi.control_frames_received, 2);
        assert_eq!(ffi.sync_datagrams_sent, 3);
        assert_eq!(ffi.sync_datagrams_received, 4);
        assert_eq!(ffi.audio_datagrams_sent, 5);
        assert_eq!(ffi.audio_datagrams_received, 6);
        assert_eq!(ffi.bytes_sent, 7);
        assert_eq!(ffi.bytes_received, 8);
        assert_eq!(ffi.malformed_frames, 9);
        assert_eq!(ffi.oversized_frames, 10);
        assert_eq!(ffi.unauthorized_frames, 11);
        assert_eq!(ffi.queue_overflows, 12);
        assert_eq!(ffi.delivery_failures, 13);
        assert_eq!(ffi.best_effort_event_drops, 14);
        assert_eq!(ffi.authoritative_event_failures, 15);
    }
}
