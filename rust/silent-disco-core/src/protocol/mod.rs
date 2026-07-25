mod codec;
mod types;

pub use codec::{
    DecodePolicy, ParseFailureClass, ProtocolDecoder, ProtocolDiagnosticCounters, ProtocolError,
    crc32, decode_frame, decode_header, encode_frame,
};
pub use types::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, Disconnect, FLAG_PAYLOAD_INTEGRITY,
    FRAME_HEADER_BYTES, FrameHeader, Heartbeat, Hello, JoinApproval, JoinRejection, JoinRequest,
    MAX_AUDIO_DATAGRAM_BYTES, MAX_CONTROL_PAYLOAD_BYTES, MAX_DISPLAY_NAME_BYTES,
    MAX_INVITE_CODE_BYTES, MAX_PROTOCOL_STRING_BYTES, MAX_REASON_BYTES, MAX_SESSION_NAME_BYTES,
    MessageKind, PROTOCOL_MAGIC, PROTOCOL_VERSION, Pause, ProtocolFrame, ResyncNotice,
    SUPPORTED_FRAME_FLAGS, Stop, StreamStart, SyncRequest, SyncResponse,
};
