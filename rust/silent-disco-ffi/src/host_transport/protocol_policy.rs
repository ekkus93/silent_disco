//! Whether this build may bind a plaintext v3 host (phase 4).

use silent_disco_core::transport::TransportProtocol;

use super::types::FfiHostTransportError;
use crate::legacy_v3::LegacyV3Policy;

/// Refuses a plaintext v3 host in a build without `legacy-v3` (phase 4),
/// before any socket is bound.
pub(super) fn check_host_protocol(
    protocol: TransportProtocol,
    legacy: LegacyV3Policy,
) -> Result<(), FfiHostTransportError> {
    if protocol == TransportProtocol::LegacyV3 && legacy == LegacyV3Policy::Forbidden {
        return Err(FfiHostTransportError::InvalidConfiguration(
            "plaintext v3 hosting is not available in this build; host with secure transport (v4)"
                .to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "host_protocol_policy_tests.rs"]
mod tests;
