//! Real loopback QUIC: the v4 listener proof on the first bidirectional
//! stream, bound to the connection's exporter.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_admission::{V4_HELLO, quic_binding, quic_host_admit, quic_listener_prove};
use super::quic_datagram::{quic_client_config, quic_server_config};
use super::secure_control_tls::{HOST_SERVER_NAME, HostTlsIdentity, host_tls_identity_from_pkcs8};
use super::secure_listener_proof::{
    ListenerProofSigner, ProofExchangeError, build_listener_response,
};
use crate::domain::SessionId;

const STEP: Duration = Duration::from_secs(10);
const GENERATION: u64 = 3;

fn host(seed: u8) -> HostTlsIdentity {
    let key = SigningKey::from_slice(&[seed; 32]).expect("fixture key");
    let pkcs8 = key.to_pkcs8_der().expect("pkcs8");
    host_tls_identity_from_pkcs8(pkcs8.as_bytes()).expect("host identity")
}

fn listener_key(seed: u8) -> SigningKey {
    SigningKey::from_slice(&[seed; 32]).expect("listener key")
}

fn session(name: &str) -> SessionId {
    SessionId::new(name).expect("session id")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// A signer that reports one key but signs with another.
struct ClaimingSigner {
    claimed: Vec<u8>,
    actual: SigningKey,
}

impl ListenerProofSigner for ClaimingSigner {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        Ok(self.claimed.clone())
    }

    fn sign_der(&self, message: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        self.actual.sign_der(message)
    }
}

/// A host endpoint and a listener connection pinned to it.
async fn connected(
    identity: &HostTlsIdentity,
) -> (quinn::Endpoint, quinn::Connection, quinn::Connection) {
    let server = quinn::Endpoint::server(
        quic_server_config(identity).expect("server"),
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
    )
    .expect("server endpoint");
    let address = server.local_addr().expect("address");
    let mut client = quinn::Endpoint::client(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .expect("client endpoint");
    client.set_default_client_config(quic_client_config(&identity.spki).expect("client"));
    let accepting = server.clone();
    let host_side = tokio::spawn(async move {
        accepting
            .accept()
            .await
            .expect("incoming")
            .await
            .expect("host handshake")
    });
    let listener = tokio::time::timeout(
        STEP,
        client.connect(address, HOST_SERVER_NAME).expect("connect"),
    )
    .await
    .expect("handshake in time")
    .expect("listener handshake");
    let host = host_side.await.expect("host task");
    (server, host, listener)
}

#[test]
fn a_listener_proving_its_key_is_admitted_and_the_stream_carries_control() {
    let identity = host(7);
    let key = listener_key(21);
    runtime().block_on(async move {
        let (_server, host_conn, listener_conn) = connected(&identity).await;
        let spki = identity.spki.clone();
        let host_task = tokio::spawn(async move {
            let (verified, mut stream) =
                quic_host_admit(&host_conn, &session("session-q"), GENERATION, &spki)
                    .await
                    .expect("admitted");
            stream.send.write_all(b"approved").await.expect("reply");
            let mut ack = [0_u8; 3];
            stream.recv.read_exact(&mut ack).await.expect("ack");
            assert_eq!(&ack, b"ack");
            (verified, host_conn)
        });
        let mut stream = quic_listener_prove(
            &listener_conn,
            &session("session-q"),
            GENERATION,
            &identity.spki,
            &key,
        )
        .await
        .expect("proof sent");
        let mut reply = [0_u8; 8];
        stream.recv.read_exact(&mut reply).await.expect("reply");
        assert_eq!(&reply, b"approved");
        stream.send.write_all(b"ack").await.expect("ack");
        let (verified, _host_conn) = host_task.await.expect("host task");
        let expected = crate::transport::secure_identity::device_id_for_listener_key(
            &key.spki_der().expect("spki"),
        );
        assert_eq!(verified.canonical_device_id(), expected);
    });
}

#[test]
fn a_listener_claiming_a_key_it_does_not_hold_is_refused() {
    let identity = host(7);
    runtime().block_on(async move {
        let (_server, host_conn, listener_conn) = connected(&identity).await;
        let spki = identity.spki.clone();
        let host_task = tokio::spawn(async move {
            quic_host_admit(&host_conn, &session("session-q"), GENERATION, &spki).await
        });
        let signer = ClaimingSigner {
            claimed: listener_key(22).spki_der().expect("spki"),
            actual: listener_key(23),
        };
        let _ = quic_listener_prove(
            &listener_conn,
            &session("session-q"),
            GENERATION,
            &identity.spki,
            &signer,
        )
        .await;
        let outcome = host_task.await.expect("host task");
        assert!(
            matches!(outcome, Err(ProofExchangeError::Proof(_))),
            "{outcome:?}"
        );
    });
}

#[test]
fn a_proof_for_another_session_is_refused() {
    let identity = host(7);
    let key = listener_key(21);
    runtime().block_on(async move {
        let (_server, host_conn, listener_conn) = connected(&identity).await;
        let spki = identity.spki.clone();
        let host_task = tokio::spawn(async move {
            quic_host_admit(&host_conn, &session("session-q"), GENERATION, &spki).await
        });
        let _ = quic_listener_prove(
            &listener_conn,
            &session("session-other"),
            GENERATION,
            &identity.spki,
            &key,
        )
        .await;
        let outcome = host_task.await.expect("host task");
        assert!(
            matches!(outcome, Err(ProofExchangeError::Proof(_))),
            "{outcome:?}"
        );
    });
}

#[test]
fn a_peer_without_the_v4_hello_is_refused_before_any_challenge() {
    let identity = host(7);
    runtime().block_on(async move {
        let (_server, host_conn, listener_conn) = connected(&identity).await;
        let spki = identity.spki.clone();
        let host_task = tokio::spawn(async move {
            quic_host_admit(&host_conn, &session("session-q"), GENERATION, &spki).await
        });
        let (mut send, mut recv) = listener_conn.open_bi().await.expect("stream");
        send.write_all(b"silent-disco/v3\0").await.expect("hello");
        let outcome = host_task.await.expect("host task");
        assert!(
            matches!(outcome, Err(ProofExchangeError::Channel(_))),
            "{outcome:?}"
        );
        // No nonce was issued to a non-v4 peer.
        let mut nonce = [0_u8; 32];
        assert!(
            tokio::time::timeout(Duration::from_millis(500), recv.read_exact(&mut nonce))
                .await
                .map_or(true, |read| read.is_err()),
            "the host issued a challenge to a non-v4 peer"
        );
    });
}

#[test]
fn a_proof_bound_to_another_connection_is_refused() {
    let identity = host(7);
    let key = listener_key(21);
    runtime().block_on(async move {
        let (_server_a, _host_a, listener_a) = connected(&identity).await;
        let (_server_b, host_b, listener_b) = connected(&identity).await;
        let other_binding = quic_binding(&listener_a).expect("binding A");
        let spki = identity.spki.clone();
        let host_task = tokio::spawn(async move {
            quic_host_admit(&host_b, &session("session-q"), GENERATION, &spki).await
        });
        // A correct proof for this session and nonce, but bound to connection A.
        let (mut send, mut recv) = listener_b.open_bi().await.expect("stream");
        send.write_all(V4_HELLO).await.expect("hello");
        let mut nonce = [0_u8; 32];
        recv.read_exact(&mut nonce).await.expect("nonce");
        let frame = build_listener_response(
            &session("session-q"),
            GENERATION,
            &identity.spki,
            nonce,
            other_binding,
            &key,
        )
        .expect("frame");
        send.write_all(&frame).await.expect("frame sent");
        let outcome = host_task.await.expect("host task");
        assert!(
            matches!(outcome, Err(ProofExchangeError::Proof(_))),
            "{outcome:?}"
        );
    });
}

/// Two listeners on one host get independent keys: each QUIC connection
/// derives its own secrets, so its exporter value (what a listener's proof is
/// bound to) differs between connections while both ends of one agree.
#[test]
fn each_listener_connection_to_one_host_has_its_own_keys() {
    let identity = host(31);
    runtime().block_on(async {
        let server = quinn::Endpoint::server(
            quic_server_config(&identity).expect("server"),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        )
        .expect("server endpoint");
        let address = server.local_addr().expect("address");
        let mut bindings = Vec::new();
        for _ in 0..2 {
            let mut client = quinn::Endpoint::client(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .expect("client endpoint");
            client.set_default_client_config(quic_client_config(&identity.spki).expect("client"));
            let accepting = server.clone();
            let host_side = tokio::spawn(async move {
                accepting
                    .accept()
                    .await
                    .expect("incoming")
                    .await
                    .expect("host handshake")
            });
            let listener = tokio::time::timeout(
                STEP,
                client.connect(address, HOST_SERVER_NAME).expect("connect"),
            )
            .await
            .expect("handshake in time")
            .expect("listener handshake");
            let host_end = host_side.await.expect("host task");
            let listener_binding = quic_binding(&listener).expect("listener exporter");
            assert_eq!(
                listener_binding,
                quic_binding(&host_end).expect("host exporter"),
                "both ends of one connection derive the same secret"
            );
            bindings.push(listener_binding);
            listener.close(0_u32.into(), b"done");
        }
        assert_ne!(
            bindings[0], bindings[1],
            "two listeners must not share connection keys"
        );
    });
}
