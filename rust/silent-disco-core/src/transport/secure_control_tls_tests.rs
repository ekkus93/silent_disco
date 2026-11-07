//! Real TLS 1.3 handshakes over loopback TCP against the pinned-host-key
//! configuration in `secure_control_tls`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use p256::{
    ecdsa::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::{
    ClientConnection, ServerConnection, StreamOwned,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
    sign::SigningKey as RustlsSigningKey,
    version::TLS13,
};

use super::secure_control_tls::{HOST_SERVER_NAME, host_server_config, pinned_client_config};

/// A host identity: its SPKI (what an invitation pins), the certificate that
/// carries the key, and the rustls signing key the server uses.
struct HostIdentity {
    spki: Vec<u8>,
    certificate: rustls::pki_types::CertificateDer<'static>,
    signing_key: Arc<dyn RustlsSigningKey>,
}

fn host_identity(seed: u8) -> HostIdentity {
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
        .expect("self-signed certificate")
        .der()
        .clone();
    let signing_key = rustls::crypto::ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
        PrivatePkcs8KeyDer::from(pkcs8),
    ))
    .expect("ecdsa signing key");
    HostIdentity {
        spki,
        certificate,
        signing_key,
    }
}

/// Runs one echo exchange over TLS. The server side is a thread that performs
/// the handshake and echoes five bytes back, so a refused handshake shows up as
/// an error on the client without hanging the test.
fn echo_over_tls(
    server: &HostIdentity,
    client_pinned_spki: &[u8],
) -> Result<(Option<rustls::ProtocolVersion>, [u8; 5]), std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server_config =
        host_server_config(server.certificate.clone(), Arc::clone(&server.signing_key))
            .expect("server config");
    let server_thread = thread::spawn(move || {
        let (tcp, _) = listener.accept().expect("accept");
        tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .ok();
        let connection = ServerConnection::new(server_config).expect("server connection");
        let mut stream = StreamOwned::new(connection, tcp);
        let mut received = [0_u8; 5];
        if stream.read_exact(&mut received).is_ok() {
            let _ = stream.write_all(&received);
            let _ = stream.flush();
        }
    });

    let client_config = pinned_client_config(client_pinned_spki).expect("client config");
    let tcp = TcpStream::connect(address)?;
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    let name = ServerName::try_from(HOST_SERVER_NAME).expect("name");
    let connection = ClientConnection::new(client_config, name)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut stream = StreamOwned::new(connection, tcp);
    let outcome = (|| -> std::io::Result<[u8; 5]> {
        stream.write_all(b"hello")?;
        stream.flush()?;
        let mut echoed = [0_u8; 5];
        stream.read_exact(&mut echoed)?;
        Ok(echoed)
    })();
    let protocol = stream.conn.protocol_version();
    drop(stream);
    server_thread.join().expect("server thread");
    outcome.map(|echoed| (protocol, echoed))
}

#[test]
fn a_listener_pinned_to_the_host_key_completes_tls13_and_exchanges_bytes() {
    let host = host_identity(3);
    let (protocol, echoed) = echo_over_tls(&host, &host.spki).expect("pinned handshake");
    assert_eq!(protocol, Some(TLS13.version));
    assert_eq!(&echoed, b"hello");
}

#[test]
fn a_listener_pinned_to_another_key_refuses_a_host_even_with_a_valid_certificate() {
    // The host presents a well-formed, self-signed certificate for its own key.
    // The listener pinned a different key, so the certificate alone must not
    // be enough: the handshake signature cannot verify under the pinned key.
    let host = host_identity(3);
    let other = host_identity(4);
    let result = echo_over_tls(&host, &other.spki);
    assert!(
        result.is_err(),
        "a server without the pinned private key completed the handshake"
    );
}

#[test]
fn the_pinned_listener_and_host_round_trip_several_handshakes() {
    // Each connection gets fresh keys and a fresh transcript; a repeated
    // success shows the pin is not consumed or cached in a way that breaks reuse.
    let host = host_identity(5);
    for round in 0..5 {
        let (_, echoed) = echo_over_tls(&host, &host.spki)
            .unwrap_or_else(|error| panic!("round {round} failed: {error}"));
        assert_eq!(&echoed, b"hello");
    }
}
