//! Real loopback QUIC through the host and listener workers.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_framing::MAX_QUIC_CONTROL_FRAME_BYTES;
use super::quic_host_worker::{QuicHostConfig, QuicHostError, QuicHostEvent, QuicHostWorker};
use super::quic_listener_worker::{QuicListenerConfig, QuicListenerEvent, QuicListenerWorker};
use super::secure_control_tls::{HostTlsIdentity, host_tls_identity_from_pkcs8};
use super::secure_identity::device_id_for_listener_key;
use super::secure_listener_proof::ListenerProofSigner;
use crate::domain::SessionId;

const WAIT: Duration = Duration::from_secs(10);
const GENERATION: u64 = 4;
/// One encoded 5 ms audio frame is 960 PCM bytes plus a small header.
const AUDIO_SIZED: usize = 1_000;

fn identity(seed: u8) -> HostTlsIdentity {
    let key = SigningKey::from_slice(&[seed; 32]).expect("fixture key");
    host_tls_identity_from_pkcs8(key.to_pkcs8_der().expect("pkcs8").as_bytes()).expect("identity")
}

fn session() -> SessionId {
    SessionId::new("session-workers").expect("session id")
}

fn host(identity: &HostTlsIdentity, max_peers: usize) -> QuicHostWorker {
    QuicHostWorker::start(
        QuicHostConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            session: session(),
            generation: GENERATION,
            max_peers,
            event_capacity: 64,
        },
        identity,
    )
    .expect("host starts")
}

fn listener(
    host: &QuicHostWorker,
    pinned: &[u8],
    key_seed: u8,
) -> Result<QuicListenerWorker, super::quic_listener_worker::QuicListenerError> {
    QuicListenerWorker::connect(
        QuicListenerConfig {
            host: host.local_addr(),
            host_spki: pinned.to_vec(),
            session: session(),
            generation: GENERATION,
            event_capacity: 64,
        },
        Arc::new(SigningKey::from_slice(&[key_seed; 32]).expect("listener key")),
    )
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

fn next_listener_event(listener: &QuicListenerWorker) -> QuicListenerEvent {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(event) = listener
            .recv_event(Duration::from_millis(50))
            .expect("listener running")
        {
            return event;
        }
        assert!(Instant::now() < deadline, "no listener event in time");
    }
}

fn admitted(host: &QuicHostWorker) -> u64 {
    match next_host_event(host) {
        QuicHostEvent::Admitted { peer, .. } => peer,
        other => panic!("expected an admission, got {other:?}"),
    }
}

#[test]
fn an_admitted_listener_exchanges_control_frames_and_datagrams_both_ways() {
    let identity = identity(9);
    let host = host(&identity, 8);
    let listener_key = SigningKey::from_slice(&[31; 32]).expect("key");
    let listener = listener(&host, &identity.spki, 31).expect("listener admitted");

    let peer = match next_host_event(&host) {
        QuicHostEvent::Admitted {
            peer,
            listener: verified,
            ..
        } => {
            assert_eq!(
                verified.canonical_device_id(),
                device_id_for_listener_key(&listener_key.spki_der().expect("spki"))
            );
            peer
        }
        other => panic!("expected an admission, got {other:?}"),
    };
    assert!(
        listener
            .max_datagram_size()
            .is_some_and(|size| size >= AUDIO_SIZED),
        "an audio-sized datagram does not fit: {:?}",
        listener.max_datagram_size()
    );

    // Control, in order, both ways.
    listener.send_control(b"join".to_vec()).expect("join");
    listener.send_control(b"report".to_vec()).expect("report");
    for expected in [&b"join"[..], b"report"] {
        match next_host_event(&host) {
            QuicHostEvent::Control { peer: from, frame } => {
                assert_eq!(from, peer);
                assert_eq!(frame, expected);
            }
            other => panic!("expected a control frame, got {other:?}"),
        }
    }
    host.send_control(peer, b"approved".to_vec())
        .expect("approval");
    match next_listener_event(&listener) {
        QuicListenerEvent::Control(frame) => assert_eq!(frame, b"approved"),
        other => panic!("expected a control frame, got {other:?}"),
    }

    // Datagrams, both ways, at audio size.
    let audio = vec![0x5a_u8; AUDIO_SIZED];
    host.send_datagram(peer, audio.clone())
        .expect("audio datagram");
    match next_listener_event(&listener) {
        QuicListenerEvent::Datagram(datagram) => assert_eq!(datagram, audio),
        other => panic!("expected a datagram, got {other:?}"),
    }
    listener
        .send_datagram(b"sync probe".to_vec())
        .expect("probe");
    match next_host_event(&host) {
        QuicHostEvent::Datagram {
            peer: from,
            datagram,
        } => {
            assert_eq!(from, peer);
            assert_eq!(datagram, b"sync probe");
        }
        other => panic!("expected a datagram, got {other:?}"),
    }
}

#[test]
fn a_broadcast_reaches_each_listener_on_its_own_connection() {
    let identity = identity(9);
    let host = host(&identity, 8);
    let first = listener(&host, &identity.spki, 31).expect("first");
    let second = listener(&host, &identity.spki, 32).expect("second");
    let peers = [admitted(&host), admitted(&host)];
    assert_ne!(peers[0], peers[1]);

    let results = host
        .broadcast_datagram(vec![7_u8; AUDIO_SIZED])
        .expect("broadcast");
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|(_, result)| result.is_ok()),
        "{results:?}"
    );
    for listener in [&first, &second] {
        match next_listener_event(listener) {
            QuicListenerEvent::Datagram(datagram) => assert_eq!(datagram, vec![7_u8; AUDIO_SIZED]),
            other => panic!("expected the broadcast, got {other:?}"),
        }
    }
}

#[test]
fn a_listener_pinned_to_another_key_cannot_connect() {
    let identity = identity(9);
    let host = host(&identity, 8);
    let outcome = listener(&host, &self::identity(10).spki, 31);
    assert!(outcome.is_err(), "connected to a host with an unpinned key");
}

#[test]
fn the_peer_limit_refuses_further_listeners_visibly() {
    let identity = identity(9);
    let host = host(&identity, 1);
    let _first = listener(&host, &identity.spki, 31).expect("first");
    let _ = admitted(&host);
    let second = listener(&host, &identity.spki, 32);
    assert!(
        second.is_err(),
        "a listener beyond the peer limit was admitted"
    );
    assert!(
        matches!(next_host_event(&host), QuicHostEvent::Refused { .. }),
        "the refusal was not reported"
    );
}

#[test]
fn a_departing_listener_is_reported_and_can_no_longer_be_addressed() {
    let identity = identity(9);
    let host = host(&identity, 8);
    let mut listener = listener(&host, &identity.spki, 31).expect("listener");
    let peer = admitted(&host);

    listener.shutdown();
    match next_host_event(&host) {
        QuicHostEvent::Closed { peer: closed, .. } => assert_eq!(closed, peer),
        other => panic!("expected the departure, got {other:?}"),
    }
    assert_eq!(
        host.send_control(peer, b"late".to_vec()),
        Err(QuicHostError::UnknownPeer(peer))
    );
}

#[test]
fn a_host_shutdown_is_reported_to_the_listener() {
    let identity = identity(9);
    let mut host = host(&identity, 8);
    let listener = listener(&host, &identity.spki, 31).expect("listener");
    let _ = admitted(&host);

    host.shutdown();
    loop {
        match next_listener_event(&listener) {
            QuicListenerEvent::Closed(_) => break,
            QuicListenerEvent::Control(_) | QuicListenerEvent::Datagram(_) => {}
        }
    }
}

/// A Stop sent just before the host shuts down must still reach the
/// listener: closing a QUIC connection discards unacknowledged stream data,
/// so shutdown waits for the queued frames first (handoff §48). Loopback
/// does not lose packets, so this guards the order of events, not the loss
/// case itself.
#[test]
fn control_frames_sent_just_before_host_shutdown_arrive_before_the_close() {
    let identity = identity(9);
    let mut host = host(&identity, 8);
    let listener = listener(&host, &identity.spki, 31).expect("listener");
    let peer = admitted(&host);

    for index in 0..50_u8 {
        host.send_control(peer, vec![index; 1_000]).expect("queued");
    }
    host.shutdown();
    let mut received = 0_u8;
    loop {
        match next_listener_event(&listener) {
            QuicListenerEvent::Control(frame) => {
                assert_eq!(frame[0], received, "frames arrive in order");
                received += 1;
            }
            QuicListenerEvent::Closed(_) => break,
            QuicListenerEvent::Datagram(_) => {}
        }
    }
    assert_eq!(
        received, 50,
        "every frame sent before shutdown arrived first"
    );
}

#[test]
fn an_oversized_control_frame_is_refused_without_breaking_the_connection() {
    let identity = identity(9);
    let host = host(&identity, 8);
    let listener = listener(&host, &identity.spki, 31).expect("listener");
    let peer = admitted(&host);

    assert!(
        listener
            .send_control(vec![0_u8; MAX_QUIC_CONTROL_FRAME_BYTES + 1])
            .is_err(),
        "an oversized control frame was sent"
    );
    listener
        .send_control(b"still here".to_vec())
        .expect("next frame");
    match next_host_event(&host) {
        QuicHostEvent::Control { peer: from, frame } => {
            assert_eq!(from, peer);
            assert_eq!(frame, b"still here");
        }
        other => panic!("expected the next frame, got {other:?}"),
    }
}

/// Android run on the A54 (phase 4): with the host's event collector stalled,
/// a full event queue must never freeze the worker. A refusal at the peer
/// limit used to wait in the worker's main loop for room in the queue, so no
/// command (a Stop broadcast included) was handled again. Here nothing reads
/// host events: the one admission fills the queue, a second listener is
/// refused at the peer limit, and a command must still be answered.
#[test]
fn a_full_event_queue_never_stops_the_host_handling_commands() {
    let identity = identity(9);
    let host = QuicHostWorker::start(
        QuicHostConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            session: session(),
            generation: GENERATION,
            max_peers: 1,
            event_capacity: 1,
        },
        &identity,
    )
    .expect("host starts");
    let _first = listener(&host, &identity.spki, 31).expect("first listener admitted");
    // Its admission event now fills the queue; nothing reads it.
    assert!(
        listener(&host, &identity.spki, 32).is_err(),
        "a listener beyond the peer limit was admitted"
    );

    // A detached thread owns the host, so a frozen worker fails this test
    // instead of hanging it; it hands the host back once answered.
    let (done, answered) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reply = host.send_control(u64::MAX, b"probe".to_vec());
        let _ = done.send((reply, host));
    });
    let (reply, host) = answered
        .recv_timeout(Duration::from_secs(5))
        .expect("the host stopped handling commands while its event queue was full");
    assert_eq!(reply, Err(QuicHostError::UnknownPeer(u64::MAX)));
    assert_eq!(
        host.dropped_refusals(),
        1,
        "the unreported refusal is counted"
    );
}

/// Joins reported while the host's consumer was stalled still come out in
/// the order they arrived (A54 saturation run, phase 4).
#[test]
fn admissions_queued_behind_a_stalled_consumer_keep_their_arrival_order() {
    let identity = identity(9);
    let host = QuicHostWorker::start(
        QuicHostConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            session: session(),
            generation: GENERATION,
            max_peers: 64,
            event_capacity: 1,
        },
        &identity,
    )
    .expect("host starts");
    // Nothing reads host events while these join one after another.
    let listeners: Vec<_> = (0..24_u8)
        .map(|seed| listener(&host, &identity.spki, 40 + seed).expect("listener admitted"))
        .collect();

    let mut order = Vec::new();
    while order.len() < listeners.len() {
        if let QuicHostEvent::Admitted { peer, .. } = next_host_event(&host) {
            order.push(peer);
        }
    }
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(
        order, sorted,
        "admissions were reported out of arrival order"
    );
}
