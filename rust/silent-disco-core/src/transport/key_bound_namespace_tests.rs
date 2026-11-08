//! A plaintext listener may not claim a device ID in the key-bound namespace.
//! Only a TLS-proven key can, so such a join is refused before the actor.

use std::sync::Arc;
use std::time::{Duration, Instant};

use p256::{ecdsa::SigningKey, pkcs8::EncodePublicKey};

use crate::domain::DeviceId;
use crate::protocol::ControlMessage;
use crate::transport::secure_identity::device_id_for_listener_key;

use super::test_support::{id_session, join_request};
use super::{
    HostTransportConfig, ListenerTransportConfig, SocketTransportFactory, SystemTransportClock,
    TransportChannel, TransportErrorKind, TransportEvent, TransportFactory,
};

fn key_bound_device() -> DeviceId {
    let der = SigningKey::from_slice(&[83_u8; 32])
        .expect("fixture key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    device_id_for_listener_key(&der)
}

#[test]
fn a_plaintext_listener_claiming_a_key_bound_device_id_is_refused_before_the_actor() {
    let session_id = id_session("namespace-session");
    let device_id = key_bound_device();
    let factory = SocketTransportFactory;
    let clock = Arc::new(SystemTransportClock::default());
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host binds");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("plaintext listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Impostor"))
        .expect("join sent");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut refused = false;
    while Instant::now() < deadline && !refused {
        if let Ok(TransportEvent::Rejected {
            channel: TransportChannel::Control,
            error,
            ..
        }) = host.recv_event(Duration::from_millis(100))
        {
            assert_eq!(
                error.kind,
                TransportErrorKind::Unauthorized,
                "{}",
                error.message
            );
            refused = true;
        }
    }
    assert!(refused, "the plaintext claim must be refused");

    // No join for that ID may reach the host actor.
    let quiet_until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < quiet_until {
        if let Ok(TransportEvent::FrameReceived { frame, .. }) =
            host.recv_event(Duration::from_millis(50))
        {
            assert!(
                !matches!(
                    frame,
                    crate::protocol::ProtocolFrame::Control(ControlMessage::JoinRequest(_))
                ),
                "a key-bound claim from plaintext must not reach the actor"
            );
        }
    }
    let _ = listener.shutdown();
    host.shutdown().expect("host shuts down");
}
