//! A listener that stops reading its control stream must not stall the host:
//! its control writes fail within a bound, it is disconnected, and every
//! other listener keeps receiving control (Stop included).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_admission::quic_listener_prove;
use super::quic_datagram::quic_client_config;
use super::quic_host_worker::{
    CONTROL_WRITE_TIMEOUT, QuicHostConfig, QuicHostError, QuicHostEvent, QuicHostWorker,
};
use super::quic_listener_worker::{QuicListenerConfig, QuicListenerEvent, QuicListenerWorker};
use super::secure_control_tls::{HOST_SERVER_NAME, HostTlsIdentity, host_tls_identity_from_pkcs8};
use crate::domain::SessionId;

const WAIT: Duration = Duration::from_secs(10);
const GENERATION: u64 = 4;

fn identity() -> HostTlsIdentity {
    let key = SigningKey::from_slice(&[9; 32]).expect("fixture key");
    host_tls_identity_from_pkcs8(key.to_pkcs8_der().expect("pkcs8").as_bytes()).expect("identity")
}

fn session() -> SessionId {
    SessionId::new("session-stalled").expect("session id")
}

fn next_host_event(host: &QuicHostWorker) -> QuicHostEvent {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(event) = host
            .recv_event(Duration::from_millis(50))
            .expect("host running")
        {
            return event;
        }
        assert!(Instant::now() < deadline, "no host event in time");
    }
}

fn admitted(host: &QuicHostWorker) -> u64 {
    match next_host_event(host) {
        QuicHostEvent::Admitted { peer, .. } => peer,
        other => panic!("expected an admission, got {other:?}"),
    }
}

/// Keeps a stalled client running until dropped.
struct StalledClient {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for StalledClient {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A raw client on its own runtime thread: it passes admission with a tiny
/// receive window and keeps its connection alive (acknowledging packets),
/// but never reads its control stream.
fn stalled_client(address: SocketAddr, host_spki: &[u8]) -> StalledClient {
    let mut config = quic_client_config(host_spki).expect("client config");
    let mut transport = quinn::TransportConfig::default();
    transport
        .stream_receive_window(2_048_u32.into())
        .receive_window(4_096_u32.into());
    config.transport_config(Arc::new(transport));
    let host_spki = host_spki.to_vec();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (ready, admitted) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let key = SigningKey::from_slice(&[51; 32]).expect("listener key");
            let mut endpoint = quinn::Endpoint::client(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .expect("client");
            endpoint.set_default_client_config(config);
            let connection = endpoint
                .connect(address, HOST_SERVER_NAME)
                .expect("connect")
                .await
                .expect("handshake");
            let stream = quic_listener_prove(&connection, &session(), GENERATION, &host_spki, &key)
                .await
                .expect("admission");
            let _ = ready.send(());
            // Hold the stream unread until the test ends.
            let _ = stopped.await;
            drop(stream);
            connection.close(0_u32.into(), b"test done");
            endpoint.wait_idle().await;
        });
    });
    admitted
        .recv_timeout(WAIT)
        .expect("stalled client admitted");
    StalledClient {
        stop: Some(stop),
        thread: Some(thread),
    }
}

#[test]
fn a_listener_that_stops_reading_is_disconnected_and_others_still_get_control() {
    let identity = identity();
    let host = QuicHostWorker::start(
        QuicHostConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            session: session(),
            generation: GENERATION,
            max_peers: 4,
            event_capacity: 64,
        },
        &identity,
    )
    .expect("host starts");
    let healthy = QuicListenerWorker::connect(
        QuicListenerConfig {
            host: host.local_addr(),
            host_spki: identity.spki.clone(),
            session: session(),
            generation: GENERATION,
            event_capacity: 64,
        },
        Arc::new(SigningKey::from_slice(&[41; 32]).expect("listener key")),
    )
    .expect("healthy listener admitted");
    let healthy_peer = admitted(&host);
    let _stalled = stalled_client(host.local_addr(), &identity.spki);
    let stalled_peer = admitted(&host);

    // A detached thread owns the host, so a stalled worker fails this test
    // instead of hanging it. Keep writing until the stalled peer's window
    // is full and a write fails.
    let (done, answered) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut failure = None;
        for _ in 0..64 {
            if let Err(error) = host.send_control(stalled_peer, vec![7; 1_000]) {
                failure = Some(error);
                break;
            }
        }
        let _ = done.send((failure, started.elapsed(), host));
    });
    let (failure, elapsed, host) = answered
        .recv_timeout(CONTROL_WRITE_TIMEOUT + WAIT)
        .expect("the host froze on a listener that stopped reading");
    match failure {
        Some(QuicHostError::Send(reason)) => {
            assert!(reason.contains("stopped reading"), "{reason}");
        }
        other => panic!("expected the stalled write to fail, got {other:?}"),
    }
    assert!(
        elapsed < CONTROL_WRITE_TIMEOUT * 3,
        "a stalled write was not bounded: {elapsed:?}"
    );

    // The stalled peer is gone, and the healthy one still gets control.
    let deadline = Instant::now() + WAIT;
    loop {
        match next_host_event(&host) {
            QuicHostEvent::Closed { peer, .. } if peer == stalled_peer => break,
            _ => assert!(
                Instant::now() < deadline,
                "the stalled peer was never closed"
            ),
        }
    }
    assert_eq!(
        host.send_control(stalled_peer, b"late".to_vec()),
        Err(QuicHostError::UnknownPeer(stalled_peer))
    );
    host.send_control(healthy_peer, b"stop".to_vec())
        .expect("healthy listener still served");
    let deadline = Instant::now() + WAIT;
    loop {
        match healthy
            .recv_event(Duration::from_millis(50))
            .expect("listener running")
        {
            Some(QuicListenerEvent::Control(frame)) if frame == b"stop" => break,
            Some(other) => panic!("unexpected listener event {other:?}"),
            None => assert!(
                Instant::now() < deadline,
                "the healthy listener got no control"
            ),
        }
    }
}
