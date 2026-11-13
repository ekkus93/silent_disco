use silent_disco_core::transport::TransportProtocol;

use super::check_host_protocol;
use crate::host_transport::FfiHostTransportHandle;
use crate::host_transport::types::FfiHostTransportError;
use crate::legacy_v3::LegacyV3Policy;

/// Phase 4: a production build never binds a plaintext v3 host.
#[test]
fn a_production_build_refuses_to_host_v3_and_allows_v4() {
    let refused = check_host_protocol(TransportProtocol::LegacyV3, LegacyV3Policy::Forbidden);
    assert!(matches!(
        refused,
        Err(FfiHostTransportError::InvalidConfiguration(message)) if message.contains("secure transport (v4)")
    ));
    assert!(check_host_protocol(TransportProtocol::SecureV4, LegacyV3Policy::Forbidden).is_ok());
    assert!(check_host_protocol(TransportProtocol::LegacyV3, LegacyV3Policy::Allowed).is_ok());
}

/// The exported v3 constructors follow the build: refused without
/// `legacy-v3`, bound with it.
#[test]
fn the_exported_v3_bind_follows_the_build_policy() {
    let result = FfiHostTransportHandle::bind_with_host_clock(
        "127.0.0.1".to_owned(),
        0,
        0,
        0,
        "session-policy".to_owned(),
        0,
    );
    if cfg!(feature = "legacy-v3") {
        let handle = result.expect("v3 binds with legacy-v3");
        handle.shutdown().expect("shutdown");
    } else {
        assert!(matches!(
            result,
            Err(FfiHostTransportError::InvalidConfiguration(_))
        ));
    }
}
