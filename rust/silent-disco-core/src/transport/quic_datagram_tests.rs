//! Real loopback QUIC handshakes and DATAGRAM frames for the v4 transport.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_datagram::{ALPN_V4, quic_client_config, quic_server_config};
use super::secure_control_tls::{
    HOST_SERVER_NAME, HostTlsIdentity, host_tls_identity_from_pkcs8, pinned_client_config,
};
use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{AudioCodec, AudioDatagram, ProtocolFrame, encode_frame};

const STEP: Duration = Duration::from_secs(10);

fn host(seed: u8) -> HostTlsIdentity {
    let key = SigningKey::from_slice(&[seed; 32]).expect("fixture key");
    let pkcs8 = key.to_pkcs8_der().expect("pkcs8");
    host_tls_identity_from_pkcs8(pkcs8.as_bytes()).expect("host identity")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn localhost() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

/// A real encoded audio frame at the current packet size: 5 ms of 48 kHz
/// stereo 16-bit PCM, 960 payload bytes plus the protocol header.
fn five_ms_audio_frame() -> Vec<u8> {
    encode_frame(&ProtocolFrame::Audio(AudioDatagram {
        session_id: SessionId::new("session-quic").expect("session id"),
        stream_id: StreamId::new("stream-quic").expect("stream id"),
        sequence: PacketSequence::new(7),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        first_sample_index: SampleIndex::new(7 * 240),
        host_presentation_time_ms: MonotonicMillis::new(1_035),
        payload: (0..960_u32).map(|byte| (byte % 251) as u8).collect(),
    }))
    .expect("encoded audio frame")
}

/// Drives the host side of one handshake that must not complete. quinn only
/// answers a handshake once the application accepts it.
async fn accept_and_expect_failure(server: quinn::Endpoint) {
    let incoming = tokio::time::timeout(STEP, server.accept())
        .await
        .expect("connection attempt in time")
        .expect("incoming");
    let outcome = tokio::time::timeout(STEP, incoming)
        .await
        .expect("host handshake settles in time");
    assert!(
        outcome.is_err(),
        "the host completed a handshake it should have failed"
    );
}

#[test]
fn a_pinned_listener_receives_a_full_audio_packet_as_one_quic_datagram() {
    let identity = host(7);
    let host_spki = identity.spki.clone();
    let frame = five_ms_audio_frame();
    runtime().block_on(async move {
        let server =
            quinn::Endpoint::server(quic_server_config(&identity).expect("server"), localhost())
                .expect("server endpoint");
        let address = server.local_addr().expect("server address");
        let sent = frame.clone();
        let host_task = tokio::spawn(async move {
            let connection = server
                .accept()
                .await
                .expect("incoming")
                .await
                .expect("host handshake");
            let limit = connection
                .max_datagram_size()
                .expect("datagrams negotiated");
            assert!(
                limit >= sent.len(),
                "a {}-byte audio frame exceeds the {limit}-byte datagram limit",
                sent.len()
            );
            connection
                .send_datagram(sent.into())
                .expect("datagram sent");
            // Hold the connection until the listener has read the datagram.
            let _ = connection.closed().await;
        });

        let mut client = quinn::Endpoint::client(localhost()).expect("client endpoint");
        client.set_default_client_config(quic_client_config(&host_spki).expect("client"));
        let connection = tokio::time::timeout(
            STEP,
            client.connect(address, HOST_SERVER_NAME).expect("connect"),
        )
        .await
        .expect("handshake in time")
        .expect("listener handshake");
        assert_eq!(
            connection.handshake_data().and_then(|data| data
                .downcast::<quinn::crypto::rustls::HandshakeData>()
                .ok()
                .and_then(|data| data.protocol)),
            Some(ALPN_V4.to_vec())
        );
        let received = tokio::time::timeout(STEP, connection.read_datagram())
            .await
            .expect("datagram in time")
            .expect("datagram");
        assert_eq!(received.as_ref(), frame.as_slice());
        connection.close(0_u32.into(), b"done");
        host_task.await.expect("host task");
    });
}

#[test]
fn a_listener_pinned_to_another_key_refuses_the_host() {
    let identity = host(7);
    let other_spki = host(8).spki;
    runtime().block_on(async move {
        let server =
            quinn::Endpoint::server(quic_server_config(&identity).expect("server"), localhost())
                .expect("server endpoint");
        let address = server.local_addr().expect("server address");
        let host_task = tokio::spawn(accept_and_expect_failure(server));
        let mut client = quinn::Endpoint::client(localhost()).expect("client endpoint");
        client.set_default_client_config(quic_client_config(&other_spki).expect("client"));
        let outcome = tokio::time::timeout(
            STEP,
            client.connect(address, HOST_SERVER_NAME).expect("connect"),
        )
        .await
        .expect("handshake settles in time");
        assert!(outcome.is_err(), "a host with an unpinned key was accepted");
        host_task.await.expect("host task");
    });
}

#[test]
fn a_peer_without_the_v4_alpn_cannot_complete_the_handshake() {
    let identity = host(7);
    let host_spki = identity.spki.clone();
    runtime().block_on(async move {
        let server =
            quinn::Endpoint::server(quic_server_config(&identity).expect("server"), localhost())
                .expect("server endpoint");
        let address = server.local_addr().expect("server address");
        let host_task = tokio::spawn(accept_and_expect_failure(server));
        // Same pinned verifier, but no v4 ALPN offered.
        let mut tls = (*pinned_client_config(&host_spki).expect("tls")).clone();
        tls.alpn_protocols = vec![b"silent-disco/3".to_vec()];
        let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls).expect("quic client");
        let mut client = quinn::Endpoint::client(localhost()).expect("client endpoint");
        client.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic)));
        let outcome = tokio::time::timeout(
            STEP,
            client.connect(address, HOST_SERVER_NAME).expect("connect"),
        )
        .await
        .expect("handshake settles in time");
        assert!(outcome.is_err(), "a peer without the v4 ALPN was accepted");
        host_task.await.expect("host task");
    });
}
