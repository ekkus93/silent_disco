//! The v4 wire itself (remediation §1 negative tests): a UDP relay between a
//! real listener worker and host worker tampers with or duplicates every
//! host-to-listener packet. QUIC's packet protection must drop a modified
//! packet (bad AEAD tag) and a repeated one, so the listener never delivers
//! an altered or duplicated control frame or datagram.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_host_worker::{QuicHostConfig, QuicHostEvent, QuicHostWorker};
use super::quic_listener_worker::{QuicListenerConfig, QuicListenerEvent, QuicListenerWorker};
use super::secure_control_tls::{HostTlsIdentity, host_tls_identity_from_pkcs8};
use crate::domain::SessionId;

const WAIT: Duration = Duration::from_secs(10);
const GENERATION: u64 = 4;
const PASS: u8 = 0;
const TAMPER: u8 = 1;
const DUPLICATE: u8 = 2;

fn identity() -> HostTlsIdentity {
    let key = SigningKey::from_slice(&[9; 32]).expect("fixture key");
    host_tls_identity_from_pkcs8(key.to_pkcs8_der().expect("pkcs8").as_bytes()).expect("identity")
}

fn session() -> SessionId {
    SessionId::new("session-on-wire").expect("session id")
}

/// Forwards UDP between one listener and the host, applying `mode` to every
/// host-to-listener packet. Stops when dropped.
struct Relay {
    address: SocketAddr,
    mode: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Relay {
    fn start(host: SocketAddr, mode: u8) -> Self {
        let front = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("relay front");
        let back = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("relay back");
        back.connect(host).expect("relay to host");
        for socket in [&front, &back] {
            socket
                .set_read_timeout(Some(Duration::from_millis(20)))
                .expect("timeout");
        }
        let address = front.local_addr().expect("relay address");
        let mode = Arc::new(AtomicU8::new(mode));
        let stop = Arc::new(AtomicBool::new(false));
        let listener: Arc<std::sync::Mutex<Option<SocketAddr>>> = Arc::default();
        let (front, back) = (Arc::new(front), Arc::new(back));
        let upstream = {
            let (front, back, stop, listener) = (
                Arc::clone(&front),
                Arc::clone(&back),
                Arc::clone(&stop),
                Arc::clone(&listener),
            );
            std::thread::spawn(move || {
                let mut buffer = vec![0_u8; 65_536];
                while !stop.load(Ordering::Acquire) {
                    if let Ok((length, from)) = front.recv_from(&mut buffer) {
                        *listener.lock().expect("listener address") = Some(from);
                        let _ = back.send(&buffer[..length]);
                    }
                }
            })
        };
        let downstream = {
            let (mode, stop) = (Arc::clone(&mode), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut buffer = vec![0_u8; 65_536];
                while !stop.load(Ordering::Acquire) {
                    let Ok(length) = back.recv(&mut buffer) else {
                        continue;
                    };
                    let Some(to) = *listener.lock().expect("listener address") else {
                        continue;
                    };
                    let packet = &mut buffer[..length];
                    match mode.load(Ordering::Acquire) {
                        TAMPER => {
                            // The last byte is inside the AEAD tag.
                            packet[length - 1] ^= 0x5a;
                            let _ = front.send_to(packet, to);
                        }
                        DUPLICATE => {
                            let _ = front.send_to(packet, to);
                            let _ = front.send_to(packet, to);
                        }
                        _ => {
                            let _ = front.send_to(packet, to);
                        }
                    }
                }
            })
        };
        Self {
            address,
            mode,
            stop,
            threads: vec![upstream, downstream],
        }
    }

    fn set(&self, mode: u8) {
        self.mode.store(mode, Ordering::Release);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn start_host(identity: &HostTlsIdentity) -> QuicHostWorker {
    QuicHostWorker::start(
        QuicHostConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            session: session(),
            generation: GENERATION,
            max_peers: 4,
            event_capacity: 64,
        },
        identity,
    )
    .expect("host starts")
}

fn connect_through(relay: &Relay, identity: &HostTlsIdentity) -> QuicListenerWorker {
    QuicListenerWorker::connect(
        QuicListenerConfig {
            host: relay.address,
            host_spki: identity.spki.clone(),
            session: session(),
            generation: GENERATION,
            event_capacity: 64,
        },
        Arc::new(SigningKey::from_slice(&[61; 32]).expect("listener key")),
    )
    .expect("listener admitted through the relay")
}

fn admitted(host: &QuicHostWorker) -> u64 {
    let deadline = Instant::now() + WAIT;
    loop {
        match host
            .recv_event(Duration::from_millis(50))
            .expect("host running")
        {
            Some(QuicHostEvent::Admitted { peer, .. }) => return peer,
            Some(_) => {}
            None => assert!(Instant::now() < deadline, "no admission in time"),
        }
    }
}

/// Every listener event until `quiet` passes with none, or `limit` elapses.
fn drain(
    listener: &QuicListenerWorker,
    quiet: Duration,
    limit: Duration,
) -> Vec<QuicListenerEvent> {
    let deadline = Instant::now() + limit;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match listener.recv_event(quiet) {
            Ok(Some(event)) => events.push(event),
            Ok(None) | Err(_) => break,
        }
    }
    events
}

#[test]
fn a_tampered_packet_is_never_delivered_and_the_connection_ends() {
    let identity = identity();
    let host = start_host(&identity);
    let relay = Relay::start(host.local_addr(), PASS);
    let listener = connect_through(&relay, &identity);
    let peer = admitted(&host);
    host.send_control(peer, b"before".to_vec())
        .expect("clean control");
    assert!(matches!(
        drain(&listener, Duration::from_millis(500), WAIT).first(),
        Some(QuicListenerEvent::Control(frame)) if frame == b"before"
    ));

    relay.set(TAMPER);
    let _ = host.send_control(peer, b"tampered control".to_vec());
    let _ = host.send_datagram(peer, b"tampered datagram".to_vec());

    // Nothing but the end of the connection may reach the listener: every
    // host packet now fails authentication, keep-alives included, so the
    // listener's idle timeout (8 s) ends it.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(Some(QuicListenerEvent::Closed(_))) | Err(_) => break,
            Ok(Some(event)) => panic!("a tampered packet was delivered: {event:?}"),
            Ok(None) => assert!(
                Instant::now() < deadline,
                "the tampered connection never ended"
            ),
        }
    }
}

#[test]
fn a_duplicated_packet_is_delivered_once() {
    let identity = identity();
    let host = start_host(&identity);
    let relay = Relay::start(host.local_addr(), PASS);
    let listener = connect_through(&relay, &identity);
    let peer = admitted(&host);

    relay.set(DUPLICATE);
    let frames: Vec<Vec<u8>> = (0..5_u8).map(|n| vec![b'c', n]).collect();
    let datagrams: Vec<Vec<u8>> = (0..5_u8).map(|n| vec![b'd', n]).collect();
    for (frame, datagram) in frames.iter().zip(&datagrams) {
        host.send_control(peer, frame.clone()).expect("control");
        host.send_datagram(peer, datagram.clone())
            .expect("datagram");
    }

    let events = drain(&listener, Duration::from_secs(1), WAIT);
    let controls: Vec<&Vec<u8>> = events
        .iter()
        .filter_map(|event| match event {
            QuicListenerEvent::Control(frame) => Some(frame),
            _ => None,
        })
        .collect();
    let received: Vec<&Vec<u8>> = events
        .iter()
        .filter_map(|event| match event {
            QuicListenerEvent::Datagram(datagram) => Some(datagram),
            _ => None,
        })
        .collect();
    assert_eq!(
        controls,
        frames.iter().collect::<Vec<_>>(),
        "control frames once each, in order"
    );
    for datagram in &datagrams {
        let copies = received.iter().filter(|got| **got == datagram).count();
        assert!(
            copies <= 1,
            "datagram {datagram:?} delivered {copies} times"
        );
    }
    assert!(
        !received.is_empty(),
        "no datagram arrived at all, so duplicate suppression was not exercised"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, QuicListenerEvent::Closed(_))),
        "duplicated packets ended the connection"
    );
}
