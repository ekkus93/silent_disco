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
#[cfg(feature = "legacy-v3")]
use crate::listener_transport::FfiListenerTransportError;

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

pub(super) fn signer(fail_spki: bool) -> Arc<dyn FfiListenerProofSigner> {
    Arc::new(PlatformSigner {
        key: SigningKey::from_slice(&[61_u8; 32]).expect("fixture key"),
        fail_spki,
    })
}

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
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
/// Three loopback ports that are free for both TCP (control) and UDP (sync and
/// audio). The host binds all three, so probing only TCP could collide with a
/// UDP port another test is using.
#[cfg(feature = "legacy-v3")]
fn free_ports() -> (u16, u16, u16) {
    loop {
        let control = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("probe control port")
            .local_addr()
            .expect("probe control address")
            .port();
        let sync = std::net::UdpSocket::bind("127.0.0.1:0")
            .expect("probe sync port")
            .local_addr()
            .expect("probe sync address")
            .port();
        let audio = std::net::UdpSocket::bind("127.0.0.1:0")
            .expect("probe audio port")
            .local_addr()
            .expect("probe audio address")
            .port();
        if control != sync && control != audio && sync != audio {
            // The probes are dropped before the host binds, so the ports can be reused.
            return (control, sync, audio);
        }
    }
}

pub(super) fn endpoint_payload(session: &str, control: u16, sync: u16, audio: u16) -> String {
    format!(
        r#"{{"hostAddress":"127.0.0.1","controlPort":{control},"syncPort":{sync},"audioPort":{audio},"sessionId":"{session}","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
        silent_disco_core::runtime::current_protocol_version()
    )
}

// A v3 host joined over TLS: only builds with `legacy-v3` may join v3.
#[cfg(feature = "legacy-v3")]
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
        listener_id(),
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
            requested = device_id == listener_id();
        }
    }
    assert!(
        requested,
        "the host must see the TLS listener's join request"
    );
    host.send_join_approval(listener_id(), false)
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

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
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
        listener_id(),
        "127.0.0.1".to_owned(),
        other_spki,
        signer(false),
    );
    assert!(outcome.is_err(), "a mismatched pin must not connect");
    host.shutdown().expect("host shutdown");
}

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
#[test]
fn a_pinned_listener_never_downgrades_to_plaintext_against_a_host_without_tls() {
    use crate::listener_transport::FfiListenerTransportHandle;

    let (control, sync, audio) = free_ports();
    // A host with no TLS identity: plaintext only.
    let host = FfiHostTransportHandle::bind_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        sync,
        audio,
        "downgrade-session".to_owned(),
        0,
    )
    .expect("plaintext host binds");
    let host_spki = SigningKey::from_slice(&[63_u8; 32])
        .expect("pin key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    let outcome = FfiListenerTransportHandle::connect_secure(
        endpoint_payload("downgrade-session", control, sync, audio),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        host_spki,
        signer(false),
    );
    assert!(
        outcome.is_err(),
        "a pinned listener must not fall back to plaintext"
    );
    host.shutdown().expect("host shutdown");
}

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
#[test]
fn a_malformed_pin_is_refused_before_any_connection() {
    use crate::listener_transport::FfiListenerTransportHandle;

    let (control, sync, audio) = free_ports();
    let host = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        sync,
        audio,
        "malformed-pin-session".to_owned(),
        0,
        signer(false),
    )
    .expect("secure host binds");
    let outcome = FfiListenerTransportHandle::connect_secure(
        endpoint_payload("malformed-pin-session", control, sync, audio),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        vec![1, 2, 3],
        signer(false),
    );
    assert!(
        outcome.is_err(),
        "a pin that is not a P-256 SPKI must not connect"
    );
    host.shutdown().expect("host shutdown");
}

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
#[test]
fn a_pinned_listener_for_another_session_is_never_approved_by_the_host() {
    use super::types::FfiHostTransportEvent;
    use crate::listener_transport::FfiListenerTransportHandle;
    use std::time::{Duration, Instant};

    let (control, sync, audio) = free_ports();
    let host = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        sync,
        audio,
        "host-session".to_owned(),
        0,
        signer(false),
    )
    .expect("secure host binds");
    let host_spki = SigningKey::from_slice(&[61_u8; 32])
        .expect("host key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    // The listener names a different session than the host runs.
    let listener = FfiListenerTransportHandle::connect_secure(
        endpoint_payload("other-session", control, sync, audio),
        0,
        listener_id(),
        "127.0.0.1".to_owned(),
        host_spki,
        signer(false),
    );
    // Refusal at connect is also a correct outcome; only an accepted connection can reach the join check.
    if let Ok(listener) = listener {
        let _ = listener.send_join_request("Mismatch".to_owned(), None);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_join = false;
        while Instant::now() < deadline {
            if let Ok(Some(FfiHostTransportEvent::JoinRequestReceived { device_id, .. })) =
                host.poll_event(100)
            {
                saw_join |= device_id == listener_id();
            }
        }
        assert!(
            !saw_join,
            "a join for another session must not reach the host actor"
        );
        listener.shutdown().expect("listener shutdown");
    }
    host.shutdown().expect("host shutdown");
}

/// The device ID the fixture listener key (seed 61) is bound to. TLS joins must
/// claim exactly this ID.
pub(super) fn listener_id() -> String {
    let spki = SigningKey::from_slice(&[61_u8; 32])
        .expect("listener key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    silent_disco_core::transport::secure_identity::device_id_for_listener_key(&spki)
        .as_str()
        .to_owned()
}

#[cfg(feature = "legacy-v3")] // binds a plaintext v3 host
#[test]
fn a_pinned_listener_cannot_claim_a_device_id_its_key_is_not_bound_to() {
    use crate::listener_transport::FfiListenerTransportHandle;

    let (control, sync, audio) = free_ports();
    let host = FfiHostTransportHandle::bind_secure_with_host_clock(
        "127.0.0.1".to_owned(),
        control,
        sync,
        audio,
        "claim-session".to_owned(),
        0,
        signer(false),
    )
    .expect("secure host binds");
    let host_spki = SigningKey::from_slice(&[61_u8; 32])
        .expect("host key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    let outcome = FfiListenerTransportHandle::connect_secure(
        endpoint_payload("claim-session", control, sync, audio),
        0,
        "someone-else".to_owned(),
        "127.0.0.1".to_owned(),
        host_spki,
        signer(false),
    );
    assert!(
        matches!(outcome, Err(FfiListenerTransportError::InvalidEndpoint(_))),
        "a pinned listener must claim its key-bound device ID"
    );
    host.shutdown().expect("host shutdown");
}
