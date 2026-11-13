//! Which transport a listener constructor may open, decided from what it
//! asked for, what the host advertises and whether this build allows
//! plaintext v3, before any connection is attempted.

use silent_disco_core::transport::TransportProtocol;

use super::types::FfiListenerTransportError;
pub(crate) use crate::legacy_v3::LegacyV3Policy;

/// Which protocol a listener constructor asked for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Requested {
    /// Plain v3. A v4 host cannot be joined this way.
    PlainV3,
    /// Pinned to the invitation key: QUIC for a v4 host, TLS for a v3 host.
    PinnedAsAdvertised,
    /// QUIC only.
    SecureV4,
}

impl Requested {
    pub(crate) fn resolve(
        self,
        host_is_v4: bool,
        legacy: LegacyV3Policy,
    ) -> Result<TransportProtocol, FfiListenerTransportError> {
        let protocol = match (self, host_is_v4) {
            (Self::PlainV3, true) => {
                return Err(FfiListenerTransportError::InvalidEndpoint(
                    "this host uses secure transport (v4); join it from the host's signed invitation"
                        .to_owned(),
                ));
            }
            (Self::SecureV4, false) => {
                return Err(FfiListenerTransportError::InvalidEndpoint(
                    "this host does not use secure transport (v4)".to_owned(),
                ));
            }
            (Self::PlainV3 | Self::PinnedAsAdvertised, false) => TransportProtocol::LegacyV3,
            (Self::PinnedAsAdvertised | Self::SecureV4, true) => TransportProtocol::SecureV4,
        };
        if protocol == TransportProtocol::LegacyV3 && legacy == LegacyV3Policy::Forbidden {
            return Err(FfiListenerTransportError::UnsupportedProtocolVersion(
                "this host uses the older unencrypted protocol (v3), which this app no longer \
                 joins; turn on secure transport (v4) on the host and scan its QR code"
                    .to_owned(),
            ));
        }
        Ok(protocol)
    }
}

#[cfg(test)]
#[path = "protocol_policy_tests.rs"]
mod tests;
