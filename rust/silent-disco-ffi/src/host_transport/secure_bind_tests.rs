//! The secure host bind takes its TLS identity from a platform signer and must
//! fail closed when that signer cannot supply its key or certificate.

use std::sync::Arc;

use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer as _},
    pkcs8::EncodePublicKey,
};

use super::FfiHostTransportHandle;
use super::types::FfiHostTransportError;
use crate::host_control::FfiBridgeError;
use crate::listener_proof::{FfiListenerProofSigner, FfiProofHostListener, listener_prove_to_host};

/// A platform signer that holds a key, or fails on request.
struct PlatformSigner {
    key: SigningKey,
    fail_spki: bool,
}

impl FfiListenerProofSigner for PlatformSigner {
    fn spki_der(&self) -> Result<Vec<u8>, FfiBridgeError> {
        if self.fail_spki {
            return Err(FfiBridgeError::Callback("key unavailable".to_owned()));
        }
        self.key
            .verifying_key()
            .to_public_key_der()
            .map(|der| der.as_bytes().to_vec())
            .map_err(|_| FfiBridgeError::Callback("spki encoding failed".to_owned()))
    }

    fn sign_der(&self, transcript: Vec<u8>) -> Result<Vec<u8>, FfiBridgeError> {
        let signature: Signature = self.key.sign(&transcript);
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

fn signer(fail_spki: bool) -> Arc<dyn FfiListenerProofSigner> {
    Arc::new(PlatformSigner {
        key: SigningKey::from_slice(&[61_u8; 32]).expect("fixture key"),
        fail_spki,
    })
}

#[test]
fn a_secure_host_binds_when_the_platform_key_is_available() {
    let handle = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        0,
        0,
        0,
        "secure-bind-session".to_owned(),
        0,
        signer(false),
    )
    .expect("secure host binds");
    handle.shutdown().expect("shutdown");
}

#[test]
fn a_secure_host_refuses_to_bind_when_the_platform_cannot_supply_its_key() {
    let outcome = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        0,
        0,
        0,
        "secure-bind-refused".to_owned(),
        0,
        signer(true),
    );
    assert!(matches!(
        outcome,
        Err(FfiHostTransportError::InvalidConfiguration(_))
    ));
}

#[test]
fn a_host_whose_key_stays_on_the_platform_is_verified_by_a_pinned_listener_over_tls() {
    let host =
        FfiProofHostListener::bind_with_platform_key("127.0.0.1:0".to_owned(), signer(false))
            .expect("platform-key host binds");
    let port = host.local_port().expect("port");
    let spki = host.host_spki();
    let session = "platform-proof-session".to_owned();
    let accepting_host = Arc::clone(&host);
    let accepting_session = session.clone();
    let accepting =
        std::thread::spawn(move || accepting_host.accept_and_verify(accepting_session, 1));
    listener_prove_to_host(format!("127.0.0.1:{port}"), spki, session, 1, signer(false))
        .expect("pinned listener completes the TLS handshake and proof");
    let verified = accepting
        .join()
        .expect("accept thread")
        .expect("host verifies the listener proof");
    assert_eq!(verified.listener_fingerprint.len(), 32);
}

/// Two free loopback ports, found by binding and releasing them.
fn free_ports() -> (u16, u16, u16) {
    let ports: Vec<u16> = (0..3)
        .map(|_| {
            std::net::TcpListener::bind("127.0.0.1:0")
                .expect("probe port")
                .local_addr()
                .expect("probe address")
                .port()
        })
        .collect();
    (ports[0], ports[1], ports[2])
}

fn endpoint_payload(session: &str, control: u16, sync: u16, audio: u16) -> String {
    format!(
        r#"{{"hostAddress":"127.0.0.1","controlPort":{control},"syncPort":{sync},"audioPort":{audio},"sessionId":"{session}","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
        silent_disco_core::runtime::current_protocol_version()
    )
}

#[test]
fn a_pinned_listener_joins_a_platform_key_host_over_tls_and_is_approved_on_the_same_channel() {
    use super::types::FfiHostTransportEvent;
    use crate::listener_transport::FfiListenerTransportEvent;
    use crate::listener_transport::FfiListenerTransportHandle;
    use std::time::{Duration, Instant};

    let session = "tls-join-session";
    let (control, sync, audio) = free_ports();
    let host = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        sync,
        audio,
        session.to_owned(),
        0,
        signer(false),
    )
    .expect("secure host binds");
    let host_spki = {
        let key = SigningKey::from_slice(&[61_u8; 32]).expect("fixture key");
        key.verifying_key()
            .to_public_key_der()
            .expect("spki")
            .as_bytes()
            .to_vec()
    };
    let listener = FfiListenerTransportHandle::connect_secure(
        endpoint_payload(session, control, sync, audio),
        0,
        "tls-listener-device".to_owned(),
        "127.0.0.1".to_owned(),
        host_spki,
        signer(false),
    )
    .expect("pinned listener connects over TLS");
    listener
        .send_join_request("Pinned listener".to_owned(), None)
        .expect("join request");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut requested = false;
    while !requested && Instant::now() < deadline {
        if let Some(FfiHostTransportEvent::JoinRequestReceived { device_id, .. }) =
            host.poll_event(200).expect("host event")
        {
            requested = device_id == "tls-listener-device";
        }
    }
    assert!(
        requested,
        "the host must see the TLS listener's join request"
    );
    host.send_join_approval("tls-listener-device".to_owned(), false)
        .expect("approval");

    let mut approved = false;
    while !approved && Instant::now() < deadline {
        if let Some(FfiListenerTransportEvent::JoinApproved { .. }) =
            listener.poll_event(200).expect("listener event")
        {
            approved = true;
        }
    }
    assert!(
        approved,
        "the approval must return over the same TLS channel"
    );
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn a_listener_pinned_to_another_key_never_connects_to_a_platform_key_host() {
    use crate::listener_transport::FfiListenerTransportHandle;

    let (control, sync, audio) = free_ports();
    let host = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        sync,
        audio,
        "tls-pin-session".to_owned(),
        0,
        signer(false),
    )
    .expect("secure host binds");
    let other_key = SigningKey::from_slice(&[62_u8; 32]).expect("other key");
    let other_spki = other_key
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    let outcome = FfiListenerTransportHandle::connect_secure(
        endpoint_payload("tls-pin-session", control, sync, audio),
        0,
        "pin-listener-device".to_owned(),
        "127.0.0.1".to_owned(),
        other_spki,
        signer(false),
    );
    assert!(outcome.is_err(), "a mismatched pin must not connect");
    host.shutdown().expect("host shutdown");
}
