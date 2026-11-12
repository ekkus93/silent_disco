//! Protocol selection picks the matching transport and never falls back.

use std::sync::Arc;

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_transport_factory::{TransportProtocol, transport_factory_for};
use super::secure_control_tls::host_tls_identity_from_pkcs8;
use super::secure_identity::device_id_for_listener_key;
use super::secure_listener_proof::ListenerProofSigner;
use super::test_support::{id_session, wait_for_host_event};
use super::{
    HostTransportConfig, ListenerTransportConfig, SecureHostTarget, SystemTransportClock,
    TransportErrorKind, TransportEvent,
};

#[test]
fn the_default_protocol_is_legacy_v3() {
    assert_eq!(TransportProtocol::default(), TransportProtocol::LegacyV3);
}

#[test]
fn legacy_v3_binds_the_socket_host_without_an_identity() {
    let clock = Arc::new(SystemTransportClock::starting_at(0));
    let mut host = transport_factory_for(TransportProtocol::LegacyV3)
        .bind_host(
            HostTransportConfig::loopback(id_session("factory-v3")),
            clock,
        )
        .expect("v3 needs no identity");
    let endpoint = host.endpoint();
    assert_ne!(
        endpoint.sync_port, endpoint.control_port,
        "v3 uses separate UDP ports"
    );
    host.shutdown().expect("shutdown");
}

#[test]
fn secure_v4_refuses_to_run_without_an_identity_instead_of_falling_back() {
    let clock = Arc::new(SystemTransportClock::starting_at(0));
    let error = transport_factory_for(TransportProtocol::SecureV4)
        .bind_host(
            HostTransportConfig::loopback(id_session("factory-v4")),
            clock,
        )
        .map(|_| ())
        .expect_err("no silent v3 fallback");
    assert_eq!(error.kind, TransportErrorKind::InvalidConfiguration);
}

#[test]
fn secure_v4_host_and_listener_from_the_factory_connect_over_quic() {
    let session = id_session("factory-v4-pair");
    let host_key = SigningKey::from_slice(&[13; 32]).expect("host key");
    let identity = Arc::new(
        host_tls_identity_from_pkcs8(host_key.to_pkcs8_der().expect("pkcs8").as_bytes())
            .expect("identity"),
    );
    let factory = transport_factory_for(TransportProtocol::SecureV4);
    let mut host_config = HostTransportConfig::loopback(session.clone());
    host_config.secure_control = Some(Arc::clone(&identity));
    let clock = Arc::new(SystemTransportClock::starting_at(0));
    let mut host = factory
        .bind_host(host_config, clock.clone())
        .expect("v4 host");
    let endpoint = host.endpoint();
    assert_eq!(
        endpoint.sync_port, endpoint.control_port,
        "v4 uses one QUIC port"
    );

    let listener_key = SigningKey::from_slice(&[14; 32]).expect("listener key");
    let device = device_id_for_listener_key(&listener_key.spki_der().expect("spki"));
    let mut listener_config = ListenerTransportConfig::loopback(session, device, endpoint);
    listener_config.secure_host = Some(SecureHostTarget {
        host_spki: identity.spki.clone(),
        signer: Arc::new(listener_key),
    });
    let _listener = factory
        .connect_listener(listener_config, clock)
        .expect("v4 listener admitted");
    wait_for_host_event(host.as_mut(), |e| {
        matches!(e, TransportEvent::PeerAccepted { .. })
    });
}
