use core::fmt;

use crate::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId, SyncConfidence,
};

pub const PROTOCOL_MAGIC: [u8; 4] = *b"SDP2";
pub const PROTOCOL_VERSION: u16 = 2;
pub const FRAME_HEADER_BYTES: usize = 16;
pub const FRAME_HEADER_LENGTH: u16 = 16;
pub const MAX_CONTROL_PAYLOAD_BYTES: usize = 64 * 1024;
pub const MAX_AUDIO_DATAGRAM_BYTES: usize = 4 * 1024;
pub const MAX_PROTOCOL_STRING_BYTES: usize = 256;
pub const MAX_SESSION_NAME_BYTES: usize = 128;
pub const MAX_DISPLAY_NAME_BYTES: usize = 128;
pub const MAX_INVITE_CODE_BYTES: usize = 64;
pub const MAX_REASON_BYTES: usize = 256;
/// Bounds `SyncConfidence::wire_name()`, which is always a short static
/// string ("excellent" is the longest at 9 bytes) -- this is deliberately
/// generous, not a measured value, since the field only ever holds one of
/// the fixed variants from `define_stable_enum!`.
pub const MAX_SYNC_CONFIDENCE_WIRE_NAME_BYTES: usize = 32;
pub const FLAG_PAYLOAD_INTEGRITY: u16 = 0x0001;
pub const SUPPORTED_FRAME_FLAGS: u16 = FLAG_PAYLOAD_INTEGRITY;

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKind {
    Hello = 1,
    JoinRequest = 2,
    JoinApproval = 3,
    JoinRejection = 4,
    Heartbeat = 5,
    StreamStart = 6,
    Pause = 7,
    Stop = 8,
    Disconnect = 9,
    ResyncNotice = 10,
    SynchronizationReport = 11,
    SyncRequest = 100,
    SyncResponse = 101,
    Audio = 200,
}

impl MessageKind {
    #[must_use]
    pub const fn code(self) -> u16 {
        self as u16
    }

    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::JoinRequest => "join_request",
            Self::JoinApproval => "join_approval",
            Self::JoinRejection => "join_rejection",
            Self::Heartbeat => "heartbeat",
            Self::StreamStart => "stream_start",
            Self::Pause => "pause",
            Self::Stop => "stop",
            Self::Disconnect => "disconnect",
            Self::ResyncNotice => "resync_notice",
            Self::SynchronizationReport => "synchronization_report",
            Self::SyncRequest => "sync_request",
            Self::SyncResponse => "sync_response",
            Self::Audio => "audio",
        }
    }

    #[must_use]
    pub const fn is_control(self) -> bool {
        matches!(
            self,
            Self::Hello
                | Self::JoinRequest
                | Self::JoinApproval
                | Self::JoinRejection
                | Self::Heartbeat
                | Self::StreamStart
                | Self::Pause
                | Self::Stop
                | Self::Disconnect
                | Self::ResyncNotice
                | Self::SynchronizationReport
        )
    }

    #[must_use]
    pub const fn maximum_payload_bytes(self) -> usize {
        match self {
            Self::Audio => MAX_AUDIO_DATAGRAM_BYTES - FRAME_HEADER_BYTES,
            _ => MAX_CONTROL_PAYLOAD_BYTES,
        }
    }
}

impl fmt::Display for MessageKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

impl TryFrom<u16> for MessageKind {
    type Error = u16;

    fn try_from(code: u16) -> Result<Self, u16> {
        match code {
            1 => Ok(Self::Hello),
            2 => Ok(Self::JoinRequest),
            3 => Ok(Self::JoinApproval),
            4 => Ok(Self::JoinRejection),
            5 => Ok(Self::Heartbeat),
            6 => Ok(Self::StreamStart),
            7 => Ok(Self::Pause),
            8 => Ok(Self::Stop),
            9 => Ok(Self::Disconnect),
            10 => Ok(Self::ResyncNotice),
            11 => Ok(Self::SynchronizationReport),
            100 => Ok(Self::SyncRequest),
            101 => Ok(Self::SyncResponse),
            200 => Ok(Self::Audio),
            _ => Err(code),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: u16,
    pub kind: MessageKind,
    pub flags: u16,
    pub header_length: u16,
    pub payload_length: u32,
}

impl FrameHeader {
    #[must_use]
    pub const fn new(kind: MessageKind, flags: u16, payload_length: u32) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            kind,
            flags,
            header_length: FRAME_HEADER_LENGTH,
            payload_length,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: DeviceId,
    pub display_name: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Hello {
    pub session_id: SessionId,
    pub session_name: String,
    pub host_name: String,
    pub approval_required: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct JoinRequest {
    pub session_id: SessionId,
    pub device: DeviceIdentity,
    pub invite_code: Option<String>,
    /// The listener's own bound local UDP ports for the synchronization and
    /// audio channels, so the host can authorize datagram routing for this
    /// peer once (if) it approves the join. The host cannot discover these
    /// any other way: the listener's audio channel is receive-only and never
    /// sends a datagram the host could observe a source address from.
    pub sync_port: u16,
    pub audio_port: u16,
}

#[derive(Clone, PartialEq, Eq)]
pub struct JoinApproval {
    pub session_id: SessionId,
    pub listener_id: DeviceId,
    pub trusted_for_future: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct JoinRejection {
    pub session_id: SessionId,
    pub listener_id: DeviceId,
    pub reason: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Heartbeat {
    pub session_id: SessionId,
    pub listener_id: DeviceId,
    pub sent_at_elapsed_ms: MonotonicMillis,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StreamStart {
    pub session_id: SessionId,
    pub stream_id: StreamId,
    pub host_start_time_ms: MonotonicMillis,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples_per_packet: u32,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Pause {
    pub session_id: SessionId,
    pub stream_id: StreamId,
    pub host_pause_time_ms: MonotonicMillis,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Stop {
    pub session_id: SessionId,
    pub stream_id: StreamId,
    pub host_stop_time_ms: MonotonicMillis,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Disconnect {
    pub session_id: SessionId,
    pub listener_id: DeviceId,
    pub reason: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResyncNotice {
    pub session_id: SessionId,
    pub listener_id: DeviceId,
    pub reason: String,
}

/// A listener's own periodic report of its locally-computed synchronization
/// quality (see `ClockSyncEstimator`/`SyncSnapshot`), so the host can
/// populate `AudioEvent::SynchronizationUpdated` for its per-listener
/// diagnostics (D2, `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`).
///
/// This exists because the host cannot compute this itself: a
/// `SyncRequest`/`SyncResponse` round trip only gives the host t1-t3 --
/// `t4` (the listener's local receipt time of the response) never reaches
/// the host over any existing channel, so offset/RTT/confidence are only
/// ever knowable on the listener's own side. The host is a purely passive
/// sync responder by design (`is_control` note above); this message is the
/// listener voluntarily reporting what it already knows, not the host
/// probing for it.
#[derive(Clone, PartialEq)]
pub struct SynchronizationReport {
    pub session_id: SessionId,
    pub listener_id: DeviceId,
    pub confidence: SyncConfidence,
    pub offset_ms: f64,
    pub round_trip_ms: f64,
    pub drift_ppm: f64,
}

#[derive(Clone, PartialEq)]
pub enum ControlMessage {
    Hello(Hello),
    JoinRequest(JoinRequest),
    JoinApproval(JoinApproval),
    JoinRejection(JoinRejection),
    Heartbeat(Heartbeat),
    StreamStart(StreamStart),
    Pause(Pause),
    Stop(Stop),
    Disconnect(Disconnect),
    ResyncNotice(ResyncNotice),
    SynchronizationReport(SynchronizationReport),
}

impl ControlMessage {
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        match self {
            Self::Hello(_) => MessageKind::Hello,
            Self::JoinRequest(_) => MessageKind::JoinRequest,
            Self::JoinApproval(_) => MessageKind::JoinApproval,
            Self::JoinRejection(_) => MessageKind::JoinRejection,
            Self::Heartbeat(_) => MessageKind::Heartbeat,
            Self::StreamStart(_) => MessageKind::StreamStart,
            Self::Pause(_) => MessageKind::Pause,
            Self::Stop(_) => MessageKind::Stop,
            Self::Disconnect(_) => MessageKind::Disconnect,
            Self::ResyncNotice(_) => MessageKind::ResyncNotice,
            Self::SynchronizationReport(_) => MessageKind::SynchronizationReport,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        match self {
            Self::Hello(value) => &value.session_id,
            Self::JoinRequest(value) => &value.session_id,
            Self::JoinApproval(value) => &value.session_id,
            Self::JoinRejection(value) => &value.session_id,
            Self::Heartbeat(value) => &value.session_id,
            Self::StreamStart(value) => &value.session_id,
            Self::Pause(value) => &value.session_id,
            Self::Stop(value) => &value.session_id,
            Self::Disconnect(value) => &value.session_id,
            Self::ResyncNotice(value) => &value.session_id,
            Self::SynchronizationReport(value) => &value.session_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequest {
    pub session_id: SessionId,
    pub correlation_id: u64,
    pub t1_listener_send_elapsed_ms: MonotonicMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncResponse {
    pub session_id: SessionId,
    pub correlation_id: u64,
    pub t1_listener_send_elapsed_ms: MonotonicMillis,
    pub t2_host_receive_elapsed_ms: MonotonicMillis,
    pub t3_host_send_elapsed_ms: MonotonicMillis,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    PcmS16Le = 1,
}

impl AudioCodec {
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for AudioCodec {
    type Error = u8;

    fn try_from(code: u8) -> Result<Self, u8> {
        match code {
            1 => Ok(Self::PcmS16Le),
            _ => Err(code),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDatagram {
    pub session_id: SessionId,
    pub stream_id: StreamId,
    pub sequence: PacketSequence,
    pub codec: AudioCodec,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples_per_packet: u32,
    pub first_sample_index: SampleIndex,
    pub host_presentation_time_ms: MonotonicMillis,
    pub payload: Vec<u8>,
}

#[derive(Clone, PartialEq)]
pub enum ProtocolFrame {
    Control(ControlMessage),
    SyncRequest(SyncRequest),
    SyncResponse(SyncResponse),
    Audio(AudioDatagram),
}

impl ProtocolFrame {
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        match self {
            Self::Control(message) => message.kind(),
            Self::SyncRequest(_) => MessageKind::SyncRequest,
            Self::SyncResponse(_) => MessageKind::SyncResponse,
            Self::Audio(_) => MessageKind::Audio,
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        match self {
            Self::Control(message) => message.session_id(),
            Self::SyncRequest(message) => &message.session_id,
            Self::SyncResponse(message) => &message.session_id,
            Self::Audio(message) => &message.session_id,
        }
    }
}
