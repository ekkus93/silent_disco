//! Control-connection admission over real TLS 1.3 on loopback TCP.

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use p256::{
    ecdsa::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};
use sha2::{Digest, Sha256};

use crate::domain::SessionId;

use super::secure_admission::{admit_listener, dial_and_prove};
use super::secure_control_tls::host_tls_identity_from_pkcs8;
use super::secure_listener_proof::{ListenerProofSigner, ProofExchangeError};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("key")
}

fn spki(seed: u8) -> Vec<u8> {
    key(seed)
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

fn host_pkcs8(seed: u8) -> Vec<u8> {
    key(seed).to_pkcs8_der().expect("pkcs8").as_bytes().to_vec()
}

/// Claims one key's SPKI and signs with another: the forgery under test.
struct ClaimingSigner {
    claimed: Vec<u8>,
    actual: SigningKey,
}

impl ListenerProofSigner for ClaimingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        Ok(self.claimed.clone())
    }
    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        self.actual.sign_der(transcript)
    }
}

/// Holds the proof until the host has given up, so admission must time out.
struct StalledSigner {
    key: SigningKey,
    stall: Duration,
}

impl ListenerProofSigner for StalledSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        self.key.spki_der()
    }
    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        thread::sleep(self.stall);
        self.key.sign_der(transcript)
    }
}

#[test]
fn an_honest_listener_is_admitted_and_the_channel_stays_usable() {
    let identity = host_tls_identity_from_pkcs8(&host_pkcs8(31)).expect("host");
    let session = SessionId::new("admission-session").expect("session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");
    let listener_key = key(32);
    let expected_fingerprint: [u8; 32] = Sha256::digest(spki(32)).into();

    let client_session = session.clone();
    let host_spki = identity.spki.clone();
    let client = thread::spawn(move || {
        dial_and_prove(address, &host_spki, &client_session, 1, &listener_key)
    });

    let (tcp, _) = server.accept().expect("accept");
    let admitted = admit_listener(tcp, &identity, &session, 1).expect("admitted");
    let listener_channel = client
        .join()
        .expect("client thread")
        .expect("listener proved");

    assert_eq!(admitted.verified.fingerprint(), &expected_fingerprint);
    // The channel carries join traffic after admission.
    admitted
        .channel
        .send(b"approved".to_vec())
        .expect("host sends");
    assert_eq!(
        listener_channel
            .recv_timeout(Duration::from_secs(10))
            .expect("listener receives"),
        b"approved"
    );
    listener_channel.close();
    admitted.channel.close();
}

#[test]
fn a_listener_claiming_another_key_is_refused_before_any_channel_is_returned() {
    let identity = host_tls_identity_from_pkcs8(&host_pkcs8(33)).expect("host");
    let session = SessionId::new("forged-admission").expect("session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");
    let signer = ClaimingSigner {
        claimed: spki(34),
        actual: key(35),
    };

    let client_session = session.clone();
    let host_spki = identity.spki.clone();
    let client =
        thread::spawn(move || dial_and_prove(address, &host_spki, &client_session, 1, &signer));

    let (tcp, _) = server.accept().expect("accept");
    let outcome = admit_listener(tcp, &identity, &session, 1);
    let _ = client.join();
    assert!(
        matches!(outcome, Err(ProofExchangeError::Proof(_))),
        "a forged listener key was admitted"
    );
}

#[test]
fn a_listener_pinned_to_another_host_never_reaches_admission() {
    let identity = host_tls_identity_from_pkcs8(&host_pkcs8(36)).expect("host");
    let session = SessionId::new("pin-admission").expect("session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client_session = session.clone();
    let wrong_pin = spki(37);
    let client =
        thread::spawn(move || dial_and_prove(address, &wrong_pin, &client_session, 1, &key(38)));

    let (tcp, _) = server.accept().expect("accept");
    let host_outcome = admit_listener(tcp, &identity, &session, 1);
    let client_outcome = client.join().expect("client thread");
    assert!(
        client_outcome.is_err(),
        "a listener pinned elsewhere completed the proof"
    );
    assert!(
        host_outcome.is_err(),
        "the host admitted a listener that never completed TLS"
    );
}

#[test]
fn a_listener_that_never_answers_the_challenge_is_dropped_on_a_bounded_deadline() {
    let identity = host_tls_identity_from_pkcs8(&host_pkcs8(39)).expect("host");
    let session = SessionId::new("stalled-admission").expect("session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client_session = session.clone();
    let host_spki = identity.spki.clone();
    let signer = StalledSigner {
        key: key(40),
        stall: Duration::from_mins(1),
    };
    let client =
        thread::spawn(move || dial_and_prove(address, &host_spki, &client_session, 1, &signer));

    let started = std::time::Instant::now();
    let (tcp, _) = server.accept().expect("accept");
    let outcome = admit_listener(tcp, &identity, &session, 1);
    assert!(outcome.is_err(), "a silent listener must not be admitted");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "admission must give up within its deadline, not wait for the listener"
    );
    // The client thread stays blocked in the stalled signer; detach it.
    drop(client);
}
