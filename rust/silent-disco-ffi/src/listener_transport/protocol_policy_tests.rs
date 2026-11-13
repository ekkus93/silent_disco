use silent_disco_core::transport::TransportProtocol;

use super::{LegacyV3Policy, Requested};
use crate::listener_transport::types::FfiListenerTransportError;

const ALLOWED: LegacyV3Policy = LegacyV3Policy::Allowed;
const FORBIDDEN: LegacyV3Policy = LegacyV3Policy::Forbidden;

fn refused_as_v3(result: Result<TransportProtocol, FfiListenerTransportError>) -> bool {
    matches!(
        result,
        Err(FfiListenerTransportError::UnsupportedProtocolVersion(message))
            if message.contains("(v3)") && message.contains("scan its QR code")
    )
}

/// Phase 4: a production build never opens v3, whether the caller had a QR
/// (pinned) or not (discovery, typed address).
#[test]
fn a_production_build_refuses_every_v3_host() {
    assert!(refused_as_v3(Requested::PlainV3.resolve(false, FORBIDDEN)));
    assert!(refused_as_v3(
        Requested::PinnedAsAdvertised.resolve(false, FORBIDDEN)
    ));
}

/// Without the host's QR there is no key to pin, so a v4 host found by
/// discovery or typed by hand is still sent to the invitation.
#[test]
fn a_production_build_sends_an_unpinned_v4_join_to_the_signed_invitation() {
    assert!(matches!(
        Requested::PlainV3.resolve(true, FORBIDDEN),
        Err(FfiListenerTransportError::InvalidEndpoint(message)) if message.contains("signed invitation")
    ));
}

#[test]
fn a_production_build_joins_a_v4_host_from_its_qr() {
    assert_eq!(
        Requested::PinnedAsAdvertised.resolve(true, FORBIDDEN).ok(),
        Some(TransportProtocol::SecureV4)
    );
    assert_eq!(
        Requested::SecureV4.resolve(true, FORBIDDEN).ok(),
        Some(TransportProtocol::SecureV4)
    );
}

/// Tests and lab tooling (the `legacy-v3` feature) keep the old rules.
#[test]
fn with_legacy_v3_allowed_the_old_rules_hold() {
    assert_eq!(
        Requested::PlainV3.resolve(false, ALLOWED).ok(),
        Some(TransportProtocol::LegacyV3)
    );
    assert_eq!(
        Requested::PinnedAsAdvertised.resolve(false, ALLOWED).ok(),
        Some(TransportProtocol::LegacyV3)
    );
    assert!(Requested::SecureV4.resolve(false, ALLOWED).is_err());
}

#[test]
fn the_build_policy_follows_the_legacy_v3_feature() {
    assert_eq!(
        LegacyV3Policy::BUILD == ALLOWED,
        cfg!(feature = "legacy-v3")
    );
}
