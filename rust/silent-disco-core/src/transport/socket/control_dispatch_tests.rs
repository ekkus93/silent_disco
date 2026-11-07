//! Accept-time routing on a real host: TLS admission, refusals, and the
//! visible reason for each refused connection.

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use p256::{
    ecdsa::SigningKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};

use crate::domain::SessionId;
use crate::transport::secure_admission::dial_and_prove;
use crate::transport::secure_control_tls::host_tls_identity_from_pkcs8;
use crate::transport::secure_listener_proof::{ListenerProofSigner, ProofExchangeError};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ManualTransportClock, TransportChannel,
    TransportErrorKind, TransportEvent,
};

use super::SocketHostTransport;

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

fn host_with_secure(seed: u8, session: &SessionId) -> (SocketHostTransport, Vec<u8>) {
    let pkcs8 = key(seed).to_pkcs8_der().expect("pkcs8").as_bytes().to_vec();
    let identity = Arc::new(host_tls_identity_from_pkcs8(&pkcs8).expect("identity"));
    let host_spki = identity.spki.clone();
    let mut config = HostTransportConfig::loopback(session.clone());
    config.secure_control = Some(identity);
    let host =
        SocketHostTransport::bind(config, Arc::new(ManualTransportClock::new(0))).expect("bind");
    (host, host_spki)
}

fn control_address(host: &SocketHostTransport) -> SocketAddr {
    let endpoint = host.endpoint();
    SocketAddr::new(endpoint.address, endpoint.control_port)
}

fn next_rejection(host: &mut SocketHostTransport) -> (TransportErrorKind, String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match host.recv_event(remaining).expect("host event") {
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                error,
                ..
            } => return (error.kind, error.message),
            _ => assert!(Instant::now() < deadline, "no rejection event arrived"),
        }
    }
}

struct ForgingSigner {
    claimed: Vec<u8>,
    actual: SigningKey,
}

impl ListenerProofSigner for ForgingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        Ok(self.claimed.clone())
    }
    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        self.actual.sign_der(transcript)
    }
}

#[test]
fn an_honest_tls_listener_is_admitted_and_its_channel_is_closed_until_the_join_path_exists() {
    let session = SessionId::new("dispatch-admit").expect("session");
    let (mut host, host_spki) = host_with_secure(41, &session);
    let listener = std::thread::spawn({
        let address = control_address(&host);
        let session = session.clone();
        move || dial_and_prove(address, &host_spki, &session, 1, &key(42))
    });

    let (kind, message) = next_rejection(&mut host);
    assert_eq!(kind, TransportErrorKind::Protocol);
    assert!(
        message.contains("admitted over TLS"),
        "unexpected message: {message}"
    );
    let channel = listener
        .join()
        .expect("listener thread")
        .expect("listener proved");
    // The host does not keep the channel open, so the listener sees it end.
    assert!(channel.recv_timeout(Duration::from_secs(10)).is_err());
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_listener_claiming_another_key_is_refused_as_unauthorized() {
    let session = SessionId::new("dispatch-forged").expect("session");
    let (mut host, host_spki) = host_with_secure(43, &session);
    let forger = ForgingSigner {
        claimed: spki(44),
        actual: key(45),
    };
    let address = control_address(&host);
    let session_for_client = session.clone();
    let client = std::thread::spawn(move || {
        dial_and_prove(address, &host_spki, &session_for_client, 1, &forger).map(drop)
    });

    let (kind, message) = next_rejection(&mut host);
    assert_eq!(kind, TransportErrorKind::Unauthorized, "message: {message}");
    let _ = client.join();
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_connection_is_refused_with_a_reason_when_no_secure_identity_is_configured() {
    let session = SessionId::new("dispatch-unconfigured").expect("session");
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        Arc::new(ManualTransportClock::new(0)),
    )
    .expect("bind");
    let address = control_address(&host);
    let pkcs8 = key(46).to_pkcs8_der().expect("pkcs8").as_bytes().to_vec();
    let host_spki = host_tls_identity_from_pkcs8(&pkcs8).expect("identity").spki;
    let session_for_client = session.clone();
    let client = std::thread::spawn(move || {
        dial_and_prove(address, &host_spki, &session_for_client, 1, &key(47)).map(drop)
    });

    let (kind, message) = next_rejection(&mut host);
    assert_eq!(kind, TransportErrorKind::Protocol);
    assert!(
        message.contains("not configured"),
        "unexpected message: {message}"
    );
    assert!(
        client.join().expect("client thread").is_err(),
        "the listener must not be admitted"
    );
    host.shutdown().expect("shutdown");
}

#[test]
fn a_silent_or_malformed_connection_is_left_to_the_plaintext_path_not_refused_here() {
    let session = SessionId::new("dispatch-legacy").expect("session");
    let (mut host, _) = host_with_secure(48, &session);
    let mut garbage = TcpStream::connect(control_address(&host)).expect("connect");
    garbage.write_all(b"GET / HTTP/1.1\r\n\r\n").expect("send");
    let _silent = TcpStream::connect(control_address(&host)).expect("connect silent");

    // Neither connection gets a TLS refusal from dispatch: the plaintext path
    // owns them, and its own decoder and timers decide what happens next.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(TransportEvent::Rejected { error, .. }) =
            host.recv_event(Duration::from_millis(100))
        {
            assert!(
                !error.message.contains("TLS") && !error.message.contains("secure control"),
                "dispatch refused a non-TLS connection: {}",
                error.message
            );
        }
    }
    host.shutdown().expect("shutdown");
}
