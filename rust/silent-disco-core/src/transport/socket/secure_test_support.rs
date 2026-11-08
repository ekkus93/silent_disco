//! Shared helpers for real-TLS control tests on a real host.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use p256::{
    ecdsa::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{ControlMessage, ProtocolFrame};
use crate::transport::secure_admission::dial_and_prove;
use crate::transport::secure_control_session::SecureControlSession;
use crate::transport::secure_control_tls::host_tls_identity_from_pkcs8;
use crate::transport::test_support::join_request;
use crate::transport::{
    HostTransportConfig, HostTransportNode, ManualTransportClock, TransportEvent,
};

use super::SocketHostTransport;

pub(super) fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("key")
}

pub(super) fn spki_of(seed: u8) -> Vec<u8> {
    key(seed)
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

/// A host with a pinned TLS identity; returns the host and its pinned SPKI.
pub(super) fn host_with_secure(seed: u8, session: &SessionId) -> (SocketHostTransport, Vec<u8>) {
    let pkcs8 = key(seed).to_pkcs8_der().expect("pkcs8").as_bytes().to_vec();
    let identity = Arc::new(host_tls_identity_from_pkcs8(&pkcs8).expect("identity"));
    let host_spki = identity.spki.clone();
    let mut config = HostTransportConfig::loopback(session.clone());
    config.secure_control = Some(identity);
    let host =
        SocketHostTransport::bind(config, Arc::new(ManualTransportClock::new(0))).expect("bind");
    (host, host_spki)
}

pub(super) fn control_address(host: &SocketHostTransport) -> SocketAddr {
    let endpoint = host.endpoint();
    SocketAddr::new(endpoint.address, endpoint.control_port)
}

/// Waits up to ten seconds for an event the predicate accepts.
pub(super) fn wait_for_event(
    host: &mut SocketHostTransport,
    mut wanted: impl FnMut(&TransportEvent) -> bool,
) -> TransportEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = host.recv_event(remaining).expect("host event");
        if wanted(&event) {
            return event;
        }
        assert!(Instant::now() < deadline, "expected event did not arrive");
    }
}

/// Dials the host over TLS, proves the listener key, and sends a `JoinRequest`
/// for `device`. The returned session must stay alive: dropping it closes the
/// connection.
pub(super) fn join_over_tls(
    host: &SocketHostTransport,
    host_spki: &[u8],
    session: &SessionId,
    device: &DeviceId,
    listener_seed: u8,
) -> SecureControlSession {
    let channel = dial_and_prove(
        control_address(host),
        host_spki,
        session,
        1,
        &key(listener_seed),
    )
    .expect("listener proved");
    let listener = SecureControlSession::new(channel, session.clone());
    listener
        .send_frame(&ProtocolFrame::Control(join_request(
            session,
            device,
            "TLS listener",
        )))
        .expect("join sent");
    listener
}

/// True for the join frame that the host forwards to its actor.
pub(super) fn is_join_frame(event: &TransportEvent) -> bool {
    matches!(
        event,
        TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            ..
        }
    )
}

/// The device ID a listener proving `key(seed)` is bound to. A join must claim
/// exactly this ID once its key is verified.
pub(super) fn listener_device(seed: u8) -> DeviceId {
    crate::transport::secure_identity::device_id_for_listener_key(&spki_of(seed))
}
