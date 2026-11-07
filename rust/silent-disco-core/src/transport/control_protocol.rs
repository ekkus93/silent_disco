//! Tells a TLS control connection from a plaintext protocol v3 connection by its
//! first bytes, so one host port can serve both during the migration.
//!
//! A TLS 1.3 `ClientHello` starts with the record type `0x16` (handshake) and the
//! legacy version `0x03 0x01`. A plaintext v3 frame starts with the magic
//! `SDP2`. Anything else is refused.

use super::super::protocol::PROTOCOL_MAGIC;

/// Which protocol a connection speaks, decided from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlProtocol {
    /// Opens with a TLS handshake record.
    Tls,
    /// Opens with the plaintext v3 magic.
    Plaintext,
    /// Too few bytes to decide yet.
    NeedMore,
    /// Matches neither protocol; the connection is refused.
    Unknown,
}

/// Classifies the first bytes a peer has sent.
#[must_use]
pub fn classify_control_prefix(prefix: &[u8]) -> ControlProtocol {
    const TLS_PREFIX: [u8; 2] = [0x16, 0x03];
    if prefix.is_empty() {
        return ControlProtocol::NeedMore;
    }
    if prefix[0] == TLS_PREFIX[0] {
        if prefix.len() < 2 {
            return ControlProtocol::NeedMore;
        }
        return if prefix[1] == TLS_PREFIX[1] {
            ControlProtocol::Tls
        } else {
            ControlProtocol::Unknown
        };
    }
    let magic = PROTOCOL_MAGIC;
    let comparable = prefix.len().min(magic.len());
    if prefix[..comparable] != magic[..comparable] {
        return ControlProtocol::Unknown;
    }
    if prefix.len() >= magic.len() {
        ControlProtocol::Plaintext
    } else {
        ControlProtocol::NeedMore
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlProtocol, classify_control_prefix};

    #[test]
    fn a_tls_handshake_record_is_classified_as_tls() {
        assert_eq!(
            classify_control_prefix(&[0x16, 0x03, 0x01, 0x00, 0xf1]),
            ControlProtocol::Tls
        );
        assert_eq!(classify_control_prefix(&[0x16, 0x03]), ControlProtocol::Tls);
    }

    #[test]
    fn the_plaintext_magic_is_classified_as_plaintext() {
        assert_eq!(
            classify_control_prefix(b"SDP2\x00\x03"),
            ControlProtocol::Plaintext
        );
        assert_eq!(classify_control_prefix(b"SDP2"), ControlProtocol::Plaintext);
    }

    #[test]
    fn partial_prefixes_need_more_bytes_rather_than_guessing() {
        assert_eq!(classify_control_prefix(&[]), ControlProtocol::NeedMore);
        assert_eq!(classify_control_prefix(&[0x16]), ControlProtocol::NeedMore);
        assert_eq!(classify_control_prefix(b"SD"), ControlProtocol::NeedMore);
    }

    #[test]
    fn anything_else_is_refused() {
        assert_eq!(classify_control_prefix(b"GET /"), ControlProtocol::Unknown);
        assert_eq!(
            classify_control_prefix(&[0x16, 0x02]),
            ControlProtocol::Unknown
        );
        assert_eq!(
            classify_control_prefix(&[0x17, 0x03, 0x03]),
            ControlProtocol::Unknown
        );
        assert_eq!(classify_control_prefix(b"SDP3"), ControlProtocol::Unknown);
    }
}
