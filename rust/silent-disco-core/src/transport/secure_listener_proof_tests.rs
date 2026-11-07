//! Listener possession proof over real TLS 1.3 connections on loopback TCP.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use p256::{
    ecdsa::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::{
    ClientConnection, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
    sign::SigningKey as RustlsSigningKey,
};

use super::secure_control_tls::{HOST_SERVER_NAME, pinned_client_config};
use super::secure_identity::ListenerProofError;
use super::secure_listener_proof::{
    ListenerProofSigner, ProofExchangeError, accept_and_verify, build_listener_response,
    client_binding, connect_and_prove, host_verify_listener, listener_spki, server_binding,
};
use crate::domain::SessionId;

const TIMEOUT: Duration = Duration::from_secs(10);

struct Host {
    spki: Vec<u8>,
    certificate: CertificateDer<'static>,
    signing_key: Arc<dyn RustlsSigningKey>,
}

fn host_identity(seed: u8) -> Host {
    let signing = SigningKey::from_slice(&[seed; 32]).expect("fixture key");
    let spki = signing
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    let pkcs8 = signing.to_pkcs8_der().expect("pkcs8").as_bytes().to_vec();
    let key_pair = KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(pkcs8.as_slice()),
        &PKCS_ECDSA_P256_SHA256,
    )
    .expect("rcgen key");
    let certificate = CertificateParams::new(vec![HOST_SERVER_NAME.to_owned()])
        .expect("params")
        .self_signed(&key_pair)
        .expect("certificate")
        .der()
        .clone();
    let signing_key = rustls::crypto::ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
        PrivatePkcs8KeyDer::from(pkcs8),
    ))
    .expect("signing key");
    Host {
        spki,
        certificate,
        signing_key,
    }
}

fn listener_key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("listener key")
}

fn session(name: &str) -> SessionId {
    SessionId::new(name).expect("session id")
}

/// Claims one key's SPKI and signs with another: the forgery under test.
struct ClaimingSigner {
    claimed_spki: Vec<u8>,
    actual_key: SigningKey,
}

impl ListenerProofSigner for ClaimingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        Ok(self.claimed_spki.clone())
    }
    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        self.actual_key.sign_der(transcript)
    }
}

/// A key store that fails, as a locked or unavailable Keystore would.
struct FailingSigner;

impl ListenerProofSigner for FailingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        Err(ProofExchangeError::SignerFailed(
            "key store locked".to_owned(),
        ))
    }
    fn sign_der(&self, _transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        Err(ProofExchangeError::SignerFailed(
            "key store locked".to_owned(),
        ))
    }
}

/// Reads its public key, then fails to sign (for example, a user-auth gate).
struct SignFailingSigner {
    key: SigningKey,
}

impl ListenerProofSigner for SignFailingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        self.key.spki_der()
    }
    fn sign_der(&self, _transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        Err(ProofExchangeError::SignerFailed(
            "signing refused".to_owned(),
        ))
    }
}

#[test]
fn a_refused_signature_is_reported_and_never_sent_as_an_empty_proof() {
    let host = host_identity(3);
    let session = session("sign-refused-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let host_spki = host.spki.clone();
    let client_session = session.clone();
    let signer = SignFailingSigner {
        key: listener_key(7),
    };
    let client =
        thread::spawn(move || connect_and_prove(address, &host_spki, &client_session, 1, &signer));
    let host_outcome = accept_and_verify(
        &server,
        &host.certificate,
        &host.signing_key,
        &host.spki,
        &session,
        1,
    );
    let client_outcome = client.join().expect("client thread");
    assert!(
        matches!(client_outcome, Err(ProofExchangeError::SignerFailed(_))),
        "a refused signature must surface to the listener: {client_outcome:?}"
    );
    assert!(host_outcome.is_err(), "an empty proof must never verify");
}

/// Low-level host side for attack tests that need the raw TLS stream.
fn accept_host(
    listener: &TcpListener,
    host: &Host,
) -> Result<(StreamOwned<ServerConnection, TcpStream>, [u8; 32]), ProofExchangeError> {
    let (mut tcp, _) = listener.accept()?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    let config = super::secure_control_tls::host_server_config(
        host.certificate.clone(),
        Arc::clone(&host.signing_key),
    )?;
    let mut connection = ServerConnection::new(config)?;
    connection.complete_io(&mut tcp)?;
    let binding = server_binding(&connection)?;
    Ok((StreamOwned::new(connection, tcp), binding))
}

/// Low-level listener side for attack tests that need the raw TLS stream.
fn dial_listener(
    address: std::net::SocketAddr,
    pinned_host_spki: &[u8],
) -> Result<(StreamOwned<ClientConnection, TcpStream>, [u8; 32]), ProofExchangeError> {
    let mut tcp = TcpStream::connect(address)?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    let config = pinned_client_config(pinned_host_spki)?;
    let name = ServerName::try_from(HOST_SERVER_NAME).expect("name");
    let mut connection = ClientConnection::new(config, name)?;
    connection.complete_io(&mut tcp)?;
    let binding = client_binding(&connection)?;
    Ok((StreamOwned::new(connection, tcp), binding))
}

#[test]
fn a_listener_proving_its_pinned_key_is_verified_end_to_end() {
    let host = host_identity(3);
    let key = listener_key(7);
    let listener_public = listener_spki(&key).expect("spki");
    let session = session("proof-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client_session = session.clone();
    let host_spki = host.spki.clone();
    let client =
        thread::spawn(move || connect_and_prove(address, &host_spki, &client_session, 1, &key));

    let verified = accept_and_verify(
        &server,
        &host.certificate,
        &host.signing_key,
        &host.spki,
        &session,
        1,
    )
    .expect("proof verifies");
    client.join().expect("client thread").expect("client proof");

    let listener_digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(&listener_public).into();
    assert_eq!(
        verified.fingerprint(),
        &listener_digest,
        "verified key must be the listener's key"
    );
    let host_digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(&host.spki).into();
    assert_eq!(verified.host_fingerprint(), &host_digest);
}

#[test]
fn a_listener_pinned_to_another_host_key_never_reaches_the_proof() {
    let host = host_identity(3);
    let other = host_identity(4);
    let session = session("pin-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let host_spki = other.spki.clone();
    let client_session = session.clone();
    let client = thread::spawn(move || {
        connect_and_prove(address, &host_spki, &client_session, 1, &listener_key(7))
    });
    let host_outcome = accept_and_verify(
        &server,
        &host.certificate,
        &host.signing_key,
        &host.spki,
        &session,
        1,
    );
    let client_outcome = client.join().expect("client thread");
    assert!(
        client_outcome.is_err(),
        "a listener pinned to another key completed the proof"
    );
    assert!(host_outcome.is_err());
}

#[test]
fn a_claimed_key_the_listener_does_not_hold_is_refused() {
    let host = host_identity(3);
    let victim = listener_spki(&listener_key(7)).expect("victim spki");
    let signer = ClaimingSigner {
        claimed_spki: victim,
        actual_key: listener_key(9),
    };
    let session = session("forgery-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let host_spki = host.spki.clone();
    let client_session = session.clone();
    let client =
        thread::spawn(move || connect_and_prove(address, &host_spki, &client_session, 1, &signer));
    let outcome = accept_and_verify(
        &server,
        &host.certificate,
        &host.signing_key,
        &host.spki,
        &session,
        1,
    );
    let _ = client.join().expect("client thread");
    assert!(
        matches!(
            outcome,
            Err(ProofExchangeError::Proof(
                ListenerProofError::SignatureMismatch
            ))
        ),
        "a forged key was accepted: {outcome:?}"
    );
}

#[test]
fn a_signer_failure_is_reported_on_the_listener_and_never_proves_anything() {
    let host = host_identity(3);
    let session = session("signer-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let host_spki = host.spki.clone();
    let client_session = session.clone();
    let client = thread::spawn(move || {
        connect_and_prove(address, &host_spki, &client_session, 1, &FailingSigner)
    });
    let host_outcome = accept_and_verify(
        &server,
        &host.certificate,
        &host.signing_key,
        &host.spki,
        &session,
        1,
    );
    let client_outcome = client.join().expect("client thread");
    assert!(
        matches!(client_outcome, Err(ProofExchangeError::SignerFailed(_))),
        "the signer failure must surface to the listener: {client_outcome:?}"
    );
    assert!(
        host_outcome.is_err(),
        "no proof may be accepted after a signer failure"
    );
}

#[test]
fn a_proof_captured_on_one_connection_is_refused_on_another() {
    let host = host_identity(3);
    let listener = listener_key(7);
    let session = session("replay-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let capture_session = session.clone();
    let capture_listener = listener.clone();
    let legitimate = thread::spawn(move || {
        let (mut stream, binding) = dial_listener(address, &host_spki_for(3)).expect("dial");
        let mut nonce = [0_u8; 32];
        stream.read_exact(&mut nonce).expect("nonce");
        let frame = build_listener_response(
            &capture_session,
            1,
            &host_spki_for(3),
            nonce,
            binding,
            &capture_listener,
        )
        .expect("frame");
        stream.write_all(&frame).expect("send");
        frame
    });
    let (mut first, first_binding) = accept_host(&server, &host).expect("accept one");
    let first_outcome = host_verify_listener(&mut first, first_binding, &session, 1, &host.spki);
    let captured = legitimate.join().expect("legitimate client");
    assert!(
        first_outcome.is_ok(),
        "the legitimate proof must verify first: {first_outcome:?}"
    );

    let replayed = thread::spawn(move || {
        let (mut stream, _) = dial_listener(address, &host_spki_for(3)).expect("dial two");
        let mut nonce = [0_u8; 32];
        stream.read_exact(&mut nonce).expect("nonce two");
        stream.write_all(&captured).expect("replay");
    });
    let (mut second, second_binding) = accept_host(&server, &host).expect("accept two");
    let replay_outcome = host_verify_listener(&mut second, second_binding, &session, 1, &host.spki);
    replayed.join().expect("replay client");
    assert!(
        matches!(
            replay_outcome,
            Err(ProofExchangeError::Proof(
                ListenerProofError::SignatureMismatch
            ))
        ),
        "a replayed proof was accepted: {replay_outcome:?}"
    );
}

fn host_spki_for(seed: u8) -> Vec<u8> {
    host_identity(seed).spki
}

#[test]
fn a_proof_for_one_session_is_refused_for_another_on_the_same_connection() {
    let host = host_identity(3);
    let key = listener_key(7);
    let signed_session = session("session-a");
    let checked_session = session("session-b");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let host_spki = host.spki.clone();
    let client =
        thread::spawn(move || connect_and_prove(address, &host_spki, &signed_session, 1, &key));
    let outcome = accept_and_verify(
        &server,
        &host.certificate,
        &host.signing_key,
        &host.spki,
        &checked_session,
        1,
    );
    let _ = client.join().expect("client thread");
    assert!(
        matches!(
            outcome,
            Err(ProofExchangeError::Proof(
                ListenerProofError::SignatureMismatch
            ))
        ),
        "a proof for another session was accepted: {outcome:?}"
    );
}

#[test]
fn an_oversized_length_prefix_is_refused_as_malformed() {
    let host = host_identity(3);
    let session = session("oversize-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client = thread::spawn(move || {
        let (mut stream, _) = dial_listener(address, &host_spki_for(3)).expect("dial");
        let mut nonce = [0_u8; 32];
        stream.read_exact(&mut nonce).expect("nonce");
        stream.write_all(&[0xff, 0xff]).expect("send");
    });
    let (mut stream, binding) = accept_host(&server, &host).expect("accept");
    let outcome = host_verify_listener(&mut stream, binding, &session, 1, &host.spki);
    client.join().expect("client thread");
    assert!(
        matches!(outcome, Err(ProofExchangeError::MalformedFrame)),
        "an oversized field was not refused as malformed: {outcome:?}"
    );
}

/// A proof signature produced by Android Keystore (`SHA256withECDSA`, DER)
/// on a Samsung SM-A546E, Android 16, captured 2026-11-07. The core verifier
/// must accept what the phone produces, not only what this crate produces.
#[test]
fn a_keystore_signature_from_android_verifies_in_the_core_verifier() {
    use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
    use p256::pkcs8::DecodePublicKey;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    let spki = unhex(
        "3059301306072a8648ce3d020106082a8648ce3d030107034200041b0cea1d93383d7ded44990a987f656a0ee706543348c5cdecd3be249f46c01a1634936f9c12a6b6072929ea6624054146ae4ff6284130a85c494f67f8cac52d",
    );
    let payload = unhex("73696c656e742d646973636f2063726f73732d6c616e677561676520766563746f72");
    let signature = unhex(
        "3046022100966e8ae2f0a2f445c994c695490ee07642de3f4418e500dc470d99ffb886ea0d022100ee7144bc8aa789058535d9acac56878aa93eeea731faa35c51e4343b72006740",
    );
    let key = VerifyingKey::from_public_key_der(&spki).expect("android spki is p-256");
    let signature = Signature::from_der(&signature).expect("android signature is der");
    key.verify(&payload, &signature)
        .expect("android signature verifies");
    // The same signature must not verify for any other transcript.
    assert!(key.verify(b"another transcript", &signature).is_err());
}
