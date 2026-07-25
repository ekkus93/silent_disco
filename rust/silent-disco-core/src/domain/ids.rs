use core::{fmt, str::FromStr};
use std::error::Error;

/// Maximum UTF-8 byte length accepted by every opaque domain identifier.
pub const MAX_IDENTIFIER_BYTES: usize = 128;

/// Identifies which domain identifier failed validation or decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdentifierKind {
    Session,
    Stream,
    Device,
    Request,
    Operation,
    DiagnosticRun,
}

impl IdentifierKind {
    /// Returns the stable, presentation-independent identifier kind name.
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Session => "session_id",
            Self::Stream => "stream_id",
            Self::Device => "device_id",
            Self::Request => "request_id",
            Self::Operation => "operation_id",
            Self::DiagnosticRun => "diagnostic_run_id",
        }
    }
}

impl fmt::Display for IdentifierKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_name())
    }
}

/// Stable reasons an opaque identifier can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdValidationReason {
    Empty,
    TooLong {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    SurroundingWhitespace,
    ControlCharacter {
        byte_index: usize,
    },
}

impl fmt::Display for IdValidationReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the identifier is blank"),
            Self::TooLong {
                actual_bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "the identifier is {actual_bytes} bytes; the maximum is {maximum_bytes} bytes"
            ),
            Self::SurroundingWhitespace => {
                formatter.write_str("leading or trailing whitespace is not allowed")
            }
            Self::ControlCharacter { byte_index } => write!(
                formatter,
                "a control character starts at UTF-8 byte index {byte_index}"
            ),
        }
    }
}

/// Validation failure that never includes the rejected identifier value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdValidationError {
    identifier_kind: IdentifierKind,
    reason: IdValidationReason,
}

impl IdValidationError {
    #[must_use]
    pub const fn identifier_kind(&self) -> IdentifierKind {
        self.identifier_kind
    }

    #[must_use]
    pub const fn reason(&self) -> &IdValidationReason {
        &self.reason
    }
}

impl fmt::Display for IdValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {}: {}",
            self.identifier_kind, self.reason
        )
    }
}

impl Error for IdValidationError {}

/// Canonical-byte decoding failure for an opaque domain identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdDecodeError {
    InvalidUtf8 {
        identifier_kind: IdentifierKind,
    },
    InvalidIdentifier(IdValidationError),
}

impl fmt::Display for IdDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 { identifier_kind } => {
                write!(formatter, "{identifier_kind} is not valid UTF-8")
            }
            Self::InvalidIdentifier(error) => error.fmt(formatter),
        }
    }
}

impl Error for IdDecodeError {}

impl From<IdValidationError> for IdDecodeError {
    fn from(error: IdValidationError) -> Self {
        Self::InvalidIdentifier(error)
    }
}

fn validate_identifier(
    identifier_kind: IdentifierKind,
    value: &str,
) -> Result<(), IdValidationError> {
    if value.trim().is_empty() {
        return Err(IdValidationError {
            identifier_kind,
            reason: IdValidationReason::Empty,
        });
    }

    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(IdValidationError {
            identifier_kind,
            reason: IdValidationReason::TooLong {
                actual_bytes: value.len(),
                maximum_bytes: MAX_IDENTIFIER_BYTES,
            },
        });
    }

    if value.trim() != value {
        return Err(IdValidationError {
            identifier_kind,
            reason: IdValidationReason::SurroundingWhitespace,
        });
    }

    if let Some((byte_index, _)) = value.char_indices().find(|(_, character)| character.is_control()) {
        return Err(IdValidationError {
            identifier_kind,
            reason: IdValidationReason::ControlCharacter { byte_index },
        });
    }

    Ok(())
}

macro_rules! define_string_identifier {
    ($name:ident, $kind:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Constructs a validated identifier.
            ///
            /// # Errors
            ///
            /// Returns [`IdValidationError`] when the identifier is blank, too
            /// long, surrounded by whitespace, or contains a control character.
            pub fn new(value: impl Into<String>) -> Result<Self, IdValidationError> {
                let value = value.into();
                validate_identifier($kind, &value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }

            /// Returns the deterministic canonical UTF-8 representation.
            #[must_use]
            pub fn to_canonical_bytes(&self) -> Vec<u8> {
                self.0.as_bytes().to_vec()
            }

            /// Decodes and validates the deterministic canonical UTF-8 representation.
            ///
            /// # Errors
            ///
            /// Returns [`IdDecodeError`] for invalid UTF-8 or any identifier
            /// value that fails normal construction validation.
            pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, IdDecodeError> {
                let value = core::str::from_utf8(bytes).map_err(|_| IdDecodeError::InvalidUtf8 {
                    identifier_kind: $kind,
                })?;
                Self::new(value).map_err(IdDecodeError::from)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $name {
            type Err = IdValidationError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdValidationError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdValidationError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

define_string_identifier!(SessionId, IdentifierKind::Session);
define_string_identifier!(StreamId, IdentifierKind::Stream);
define_string_identifier!(DeviceId, IdentifierKind::Device);
define_string_identifier!(RequestId, IdentifierKind::Request);
define_string_identifier!(OperationId, IdentifierKind::Operation);
define_string_identifier!(DiagnosticRunId, IdentifierKind::DiagnosticRun);

macro_rules! define_u64_identifier {
    ($name:ident) => {
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }

            /// Returns the deterministic network-byte-order representation.
            #[must_use]
            pub const fn to_be_bytes(self) -> [u8; 8] {
                self.0.to_be_bytes()
            }

            /// Decodes a deterministic network-byte-order representation.
            #[must_use]
            pub const fn from_be_bytes(bytes: [u8; 8]) -> Self {
                Self(u64::from_be_bytes(bytes))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.get()
            }
        }
    };
}

define_u64_identifier!(MonotonicMillis);
define_u64_identifier!(PacketSequence);
define_u64_identifier!(SampleIndex);

#[cfg(test)]
mod tests {
    use super::{
        DeviceId, DiagnosticRunId, IdDecodeError, IdValidationReason, MonotonicMillis,
        OperationId, PacketSequence, RequestId, SampleIndex, SessionId, StreamId,
        MAX_IDENTIFIER_BYTES,
    };

    #[test]
    fn every_string_identifier_accepts_a_normal_value() {
        assert_eq!(SessionId::new("session-1").map(|id| id.to_string()), Ok("session-1".into()));
        assert_eq!(StreamId::new("stream-1").map(|id| id.to_string()), Ok("stream-1".into()));
        assert_eq!(DeviceId::new("device-1").map(|id| id.to_string()), Ok("device-1".into()));
        assert_eq!(RequestId::new("request-1").map(|id| id.to_string()), Ok("request-1".into()));
        assert_eq!(OperationId::new("operation-1").map(|id| id.to_string()), Ok("operation-1".into()));
        assert_eq!(
            DiagnosticRunId::new("diagnostic-1").map(|id| id.to_string()),
            Ok("diagnostic-1".into())
        );
    }

    #[test]
    fn rejects_blank_and_boundary_whitespace_without_echoing_values() {
        let blank = SessionId::new("   ").expect_err("whitespace-only identifier must fail");
        assert_eq!(blank.reason(), &IdValidationReason::Empty);
        assert!(!blank.to_string().contains("   "));

        let surrounded = SessionId::new(" session-1 ")
            .expect_err("surrounding whitespace must fail");
        assert_eq!(surrounded.reason(), &IdValidationReason::SurroundingWhitespace);
        assert!(!surrounded.to_string().contains("session-1"));
    }

    #[test]
    fn enforces_utf8_byte_limit_and_control_character_rule() {
        let maximum = "a".repeat(MAX_IDENTIFIER_BYTES);
        assert!(SessionId::new(maximum).is_ok());

        let oversized = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        let error = SessionId::new(oversized).expect_err("oversized identifier must fail");
        assert_eq!(
            error.reason(),
            &IdValidationReason::TooLong {
                actual_bytes: MAX_IDENTIFIER_BYTES + 1,
                maximum_bytes: MAX_IDENTIFIER_BYTES,
            }
        );

        let control = SessionId::new("session\n1").expect_err("control character must fail");
        assert_eq!(
            control.reason(),
            &IdValidationReason::ControlCharacter { byte_index: 7 }
        );
    }

    #[test]
    fn canonical_bytes_round_trip_without_normalization() {
        let original = DeviceId::new("device-é-1").expect("valid device identifier");
        let encoded = original.to_canonical_bytes();
        let decoded = DeviceId::from_canonical_bytes(&encoded);
        assert_eq!(decoded, Ok(original));
    }

    #[test]
    fn canonical_decoder_rejects_invalid_utf8_and_invalid_values() {
        assert_eq!(
            SessionId::from_canonical_bytes(&[0xff]),
            Err(IdDecodeError::InvalidUtf8 {
                identifier_kind: super::IdentifierKind::Session,
            })
        );
        assert!(matches!(
            SessionId::from_canonical_bytes(b""),
            Err(IdDecodeError::InvalidIdentifier(_))
        ));
    }

    #[test]
    fn numeric_identifiers_use_deterministic_network_byte_order() {
        let monotonic = MonotonicMillis::new(0x0102_0304_0506_0708);
        assert_eq!(
            monotonic.to_be_bytes(),
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
        assert_eq!(
            MonotonicMillis::from_be_bytes(monotonic.to_be_bytes()),
            monotonic
        );
        assert_eq!(PacketSequence::new(42).to_string(), "42");
        assert_eq!(SampleIndex::new(84).get(), 84);
    }
}
