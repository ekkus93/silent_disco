//! Owner-thread TLS frame channel over real TLS 1.3 on loopback TCP.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use p256::{
    ecdsa::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};
use rustls::{ClientConnection, Connection, ServerConnection, pki_types::ServerName};

use super::secure_control_tls::{
    HOST_SERVER_NAME, host_server_config, host_tls_identity_from_pkcs8, pinned_client_config,
};
use super::secure_frame_channel::{
    MAX_SECURE_FRAME_BYTES, SecureChannelError, spawn_secure_frame_channel,
};

const WAIT: Duration = Duration::from_secs(10);

fn host_pkcs8(seed: u8) -> Vec<u8> {
    SigningKey::from_slice(&[seed; 32])
        .expect("key")
        .to_pkcs8_der()
        .expect("pkcs8")
        .as_bytes()
        .to_vec()
}

fn spki_of(seed: u8) -> Vec<u8> {
    SigningKey::from_slice(&[seed; 32])
        .expect("key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

/// Starts a host channel on an accepted socket and a listener channel that
/// pins `pinned_spki`. Returns both channels.
fn pair(
    host_seed: u8,
    pinned_spki: &[u8],
) -> (
    super::secure_frame_channel::SecureFrameChannel,
    super::secure_frame_channel::SecureFrameChannel,
) {
    let identity = host_tls_identity_from_pkcs8(&host_pkcs8(host_seed)).expect("identity");
    let server = TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = server.local_addr().expect("address");
    let server_config = host_server_config(
        identity.certificate.clone(),
        Arc::clone(&identity.signing_key),
    )
    .expect("server config");
    let host_thread = std::thread::spawn(move || {
        let (tcp, _) = server.accept().expect("accept");
        let connection = ServerConnection::new(server_config).expect("server connection");
        spawn_secure_frame_channel(tcp, Connection::Server(connection)).expect("host channel")
    });

    let client_config = pinned_client_config(pinned_spki).expect("client config");
    let name = ServerName::try_from(HOST_SERVER_NAME).expect("name");
    let client_connection = ClientConnection::new(client_config, name).expect("client connection");
    let tcp = TcpStream::connect(address).expect("connect");
    let client = spawn_secure_frame_channel(tcp, Connection::Client(client_connection))
        .expect("listener channel");
    (host_thread.join().expect("host thread"), client)
}

#[test]
fn frames_flow_both_ways_over_a_pinned_tls_13_connection() {
    let host_spki = spki_of(21);
    let (host, listener) = pair(21, &host_spki);

    listener
        .send(b"join request".to_vec())
        .expect("send to host");
    assert_eq!(
        host.recv_timeout(WAIT).expect("host receives"),
        b"join request"
    );

    host.send(b"approved".to_vec()).expect("send to listener");
    assert_eq!(
        listener.recv_timeout(WAIT).expect("listener receives"),
        b"approved"
    );

    // Largest legal frame and a few small ones, in order.
    let big = vec![0x5a_u8; MAX_SECURE_FRAME_BYTES];
    listener.send(big.clone()).expect("send big");
    listener.send(vec![1]).expect("send small");
    assert_eq!(host.recv_timeout(WAIT).expect("big"), big);
    assert_eq!(host.recv_timeout(WAIT).expect("small"), vec![1]);

    listener.close();
    host.close();
}

#[test]
fn a_listener_pinned_to_another_key_gets_no_frames_and_the_host_sees_the_failure() {
    let (host, listener) = pair(21, &spki_of(22));
    // Whatever the listener sends after a failed handshake never reaches the host.
    let _ = listener.send(b"should not arrive".to_vec());
    assert!(
        listener.recv_timeout(WAIT).is_err(),
        "the listener must not receive application data"
    );
    assert!(
        host.recv_timeout(Duration::from_secs(3)).is_err(),
        "the host must not receive application data from an unpinned listener"
    );
    listener.close();
    host.close();
}

#[test]
fn an_oversized_or_empty_frame_is_refused_before_it_reaches_the_wire() {
    let host_spki = spki_of(23);
    let (host, listener) = pair(23, &host_spki);
    assert!(matches!(
        listener.send(vec![0; MAX_SECURE_FRAME_BYTES + 1]),
        Err(SecureChannelError::FrameTooLarge { .. })
    ));
    assert!(matches!(
        listener.send(Vec::new()),
        Err(SecureChannelError::FrameTooLarge { length: 0 })
    ));
    // The channel still works after refusals.
    listener
        .send(b"still open".to_vec())
        .expect("send after refusal");
    assert_eq!(host.recv_timeout(WAIT).expect("receive"), b"still open");
    listener.close();
    host.close();
}

#[test]
fn a_clean_close_is_observed_by_the_peer() {
    let host_spki = spki_of(24);
    let (host, listener) = pair(24, &host_spki);
    listener.send(b"last".to_vec()).expect("send");
    assert_eq!(host.recv_timeout(WAIT).expect("receive"), b"last");
    listener.close();
    // After the listener closes, the host sees the end of the connection.
    assert!(
        host.recv_timeout(WAIT).is_err(),
        "the host must observe the closed connection"
    );
    host.close();
}

#[test]
fn a_frame_queued_just_before_the_channel_is_dropped_still_reaches_the_peer() {
    let host_spki = spki_of(25);
    let (host, listener) = pair(25, &host_spki);
    listener.send(b"proof".to_vec()).expect("send");
    // Drop immediately, as a listener does once its proof is sent.
    drop(listener);
    assert_eq!(
        host.recv_timeout(WAIT)
            .expect("queued frame must be delivered before close"),
        b"proof"
    );
    host.close();
}
