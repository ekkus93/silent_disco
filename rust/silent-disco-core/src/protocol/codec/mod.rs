mod decoding;
mod encoding;
#[cfg(test)]
mod tests;

pub use decoding::{decode_frame, decode_header};
pub use encoding::encode_frame;

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

#[derive(Debug)]
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
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
