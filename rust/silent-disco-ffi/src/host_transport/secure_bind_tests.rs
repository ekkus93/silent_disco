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
