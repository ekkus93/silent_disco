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

use super::secure_control_tls::{HOST_SERVER_NAME, host_server_config, pinned_client_config};
use super::secure_identity::ListenerProofError;
use super::secure_listener_proof::{
    ProofExchangeError, build_listener_response, client_binding, host_verify_listener,
    listener_prove, listener_spki, server_binding,
};
use crate::domain::SessionId;

const TIMEOUT: Duration = Duration::from_secs(10);

struct Host {
    spki: Vec<u8>,
    certificate: CertificateDer<'static>,
    signing_key: Arc<dyn RustlsSigningKey>,
}

fn host(seed: u8) -> Host {
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

/// Host side: accept, complete the handshake and return the TLS stream with
/// its exporter binding.
fn accept_host(
    listener: &TcpListener,
    host: &Host,
) -> Result<(StreamOwned<ServerConnection, TcpStream>, [u8; 32]), ProofExchangeError> {
    let (mut tcp, _) = listener.accept()?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    let config = host_server_config(host.certificate.clone(), Arc::clone(&host.signing_key))?;
    let mut connection = ServerConnection::new(config)?;
    connection.complete_io(&mut tcp)?;
    let binding = server_binding(&connection)?;
    Ok((StreamOwned::new(connection, tcp), binding))
}

/// Listener side: connect, pin the host key and complete the handshake.
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

fn session(name: &str) -> SessionId {
    SessionId::new(name).expect("session id")
}

#[test]
fn a_listener_proving_its_pinned_key_over_tls_is_verified_for_this_session() {
    let host = host(3);
    let listener_key = listener_key(7);
    let listener_public = listener_spki(&listener_key).expect("spki");
    let session = session("proof-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client_session = session.clone();
    let client_public = listener_public.clone();
    let client = thread::spawn(move || {
        let (mut stream, binding) = dial_listener(address, &host_spki_for(3)).expect("dial");
        listener_prove(
            &mut stream,
            binding,
            &client_session,
            1,
            &host_spki_for(3),
            &client_public,
            &listener_key,
        )
    });

    let (mut stream, binding) = accept_host(&server, &host).expect("accept");
    let verified = host_verify_listener(&mut stream, binding, &session, 1, &host.spki)
        .expect("proof verifies");
    client.join().expect("client thread").expect("client proof");
    let host_digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(&host.spki).into();
    assert_eq!(verified.host_fingerprint(), &host_digest);
}

#[test]
fn a_valid_proof_is_refused_for_a_different_session_on_the_same_connection() {
    let host = host(3);
    let listener = listener_key(7);
    let public = listener_spki(&listener).expect("spki");
    let signed_session = session("session-a");
    let checked_session = session("session-b");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client = thread::spawn(move || {
        let (mut stream, binding) = dial_listener(address, &host_spki_for(3)).expect("dial");
        let mut nonce = [0_u8; 32];
        stream.read_exact(&mut nonce).expect("nonce");
        let frame = build_listener_response(
            &signed_session,
            1,
            &host_spki_for(3),
            nonce,
            binding,
            &public,
            &listener,
        )
        .expect("frame");
        stream.write_all(&frame).expect("send");
    });
    let (mut stream, binding) = accept_host(&server, &host).expect("accept");
    let outcome = host_verify_listener(&mut stream, binding, &checked_session, 1, &host.spki);
    client.join().expect("client thread");
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

fn host_spki_for(seed: u8) -> Vec<u8> {
    host(seed).spki
}

#[test]
fn a_listener_claiming_a_key_it_does_not_hold_is_refused() {
    // The attacker announces the victim's SPKI but signs with its own key.
    let host = host(3);
    let victim = listener_spki(&listener_key(7)).expect("victim spki");
    let attacker_key = listener_key(9);
    let session = session("forgery-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client_session = session.clone();
    let client = thread::spawn(move || {
        let (mut stream, binding) = dial_listener(address, &host_spki_for(3)).expect("dial");
        let mut nonce = [0_u8; 32];
        stream.read_exact(&mut nonce).expect("nonce");
        let frame = build_listener_response(
            &client_session,
            1,
            &host_spki_for(3),
            nonce,
            binding,
            &victim,
            &attacker_key,
        )
        .expect("frame");
        stream.write_all(&frame).expect("send");
    });

    let (mut stream, binding) = accept_host(&server, &host).expect("accept");
    let outcome = host_verify_listener(&mut stream, binding, &session, 1, &host.spki);
    client.join().expect("client thread");
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
fn a_proof_captured_on_one_connection_is_refused_on_another() {
    let host = host(3);
    let listener = listener_key(7);
    let public = listener_spki(&listener).expect("spki");
    let session = session("replay-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    // Connection one: a legitimate proof, captured by the attacker.
    let capture_session = session.clone();
    let capture_public = public.clone();
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
            &capture_public,
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

    // Connection two: the attacker replays the captured frame against a fresh
    // nonce and a different exporter value.
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

#[test]
fn an_oversized_length_prefix_is_refused_before_any_allocation_of_its_size() {
    let host = host(3);
    let session = session("oversize-session");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");

    let client = thread::spawn(move || {
        let (mut stream, _) = dial_listener(address, &host_spki_for(3)).expect("dial");
        let mut nonce = [0_u8; 32];
        stream.read_exact(&mut nonce).expect("nonce");
        // A 65,535-byte claim with no body behind it.
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
