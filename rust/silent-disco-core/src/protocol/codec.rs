use core::fmt;
use std::error::Error;

use crate::{
    domain::{
        DeviceId, IdDecodeError, MonotonicMillis, OperationId, PacketSequence, SampleIndex,
        SessionId, StreamId,
    },
    error::{CoreError, CoreErrorCode, CoreSubsystem, ErrorSeverity},
};

use super::types::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, Disconnect, FLAG_PAYLOAD_INTEGRITY,
    FRAME_HEADER_BYTES, FrameHeader, Heartbeat, Hello, JoinApproval, JoinRejection, JoinRequest,
    MAX_AUDIO_DATAGRAM_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_INVITE_CODE_BYTES, MAX_REASON_BYTES,
    MAX_SESSION_NAME_BYTES, MessageKind, PROTOCOL_MAGIC, PROTOCOL_VERSION, Pause, ProtocolFrame,
    ResyncNotice, SUPPORTED_FRAME_FLAGS, Stop, StreamStart, SyncRequest, SyncResponse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseFailureClass {
    Malformed,
    Unsupported,
    Unauthorized,
    Stale,
    Oversized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    Truncated,
    TrailingBytes,
    InvalidMagic,
    UnsupportedVersion { version: u16 },
    UnsupportedMessageKind { kind: u16 },
    UnsupportedFlags { flags: u16 },
    InvalidHeaderLength { length: u16 },
    PayloadTooLarge { actual: usize, maximum: usize },
    LengthMismatch { declared: usize, actual: usize },
    InvalidBoolean { value: u8 },
    InvalidUtf8,
    InvalidIdentifier,
    InvalidField { field: &'static str },
    UnsupportedAudioCodec { codec: u8 },
    IntegrityMismatch,
    UnauthorizedSession,
    StaleAudioSequence,
}

impl ProtocolError {
    #[must_use]
    pub const fn classification(&self) -> ParseFailureClass {
        match self {
            Self::UnsupportedVersion { .. }
            | Self::UnsupportedMessageKind { .. }
            | Self::UnsupportedFlags { .. }
            | Self::UnsupportedAudioCodec { .. } => ParseFailureClass::Unsupported,
            Self::PayloadTooLarge { .. } => ParseFailureClass::Oversized,
            Self::UnauthorizedSession => ParseFailureClass::Unauthorized,
            Self::StaleAudioSequence => ParseFailureClass::Stale,
            Self::Truncated
            | Self::TrailingBytes
            | Self::InvalidMagic
            | Self::InvalidHeaderLength { .. }
            | Self::LengthMismatch { .. }
            | Self::InvalidBoolean { .. }
            | Self::InvalidUtf8
            | Self::InvalidIdentifier
            | Self::InvalidField { .. }
            | Self::IntegrityMismatch => ParseFailureClass::Malformed,
        }
    }

    #[must_use]
    pub const fn core_error_code(&self) -> CoreErrorCode {
        match self {
            Self::UnsupportedVersion { .. } => CoreErrorCode::UnsupportedProtocolVersion,
            Self::UnsupportedMessageKind { .. } | Self::UnsupportedAudioCodec { .. } => {
                CoreErrorCode::UnsupportedMessageKind
            }
            Self::PayloadTooLarge { .. } => CoreErrorCode::ProtocolFrameTooLarge,
            Self::IntegrityMismatch => CoreErrorCode::IntegrityCheckFailed,
            Self::UnauthorizedSession => CoreErrorCode::TransportDeliveryFailed,
            _ => CoreErrorCode::MalformedProtocolFrame,
        }
    }

    #[must_use]
    pub fn to_core_error(&self, operation_id: Option<OperationId>) -> CoreError {
        CoreError {
            code: self.core_error_code(),
            message: self.to_string(),
            subsystem: match self {
                Self::UnauthorizedSession => CoreSubsystem::Transport,
                _ => CoreSubsystem::Protocol,
            },
            severity: ErrorSeverity::Error,
            retryable: matches!(self, Self::StaleAudioSequence),
            operation_id,
            context: Vec::new(),
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("protocol input is truncated"),
            Self::TrailingBytes => formatter.write_str("protocol input contains trailing bytes"),
            Self::InvalidMagic => formatter.write_str("protocol magic is invalid"),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "protocol version {version} is unsupported")
            }
            Self::UnsupportedMessageKind { kind } => {
                write!(formatter, "protocol message kind {kind} is unsupported")
            }
            Self::UnsupportedFlags { flags } => {
                write!(formatter, "protocol flags 0x{flags:04x} are unsupported")
            }
            Self::InvalidHeaderLength { length } => {
                write!(formatter, "protocol header length {length} is invalid")
            }
            Self::PayloadTooLarge { actual, maximum } => write!(
                formatter,
                "protocol payload is {actual} bytes; maximum is {maximum} bytes"
            ),
            Self::LengthMismatch { declared, actual } => write!(
                formatter,
                "protocol payload length declares {declared} bytes but {actual} bytes are present"
            ),
            Self::InvalidBoolean { value } => {
                write!(formatter, "protocol boolean value {value} is invalid")
            }
            Self::InvalidUtf8 => formatter.write_str("protocol text is not valid UTF-8"),
            Self::InvalidIdentifier => formatter.write_str("protocol identifier is invalid"),
            Self::InvalidField { field } => write!(formatter, "protocol field {field} is invalid"),
            Self::UnsupportedAudioCodec { codec } => {
                write!(formatter, "audio codec {codec} is unsupported")
            }
            Self::IntegrityMismatch => formatter.write_str("audio payload integrity check failed"),
            Self::UnauthorizedSession => {
                formatter.write_str("protocol frame belongs to an unauthorized session")
            }
            Self::StaleAudioSequence => formatter.write_str("audio frame sequence is stale"),
        }
    }
}

impl Error for ProtocolError {}

impl From<IdDecodeError> for ProtocolError {
    fn from(_: IdDecodeError) -> Self {
        Self::InvalidIdentifier
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProtocolDiagnosticCounters {
    pub malformed: u64,
    pub unsupported: u64,
    pub unauthorized: u64,
    pub stale: u64,
    pub oversized: u64,
}

impl ProtocolDiagnosticCounters {
    fn record(&mut self, classification: ParseFailureClass) {
        match classification {
            ParseFailureClass::Malformed => self.malformed = self.malformed.saturating_add(1),
            ParseFailureClass::Unsupported => {
                self.unsupported = self.unsupported.saturating_add(1);
            }
            ParseFailureClass::Unauthorized => {
                self.unauthorized = self.unauthorized.saturating_add(1);
            }
            ParseFailureClass::Stale => self.stale = self.stale.saturating_add(1),
            ParseFailureClass::Oversized => self.oversized = self.oversized.saturating_add(1),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DecodePolicy<'a> {
    pub expected_session_id: Option<&'a SessionId>,
    pub minimum_audio_sequence: Option<PacketSequence>,
}

#[derive(Debug, Default)]
pub struct ProtocolDecoder {
    counters: ProtocolDiagnosticCounters,
}

impl ProtocolDecoder {
    #[must_use]
    pub const fn counters(&self) -> ProtocolDiagnosticCounters {
        self.counters
    }

    /// Decodes one complete frame and applies optional authorization/staleness policy.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] when framing, payload validation, authorization,
    /// or sequence policy fails. Every failure is counted by classification.
    pub fn decode(
        &mut self,
        bytes: &[u8],
        policy: DecodePolicy<'_>,
    ) -> Result<ProtocolFrame, ProtocolError> {
        let result = decode_frame(bytes).and_then(|frame| {
            if policy
                .expected_session_id
                .is_some_and(|expected| frame.session_id() != expected)
            {
                return Err(ProtocolError::UnauthorizedSession);
            }
            if let (ProtocolFrame::Audio(audio), Some(minimum)) =
                (&frame, policy.minimum_audio_sequence)
                && audio.sequence < minimum
            {
                return Err(ProtocolError::StaleAudioSequence);
            }
            Ok(frame)
        });
        if let Err(error) = &result {
            self.counters.record(error.classification());
        }
        result
    }
}

#[derive(Debug, Default)]
struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn put_u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn put_u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn put_bool(&mut self, value: bool) {
        self.put_u8(u8::from(value));
    }

    fn put_bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn put_string(
        &mut self,
        field: &'static str,
        value: &str,
        maximum: usize,
        secret: bool,
    ) -> Result<(), ProtocolError> {
        validate_text(field, value, maximum, secret)?;
        let length =
            u16::try_from(value.len()).map_err(|_| ProtocolError::InvalidField { field })?;
        self.put_u16(length);
        self.put_bytes(value.as_bytes());
        Ok(())
    }

    fn put_optional_string(
        &mut self,
        field: &'static str,
        value: Option<&str>,
        maximum: usize,
        secret: bool,
    ) -> Result<(), ProtocolError> {
        if let Some(value) = value {
            self.put_u8(1);
            self.put_string(field, value, maximum, secret)
        } else {
            self.put_u8(0);
            Ok(())
        }
    }

    fn put_session_id(&mut self, value: &SessionId) -> Result<(), ProtocolError> {
        self.put_string(
            "session_id",
            value.as_str(),
            crate::domain::MAX_IDENTIFIER_BYTES,
            false,
        )
    }

    fn put_stream_id(&mut self, value: &StreamId) -> Result<(), ProtocolError> {
        self.put_string(
            "stream_id",
            value.as_str(),
            crate::domain::MAX_IDENTIFIER_BYTES,
            false,
        )
    }

    fn put_device_id(&mut self, value: &DeviceId) -> Result<(), ProtocolError> {
        self.put_string(
            "device_id",
            value.as_str(),
            crate::domain::MAX_IDENTIFIER_BYTES,
            false,
        )
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[derive(Debug)]
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(ProtocolError::Truncated)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(ProtocolError::Truncated)?;
        self.position = end;
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<u8, ProtocolError> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16, ProtocolError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, ProtocolError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, ProtocolError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| ProtocolError::Truncated)?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn read_bool(&mut self) -> Result<bool, ProtocolError> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(ProtocolError::InvalidBoolean { value }),
        }
    }

    fn read_string(
        &mut self,
        field: &'static str,
        maximum: usize,
        secret: bool,
    ) -> Result<String, ProtocolError> {
        let length = usize::from(self.read_u16()?);
        if length > maximum {
            return Err(ProtocolError::PayloadTooLarge {
                actual: length,
                maximum,
            });
        }
        let value =
            core::str::from_utf8(self.take(length)?).map_err(|_| ProtocolError::InvalidUtf8)?;
        validate_text(field, value, maximum, secret)?;
        Ok(value.to_owned())
    }

    fn read_optional_string(
        &mut self,
        field: &'static str,
        maximum: usize,
        secret: bool,
    ) -> Result<Option<String>, ProtocolError> {
        match self.read_u8()? {
            0 => Ok(None),
            1 => self.read_string(field, maximum, secret).map(Some),
            value => Err(ProtocolError::InvalidBoolean { value }),
        }
    }

    fn read_session_id(&mut self) -> Result<SessionId, ProtocolError> {
        let value = self.read_string("session_id", crate::domain::MAX_IDENTIFIER_BYTES, false)?;
        SessionId::new(value).map_err(|_| ProtocolError::InvalidIdentifier)
    }

    fn read_stream_id(&mut self) -> Result<StreamId, ProtocolError> {
        let value = self.read_string("stream_id", crate::domain::MAX_IDENTIFIER_BYTES, false)?;
        StreamId::new(value).map_err(|_| ProtocolError::InvalidIdentifier)
    }

    fn read_device_id(&mut self) -> Result<DeviceId, ProtocolError> {
        let value = self.read_string("device_id", crate::domain::MAX_IDENTIFIER_BYTES, false)?;
        DeviceId::new(value).map_err(|_| ProtocolError::InvalidIdentifier)
    }

    fn finish(self) -> Result<(), ProtocolError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(ProtocolError::TrailingBytes)
        }
    }
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum: usize,
    secret: bool,
) -> Result<(), ProtocolError> {
    if value.trim().is_empty() || value.trim() != value || value.len() > maximum {
        return Err(ProtocolError::InvalidField { field });
    }
    if value.chars().any(char::is_control) {
        return Err(ProtocolError::InvalidField { field });
    }
    if secret && value.chars().any(char::is_whitespace) {
        return Err(ProtocolError::InvalidField { field });
    }
    Ok(())
}

fn validate_audio_parameters(audio: &AudioDatagram) -> Result<(), ProtocolError> {
    if !(8_000..=384_000).contains(&audio.sample_rate) {
        return Err(ProtocolError::InvalidField {
            field: "sample_rate",
        });
    }
    if !(1..=8).contains(&audio.channels) {
        return Err(ProtocolError::InvalidField { field: "channels" });
    }
    if audio.samples_per_packet == 0 {
        return Err(ProtocolError::InvalidField {
            field: "samples_per_packet",
        });
    }
    let expected = usize::try_from(audio.samples_per_packet)
        .ok()
        .and_then(|samples| samples.checked_mul(usize::from(audio.channels)))
        .and_then(|sample_values| sample_values.checked_mul(2))
        .ok_or(ProtocolError::InvalidField {
            field: "samples_per_packet",
        })?;
    if expected != audio.payload.len() {
        return Err(ProtocolError::LengthMismatch {
            declared: expected,
            actual: audio.payload.len(),
        });
    }
    Ok(())
}

fn encode_control(message: &ControlMessage) -> Result<Vec<u8>, ProtocolError> {
    let mut output = Encoder::with_capacity(256);
    match message {
        ControlMessage::Hello(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_string(
                "session_name",
                &value.session_name,
                MAX_SESSION_NAME_BYTES,
                false,
            )?;
            output.put_string("host_name", &value.host_name, MAX_DISPLAY_NAME_BYTES, false)?;
            output.put_bool(value.approval_required);
        }
        ControlMessage::JoinRequest(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.device.device_id)?;
            output.put_string(
                "display_name",
                &value.device.display_name,
                MAX_DISPLAY_NAME_BYTES,
                false,
            )?;
            output.put_optional_string(
                "invite_code",
                value.invite_code.as_deref(),
                MAX_INVITE_CODE_BYTES,
                true,
            )?;
        }
        ControlMessage::JoinApproval(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_bool(value.trusted_for_future);
        }
        ControlMessage::JoinRejection(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_string("reason", &value.reason, MAX_REASON_BYTES, false)?;
        }
        ControlMessage::Heartbeat(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_u64(value.sent_at_elapsed_ms.get());
        }
        ControlMessage::StreamStart(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_stream_id(&value.stream_id)?;
            output.put_u64(value.host_start_time_ms.get());
            output.put_u32(value.sample_rate);
            output.put_u16(value.channels);
            output.put_u32(value.samples_per_packet);
        }
        ControlMessage::Pause(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_stream_id(&value.stream_id)?;
            output.put_u64(value.host_pause_time_ms.get());
        }
        ControlMessage::Stop(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_stream_id(&value.stream_id)?;
            output.put_u64(value.host_stop_time_ms.get());
        }
        ControlMessage::Disconnect(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_string("reason", &value.reason, MAX_REASON_BYTES, false)?;
        }
        ControlMessage::ResyncNotice(value) => {
            output.put_session_id(&value.session_id)?;
            output.put_device_id(&value.listener_id)?;
            output.put_string("reason", &value.reason, MAX_REASON_BYTES, false)?;
        }
    }
    Ok(output.finish())
}

fn decode_control(kind: MessageKind, payload: &[u8]) -> Result<ControlMessage, ProtocolError> {
    let mut input = Reader::new(payload);
    let message = match kind {
        MessageKind::Hello => ControlMessage::Hello(Hello {
            session_id: input.read_session_id()?,
            session_name: input.read_string("session_name", MAX_SESSION_NAME_BYTES, false)?,
            host_name: input.read_string("host_name", MAX_DISPLAY_NAME_BYTES, false)?,
            approval_required: input.read_bool()?,
        }),
        MessageKind::JoinRequest => ControlMessage::JoinRequest(JoinRequest {
            session_id: input.read_session_id()?,
            device: DeviceIdentity {
                device_id: input.read_device_id()?,
                display_name: input.read_string("display_name", MAX_DISPLAY_NAME_BYTES, false)?,
            },
            invite_code: input.read_optional_string("invite_code", MAX_INVITE_CODE_BYTES, true)?,
        }),
        MessageKind::JoinApproval => ControlMessage::JoinApproval(JoinApproval {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            trusted_for_future: input.read_bool()?,
        }),
        MessageKind::JoinRejection => ControlMessage::JoinRejection(JoinRejection {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            reason: input.read_string("reason", MAX_REASON_BYTES, false)?,
        }),
        MessageKind::Heartbeat => ControlMessage::Heartbeat(Heartbeat {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            sent_at_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
        }),
        MessageKind::StreamStart => ControlMessage::StreamStart(StreamStart {
            session_id: input.read_session_id()?,
            stream_id: input.read_stream_id()?,
            host_start_time_ms: MonotonicMillis::new(input.read_u64()?),
            sample_rate: input.read_u32()?,
            channels: input.read_u16()?,
            samples_per_packet: input.read_u32()?,
        }),
        MessageKind::Pause => ControlMessage::Pause(Pause {
            session_id: input.read_session_id()?,
            stream_id: input.read_stream_id()?,
            host_pause_time_ms: MonotonicMillis::new(input.read_u64()?),
        }),
        MessageKind::Stop => ControlMessage::Stop(Stop {
            session_id: input.read_session_id()?,
            stream_id: input.read_stream_id()?,
            host_stop_time_ms: MonotonicMillis::new(input.read_u64()?),
        }),
        MessageKind::Disconnect => ControlMessage::Disconnect(Disconnect {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            reason: input.read_string("reason", MAX_REASON_BYTES, false)?,
        }),
        MessageKind::ResyncNotice => ControlMessage::ResyncNotice(ResyncNotice {
            session_id: input.read_session_id()?,
            listener_id: input.read_device_id()?,
            reason: input.read_string("reason", MAX_REASON_BYTES, false)?,
        }),
        _ => {
            return Err(ProtocolError::UnsupportedMessageKind { kind: kind.code() });
        }
    };
    input.finish()?;
    Ok(message)
}

fn encode_sync_request(value: &SyncRequest) -> Result<Vec<u8>, ProtocolError> {
    let mut output = Encoder::with_capacity(160);
    output.put_session_id(&value.session_id)?;
    output.put_u64(value.correlation_id);
    output.put_u64(value.t1_listener_send_elapsed_ms.get());
    Ok(output.finish())
}

fn decode_sync_request(payload: &[u8]) -> Result<SyncRequest, ProtocolError> {
    let mut input = Reader::new(payload);
    let value = SyncRequest {
        session_id: input.read_session_id()?,
        correlation_id: input.read_u64()?,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
    };
    input.finish()?;
    Ok(value)
}

fn encode_sync_response(value: &SyncResponse) -> Result<Vec<u8>, ProtocolError> {
    if value.t3_host_send_elapsed_ms < value.t2_host_receive_elapsed_ms {
        return Err(ProtocolError::InvalidField {
            field: "host_timestamp_order",
        });
    }
    let mut output = Encoder::with_capacity(176);
    output.put_session_id(&value.session_id)?;
    output.put_u64(value.correlation_id);
    output.put_u64(value.t1_listener_send_elapsed_ms.get());
    output.put_u64(value.t2_host_receive_elapsed_ms.get());
    output.put_u64(value.t3_host_send_elapsed_ms.get());
    Ok(output.finish())
}

fn decode_sync_response(payload: &[u8]) -> Result<SyncResponse, ProtocolError> {
    let mut input = Reader::new(payload);
    let value = SyncResponse {
        session_id: input.read_session_id()?,
        correlation_id: input.read_u64()?,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
        t3_host_send_elapsed_ms: MonotonicMillis::new(input.read_u64()?),
    };
    input.finish()?;
    if value.t3_host_send_elapsed_ms < value.t2_host_receive_elapsed_ms {
        return Err(ProtocolError::InvalidField {
            field: "host_timestamp_order",
        });
    }
    Ok(value)
}

fn encode_audio(value: &AudioDatagram) -> Result<Vec<u8>, ProtocolError> {
    validate_audio_parameters(value)?;
    let mut output = Encoder::with_capacity(320 + value.payload.len());
    output.put_session_id(&value.session_id)?;
    output.put_stream_id(&value.stream_id)?;
    output.put_u64(value.sequence.get());
    output.put_u8(value.codec.code());
    output.put_u32(value.sample_rate);
    output.put_u16(value.channels);
    output.put_u32(value.samples_per_packet);
    output.put_u64(value.first_sample_index.get());
    output.put_u64(value.host_presentation_time_ms.get());
    let payload_length =
        u16::try_from(value.payload.len()).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: value.payload.len(),
            maximum: u16::MAX.into(),
        })?;
    output.put_u16(payload_length);
    output.put_u32(crc32(&value.payload));
    output.put_bytes(&value.payload);
    let encoded = output.finish();
    let maximum = MAX_AUDIO_DATAGRAM_BYTES - FRAME_HEADER_BYTES;
    if encoded.len() > maximum {
        return Err(ProtocolError::PayloadTooLarge {
            actual: encoded.len(),
            maximum,
        });
    }
    Ok(encoded)
}

fn decode_audio(payload: &[u8]) -> Result<AudioDatagram, ProtocolError> {
    let mut input = Reader::new(payload);
    let session_id = input.read_session_id()?;
    let stream_id = input.read_stream_id()?;
    let sequence = PacketSequence::new(input.read_u64()?);
    let codec_code = input.read_u8()?;
    let codec = AudioCodec::try_from(codec_code)
        .map_err(|codec| ProtocolError::UnsupportedAudioCodec { codec })?;
    let sample_rate = input.read_u32()?;
    let channels = input.read_u16()?;
    let samples_per_packet = input.read_u32()?;
    let first_sample_index = SampleIndex::new(input.read_u64()?);
    let host_presentation_time_ms = MonotonicMillis::new(input.read_u64()?);
    let declared_payload_length = usize::from(input.read_u16()?);
    let checksum = input.read_u32()?;
    if declared_payload_length > input.remaining() {
        return Err(ProtocolError::LengthMismatch {
            declared: declared_payload_length,
            actual: input.remaining(),
        });
    }
    if declared_payload_length < input.remaining() {
        return Err(ProtocolError::TrailingBytes);
    }
    let payload_bytes = input.take(declared_payload_length)?;
    if crc32(payload_bytes) != checksum {
        return Err(ProtocolError::IntegrityMismatch);
    }
    input.finish()?;
    let value = AudioDatagram {
        session_id,
        stream_id,
        sequence,
        codec,
        sample_rate,
        channels,
        samples_per_packet,
        first_sample_index,
        host_presentation_time_ms,
        payload: payload_bytes.to_vec(),
    };
    validate_audio_parameters(&value)?;
    Ok(value)
}

fn encode_payload(frame: &ProtocolFrame) -> Result<(Vec<u8>, u16), ProtocolError> {
    match frame {
        ProtocolFrame::Control(message) => encode_control(message).map(|payload| (payload, 0)),
        ProtocolFrame::SyncRequest(value) => encode_sync_request(value).map(|payload| (payload, 0)),
        ProtocolFrame::SyncResponse(value) => {
            encode_sync_response(value).map(|payload| (payload, 0))
        }
        ProtocolFrame::Audio(value) => {
            encode_audio(value).map(|payload| (payload, FLAG_PAYLOAD_INTEGRITY))
        }
    }
}

/// Encodes one canonical protocol-v2 frame.
///
/// # Errors
///
/// Returns [`ProtocolError`] when a field violates its bound, audio metadata
/// is inconsistent, or the encoded payload exceeds the message-kind limit.
pub fn encode_frame(frame: &ProtocolFrame) -> Result<Vec<u8>, ProtocolError> {
    let kind = frame.kind();
    let (payload, flags) = encode_payload(frame)?;
    let maximum = kind.maximum_payload_bytes();
    if payload.len() > maximum {
        return Err(ProtocolError::PayloadTooLarge {
            actual: payload.len(),
            maximum,
        });
    }
    let payload_length =
        u32::try_from(payload.len()).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: payload.len(),
            maximum,
        })?;
    let header = FrameHeader::new(kind, flags, payload_length);
    let mut output = Encoder::with_capacity(FRAME_HEADER_BYTES + payload.len());
    output.put_bytes(&PROTOCOL_MAGIC);
    output.put_u16(header.version);
    output.put_u16(header.kind.code());
    output.put_u16(header.flags);
    output.put_u16(header.header_length);
    output.put_u32(header.payload_length);
    output.put_bytes(&payload);
    Ok(output.finish())
}

/// Validates and decodes the fixed-width protocol-v2 header without allocating a payload.
///
/// # Errors
///
/// Returns [`ProtocolError`] for truncated input, invalid magic, unsupported
/// versions/kinds/flags, invalid header length, or an oversized declared payload.
pub fn decode_header(bytes: &[u8]) -> Result<FrameHeader, ProtocolError> {
    if bytes.len() < FRAME_HEADER_BYTES {
        return Err(ProtocolError::Truncated);
    }
    let mut input = Reader::new(&bytes[..FRAME_HEADER_BYTES]);
    if input.take(PROTOCOL_MAGIC.len())? != PROTOCOL_MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    let version = input.read_u16()?;
    if version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion { version });
    }
    let kind_code = input.read_u16()?;
    let kind = MessageKind::try_from(kind_code)
        .map_err(|kind| ProtocolError::UnsupportedMessageKind { kind })?;
    let flags = input.read_u16()?;
    if flags & !SUPPORTED_FRAME_FLAGS != 0 {
        return Err(ProtocolError::UnsupportedFlags { flags });
    }
    let expected_flags = if kind == MessageKind::Audio {
        FLAG_PAYLOAD_INTEGRITY
    } else {
        0
    };
    if flags != expected_flags {
        return Err(ProtocolError::UnsupportedFlags { flags });
    }
    let header_length = input.read_u16()?;
    if usize::from(header_length) != FRAME_HEADER_BYTES {
        return Err(ProtocolError::InvalidHeaderLength {
            length: header_length,
        });
    }
    let payload_length = input.read_u32()?;
    input.finish()?;
    let payload_length_usize =
        usize::try_from(payload_length).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: usize::MAX,
            maximum: kind.maximum_payload_bytes(),
        })?;
    let maximum = kind.maximum_payload_bytes();
    if payload_length_usize > maximum {
        return Err(ProtocolError::PayloadTooLarge {
            actual: payload_length_usize,
            maximum,
        });
    }
    Ok(FrameHeader {
        version,
        kind,
        flags,
        header_length,
        payload_length,
    })
}

/// Decodes exactly one complete canonical protocol-v2 frame.
///
/// # Errors
///
/// Returns [`ProtocolError`] when the frame header or payload is malformed,
/// unsupported, oversized, truncated, non-canonical, or fails integrity checks.
pub fn decode_frame(bytes: &[u8]) -> Result<ProtocolFrame, ProtocolError> {
    let header = decode_header(bytes)?;
    let payload_length =
        usize::try_from(header.payload_length).map_err(|_| ProtocolError::PayloadTooLarge {
            actual: usize::MAX,
            maximum: header.kind.maximum_payload_bytes(),
        })?;
    let expected_total =
        FRAME_HEADER_BYTES
            .checked_add(payload_length)
            .ok_or(ProtocolError::PayloadTooLarge {
                actual: usize::MAX,
                maximum: header.kind.maximum_payload_bytes(),
            })?;
    match bytes.len().cmp(&expected_total) {
        core::cmp::Ordering::Less => {
            return Err(ProtocolError::LengthMismatch {
                declared: payload_length,
                actual: bytes.len().saturating_sub(FRAME_HEADER_BYTES),
            });
        }
        core::cmp::Ordering::Greater => return Err(ProtocolError::TrailingBytes),
        core::cmp::Ordering::Equal => {}
    }
    let payload = &bytes[FRAME_HEADER_BYTES..expected_total];
    match header.kind {
        kind if kind.is_control() => decode_control(kind, payload).map(ProtocolFrame::Control),
        MessageKind::SyncRequest => decode_sync_request(payload).map(ProtocolFrame::SyncRequest),
        MessageKind::SyncResponse => decode_sync_response(payload).map(ProtocolFrame::SyncResponse),
        MessageKind::Audio => decode_audio(payload).map(ProtocolFrame::Audio),
        kind => Err(ProtocolError::UnsupportedMessageKind { kind: kind.code() }),
    }
}

#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::{
        DecodePolicy, ParseFailureClass, ProtocolDecoder, ProtocolError, crc32, decode_frame,
        decode_header, encode_frame,
    };
    use crate::{
        domain::{DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId},
        protocol::{
            AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, FRAME_HEADER_BYTES, Hello,
            JoinRequest, MAX_CONTROL_PAYLOAD_BYTES, MessageKind, PROTOCOL_MAGIC, PROTOCOL_VERSION,
            ProtocolFrame, SyncRequest, SyncResponse,
        },
    };

    fn session_id() -> SessionId {
        SessionId::new("session-1").expect("test session identifier")
    }

    fn stream_id() -> StreamId {
        StreamId::new("stream-1").expect("test stream identifier")
    }

    fn device_id() -> DeviceId {
        DeviceId::new("device-1").expect("test device identifier")
    }

    fn hello_frame() -> ProtocolFrame {
        ProtocolFrame::Control(ControlMessage::Hello(Hello {
            session_id: session_id(),
            session_name: "Test Session".into(),
            host_name: "Host Phone".into(),
            approval_required: true,
        }))
    }

    #[test]
    fn header_is_fixed_width_and_network_ordered() {
        let encoded = encode_frame(&hello_frame()).expect("hello frame encodes");
        assert_eq!(&encoded[..4], &PROTOCOL_MAGIC);
        assert_eq!(
            u16::from_be_bytes([encoded[4], encoded[5]]),
            PROTOCOL_VERSION
        );
        assert_eq!(
            u16::from_be_bytes([encoded[6], encoded[7]]),
            MessageKind::Hello.code()
        );
        assert_eq!(
            u16::from_be_bytes([encoded[10], encoded[11]]) as usize,
            FRAME_HEADER_BYTES
        );
        let header = decode_header(&encoded).expect("header decodes");
        assert_eq!(header.kind, MessageKind::Hello);
    }

    #[test]
    fn control_sync_and_audio_round_trip_canonically() {
        let frames = vec![
            hello_frame(),
            ProtocolFrame::Control(ControlMessage::JoinRequest(JoinRequest {
                session_id: session_id(),
                device: DeviceIdentity {
                    device_id: device_id(),
                    display_name: "Listener".into(),
                },
                invite_code: Some("123456".into()),
            })),
            ProtocolFrame::SyncRequest(SyncRequest {
                session_id: session_id(),
                correlation_id: 7,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(1_000),
            }),
            ProtocolFrame::SyncResponse(SyncResponse {
                session_id: session_id(),
                correlation_id: 7,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(1_000),
                t2_host_receive_elapsed_ms: MonotonicMillis::new(2_000),
                t3_host_send_elapsed_ms: MonotonicMillis::new(2_001),
            }),
            ProtocolFrame::Audio(AudioDatagram {
                session_id: session_id(),
                stream_id: stream_id(),
                sequence: PacketSequence::new(9),
                codec: AudioCodec::PcmS16Le,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 2,
                first_sample_index: SampleIndex::new(4),
                host_presentation_time_ms: MonotonicMillis::new(3_000),
                payload: vec![0, 1, 2, 3, 4, 5, 6, 7],
            }),
        ];

        for frame in frames {
            let encoded = encode_frame(&frame).expect("frame encodes");
            let decoded = decode_frame(&encoded).expect("frame decodes");
            assert!(decoded == frame);
            assert_eq!(encode_frame(&decoded), Ok(encoded));
        }
    }

    #[test]
    fn rejects_unknown_version_kind_flags_and_oversized_length_from_header_only() {
        let mut bytes = encode_frame(&hello_frame()).expect("hello frame encodes");
        bytes[4..6].copy_from_slice(&3_u16.to_be_bytes());
        assert!(matches!(
            decode_frame(&bytes),
            Err(ProtocolError::UnsupportedVersion { version: 3 })
        ));

        let mut bytes = encode_frame(&hello_frame()).expect("hello frame encodes");
        bytes[6..8].copy_from_slice(&999_u16.to_be_bytes());
        assert!(matches!(
            decode_frame(&bytes),
            Err(ProtocolError::UnsupportedMessageKind { kind: 999 })
        ));

        let mut bytes = encode_frame(&hello_frame()).expect("hello frame encodes");
        bytes[8..10].copy_from_slice(&0x8000_u16.to_be_bytes());
        assert!(matches!(
            decode_frame(&bytes),
            Err(ProtocolError::UnsupportedFlags { flags: 0x8000 })
        ));

        let mut header_only = [0_u8; FRAME_HEADER_BYTES];
        header_only[..4].copy_from_slice(&PROTOCOL_MAGIC);
        header_only[4..6].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        header_only[6..8].copy_from_slice(&MessageKind::Hello.code().to_be_bytes());
        header_only[10..12].copy_from_slice(&(FRAME_HEADER_BYTES as u16).to_be_bytes());
        header_only[12..16].copy_from_slice(
            &u32::try_from(MAX_CONTROL_PAYLOAD_BYTES + 1)
                .expect("test length")
                .to_be_bytes(),
        );
        assert!(matches!(
            decode_frame(&header_only),
            Err(ProtocolError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_truncation_trailing_bytes_and_integrity_failure() {
        let encoded = encode_frame(&hello_frame()).expect("hello frame encodes");
        assert!(matches!(
            decode_frame(&encoded[..encoded.len() - 1]),
            Err(ProtocolError::LengthMismatch { .. })
        ));
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(matches!(
            decode_frame(&trailing),
            Err(ProtocolError::TrailingBytes)
        ));

        let audio = ProtocolFrame::Audio(AudioDatagram {
            session_id: session_id(),
            stream_id: stream_id(),
            sequence: PacketSequence::new(1),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 1,
            samples_per_packet: 2,
            first_sample_index: SampleIndex::new(0),
            host_presentation_time_ms: MonotonicMillis::new(100),
            payload: vec![1, 2, 3, 4],
        });
        let mut encoded = encode_frame(&audio).expect("audio frame encodes");
        let last = encoded.len() - 1;
        encoded[last] ^= 0xff;
        assert!(matches!(
            decode_frame(&encoded),
            Err(ProtocolError::IntegrityMismatch)
        ));
    }

    #[test]
    fn decoder_policy_counts_each_failure_class() {
        let expected = session_id();
        let other = SessionId::new("session-other").expect("other session identifier");
        let unauthorized = ProtocolFrame::Control(ControlMessage::Hello(Hello {
            session_id: other,
            session_name: "Other Session".into(),
            host_name: "Other Host".into(),
            approval_required: false,
        }));
        let mut decoder = ProtocolDecoder::default();
        let unauthorized_bytes = encode_frame(&unauthorized).expect("unauthorized frame encodes");
        assert!(matches!(
            decoder.decode(
                &unauthorized_bytes,
                DecodePolicy {
                    expected_session_id: Some(&expected),
                    minimum_audio_sequence: None,
                },
            ),
            Err(ProtocolError::UnauthorizedSession)
        ));

        let stale = ProtocolFrame::Audio(AudioDatagram {
            session_id: expected.clone(),
            stream_id: stream_id(),
            sequence: PacketSequence::new(4),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 1,
            samples_per_packet: 1,
            first_sample_index: SampleIndex::new(0),
            host_presentation_time_ms: MonotonicMillis::new(0),
            payload: vec![0, 0],
        });
        let stale_bytes = encode_frame(&stale).expect("stale frame encodes");
        assert!(matches!(
            decoder.decode(
                &stale_bytes,
                DecodePolicy {
                    expected_session_id: Some(&expected),
                    minimum_audio_sequence: Some(PacketSequence::new(5)),
                },
            ),
            Err(ProtocolError::StaleAudioSequence)
        ));

        assert_eq!(decoder.counters().unauthorized, 1);
        assert_eq!(decoder.counters().stale, 1);
        assert_eq!(
            ProtocolError::StaleAudioSequence.classification(),
            ParseFailureClass::Stale
        );
    }

    #[test]
    fn crc32_matches_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn arbitrary_inputs_do_not_panic_or_allocate_from_untrusted_lengths() {
        let mut state = 0x1234_5678_9abc_def0_u64;
        for length in 0..=512 {
            let mut bytes = vec![0_u8; length];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state.to_le_bytes()[0];
            }
            let _ = decode_frame(&bytes);
        }
    }
}
