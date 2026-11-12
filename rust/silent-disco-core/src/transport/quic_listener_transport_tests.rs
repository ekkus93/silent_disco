//! `QuicListenerTransport` against `QuicHostTransport` over real loopback
//! QUIC, both driven only through their transport traits.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_host_transport::QuicHostTransport;
use super::quic_host_worker::{QuicHostConfig, QuicHostEvent, QuicHostWorker};
use super::quic_listener_transport::QuicListenerTransport;
use super::quic_listener_worker::QUIC_CONNECT_TIMEOUT;
use super::secure_admission::SECURE_ADMISSION_GENERATION;
use super::secure_control_tls::{HostTlsIdentity, host_tls_identity_from_pkcs8};
use super::secure_identity::device_id_for_listener_key;
use super::secure_listener_proof::ListenerProofSigner;
use super::test_support::{audio_frame, id_device, id_session, stream_start, wait_for_host_event};
use super::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    SecureHostTarget, SystemTransportClock, TransportChannel, TransportErrorKind, TransportEvent,
};
use crate::domain::{DeviceId, MonotonicMillis, SessionId};
use crate::protocol::{
    ControlMessage, DeviceIdentity, Disconnect, Heartbeat, Hello, JoinApproval, JoinRequest,
    ProtocolFrame, SyncRequest, SyncResponse, encode_frame,
};
use crate::runtime::NetworkEndpoint;

const WAIT: Duration = Duration::from_secs(10);

pub(super) fn session() -> SessionId {
    id_session("session-quic-listener")
}

pub(super) fn identity() -> Arc<HostTlsIdentity> {
    let key = SigningKey::from_slice(&[12; 32]).expect("fixture key");
    Arc::new(
        host_tls_identity_from_pkcs8(key.to_pkcs8_der().expect("pkcs8").as_bytes())
            .expect("identity"),
    )
}

pub(super) fn host(identity: &Arc<HostTlsIdentity>) -> QuicHostTransport {
    let mut config = HostTransportConfig::loopback(session());
    config.secure_control = Some(Arc::clone(identity));
    QuicHostTransport::bind(config, clock()).expect("QUIC host binds")
}

pub(super) fn clock() -> Arc<SystemTransportClock> {
    Arc::new(SystemTransportClock::starting_at(1_000))
}

pub(super) fn listener_config(
    endpoint: NetworkEndpoint,
    identity: &HostTlsIdentity,
    seed: u8,
) -> ListenerTransportConfig {
    let key = SigningKey::from_slice(&[seed; 32]).expect("listener key");
    let device = device_id_for_listener_key(&key.spki_der().expect("spki"));
    let mut config = ListenerTransportConfig::loopback(session(), device, endpoint);
    config.secure_host = Some(SecureHostTarget {
        host_spki: identity.spki.clone(),
        signer: Arc::new(key),
    });
    config
}

pub(super) fn connect(
    endpoint: NetworkEndpoint,
    identity: &HostTlsIdentity,
    seed: u8,
) -> QuicListenerTransport {
    QuicListenerTransport::connect(listener_config(endpoint, identity, seed), clock())
        .expect("listener connects")
}

pub(super) fn device(listener_seed: u8) -> DeviceId {
    let key = SigningKey::from_slice(&[listener_seed; 32]).expect("listener key");
    device_id_for_listener_key(&key.spki_der().expect("spki"))
}

pub(super) fn join(listener: &QuicListenerTransport, device: &DeviceId) {
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session(),
            device: DeviceIdentity {
                device_id: device.clone(),
                display_name: "listener".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("join sent");
}

pub(super) fn next(listener: &QuicListenerTransport) -> TransportEvent {
    listener.recv_event(WAIT).expect("listener event")
}

pub(super) fn next_control(listener: &QuicListenerTransport) -> ControlMessage {
    match next(listener) {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(message),
            ..
        } => message,
        _ => panic!("expected a host control frame"),
    }
}

/// Joins, approves and authorizes one listener; returns it ready for playback.
pub(super) fn admitted(
    host: &mut QuicHostTransport,
    identity: &HostTlsIdentity,
    seed: u8,
) -> (QuicListenerTransport, DeviceId) {
    let listener = connect(host.endpoint(), identity, seed);
    let device = device(seed);
    wait_for_host_event(host, |e| matches!(e, TransportEvent::PeerAccepted { .. }));
    join(&listener, &device);
    let join = wait_for_host_event(host, |e| {
        matches!(
            e,
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(_),
                ..
            }
        )
    });
    let TransportEvent::FrameReceived {
        frame: ProtocolFrame::Control(ControlMessage::JoinRequest(request)),
        ..
    } = join
    else {
        panic!("expected the join");
    };
    host.send_pending_control(
        &device,
        &ControlMessage::JoinApproval(JoinApproval {
            session_id: session(),
            listener_id: device.clone(),
            trusted_for_future: false,
        }),
    )
    .expect("approval");
    host.authorize_peer_ports(&device, request.sync_port, request.audio_port)
        .expect("authorized with the ports the listener reported");
    wait_for_host_event(host, |e| matches!(e, TransportEvent::PeerAuthorized { .. }));
    assert!(matches!(
        next_control(&listener),
        ControlMessage::JoinApproval(_)
    ));
    (listener, device)
}

#[test]
fn a_v4_listener_requires_a_pinned_host_and_a_listener_key() {
    let identity = identity();
    let host = host(&identity);
    let mut config = listener_config(host.endpoint(), &identity, 41);
    config.secure_host = None;
    let error = QuicListenerTransport::connect(config, clock()).expect_err("no v4 without keys");
    assert_eq!(error.kind, TransportErrorKind::InvalidConfiguration);
}

#[test]
fn a_listener_joins_and_receives_controls_audio_and_clock_replies() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = connect(host.endpoint(), &identity, 42);
    let routes = listener.local_routes();
    assert_ne!(routes.audio.port(), 0, "the join reports real ports");
    assert_eq!(
        routes.audio, routes.synchronization,
        "one QUIC connection carries both"
    );

    let device = device(42);
    wait_for_host_event(&mut host, |e| {
        matches!(e, TransportEvent::PeerAccepted { .. })
    });
    join(&listener, &device);
    wait_for_host_event(&mut host, |e| {
        matches!(e, TransportEvent::FrameReceived { .. })
    });
    host.send_pending_control(
        &device,
        &ControlMessage::Hello(Hello {
            session_id: session(),
            session_name: "Party".to_owned(),
            host_name: "Host".to_owned(),
            approval_required: true,
        }),
    )
    .expect("hello");
    assert!(matches!(next_control(&listener), ControlMessage::Hello(_)));
    drop(listener);

    let (listener, device) = admitted(&mut host, &identity, 43);
    host.broadcast_control(&stream_start(&session(), 2_000))
        .expect("start");
    assert!(matches!(
        next_control(&listener),
        ControlMessage::StreamStart(_)
    ));

    let audio = audio_frame(&session(), 3);
    host.broadcast_audio(&audio).expect("audio");
    match next(&listener) {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            frame,
            ..
        } => assert!(frame == audio),
        _ => panic!("expected audio on the audio channel"),
    }

    let request = SyncRequest {
        session_id: session(),
        correlation_id: 9,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(10),
    };
    listener.send_sync_request(&request).expect("probe");
    wait_for_host_event(&mut host, |e| {
        matches!(
            e,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                ..
            }
        )
    });
    let reply = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session(),
        correlation_id: 9,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(10),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(1_000),
        t3_host_send_elapsed_ms: MonotonicMillis::new(1_001),
    });
    host.send_sync_to_peer(&device, &reply).expect("reply");
    match next(&listener) {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization,
            frame,
            ..
        } => assert!(frame == reply),
        _ => panic!("expected the clock reply on the synchronization channel"),
    }
    let counters = listener.counters();
    assert_eq!(counters.audio_datagrams_received, 1);
    assert_eq!(counters.sync_datagrams_sent, 1);
    assert_eq!(counters.sync_datagrams_received, 1);
}

#[test]
fn a_clock_probe_is_not_held_up_by_a_blocked_receive() {
    let identity = identity();
    let mut host = host(&identity);
    let (listener, _) = admitted(&mut host, &identity, 44);
    let listener = Arc::new(listener);
    let receiver = Arc::clone(&listener);
    let blocked = thread::spawn(move || receiver.recv_event(Duration::from_secs(3)));
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    listener
        .send_sync_request(&SyncRequest {
            session_id: session(),
            correlation_id: 1,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1),
        })
        .expect("probe");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the probe waited {:?} behind the receive",
        started.elapsed()
    );
    drop(blocked.join());
}

#[test]
fn a_listener_sends_only_its_own_listener_controls() {
    let identity = identity();
    let host = host(&identity);
    let listener = connect(host.endpoint(), &identity, 45);
    let as_someone_else = ControlMessage::Heartbeat(Heartbeat {
        session_id: session(),
        listener_id: id_device("someone-else"),
        sent_at_elapsed_ms: MonotonicMillis::new(1),
    });
    let host_only = stream_start(&session(), 1);
    for message in [as_someone_else, host_only] {
        let error = listener
            .send_control(&message)
            .expect_err("refused locally");
        assert_eq!(error.kind, TransportErrorKind::Unauthorized);
    }
    assert_eq!(listener.counters().control_frames_sent, 0);
}

#[test]
fn a_leaving_listener_disconnect_reaches_the_host_before_the_close() {
    let identity = identity();
    let mut host = host(&identity);
    let (mut listener, device) = admitted(&mut host, &identity, 46);
    listener
        .send_control(&ControlMessage::Disconnect(Disconnect {
            session_id: session(),
            listener_id: device.clone(),
            reason: "going home".to_owned(),
        }))
        .expect("disconnect sent");
    listener.shutdown().expect("shut down");
    wait_for_host_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::Disconnect(_)),
                ..
            }
        )
    });
    wait_for_host_event(&mut host, |e| {
        matches!(e, TransportEvent::PeerDisconnected { .. })
    });
    let error = listener
        .send_sync_request(&SyncRequest {
            session_id: session(),
            correlation_id: 2,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(2),
        })
        .expect_err("nothing after shutdown");
    assert_eq!(error.kind, TransportErrorKind::ShuttingDown);
}

#[test]
fn a_kicked_listener_sees_the_disconnect_then_a_failed_session() {
    let identity = identity();
    let mut host = host(&identity);
    let (listener, device) = admitted(&mut host, &identity, 47);
    host.send_disconnect(
        &device,
        &ControlMessage::Disconnect(Disconnect {
            session_id: session(),
            listener_id: device.clone(),
            reason: "kicked".to_owned(),
        }),
    )
    .expect("kicked");
    assert!(matches!(
        next_control(&listener),
        ControlMessage::Disconnect(_)
    ));
    match next(&listener) {
        TransportEvent::PeerDisconnected { error, .. } => {
            assert!(error.is_some(), "the host ended the session");
        }
        _ => panic!("expected the disconnect"),
    }
    let late = listener
        .send_control(&ControlMessage::Heartbeat(Heartbeat {
            session_id: session(),
            listener_id: device,
            sent_at_elapsed_ms: MonotonicMillis::new(3),
        }))
        .expect_err("retired after the host closed");
    assert_eq!(late.kind, TransportErrorKind::ShuttingDown);
}

#[test]
fn a_host_sending_an_invalid_control_is_disconnected_with_the_reason() {
    let identity = identity();
    // A raw worker can send what the host transport never would.
    let raw = QuicHostWorker::start(
        QuicHostConfig {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            session: session(),
            generation: SECURE_ADMISSION_GENERATION,
            max_peers: 4,
            event_capacity: 16,
        },
        &identity,
    )
    .expect("raw host");
    let endpoint = NetworkEndpoint {
        address: Ipv4Addr::LOCALHOST.into(),
        control_port: raw.local_addr().port(),
        sync_port: raw.local_addr().port(),
        audio_port: raw.local_addr().port(),
    };
    let listener = connect(endpoint, &identity, 48);
    let peer = loop {
        match raw.recv_event(WAIT).expect("raw host running") {
            Some(QuicHostEvent::Admitted { peer, .. }) => break peer,
            Some(_) => {}
            None => panic!("no admission"),
        }
    };
    // An approval addressed to a different listener.
    let misaddressed = ControlMessage::JoinApproval(JoinApproval {
        session_id: session(),
        listener_id: id_device("someone-else"),
        trusted_for_future: true,
    });
    raw.send_control(
        peer,
        encode_frame(&ProtocolFrame::Control(misaddressed)).expect("encodes"),
    )
    .expect("sent");
    match next(&listener) {
        TransportEvent::Rejected { error, .. } => {
            assert_eq!(error.kind, TransportErrorKind::Unauthorized);
        }
        _ => panic!("expected the rejection"),
    }
    match next(&listener) {
        TransportEvent::PeerDisconnected { error, .. } => {
            assert_eq!(
                error.map(|e| e.kind),
                Some(TransportErrorKind::Unauthorized),
                "the disconnect carries the reason"
            );
        }
        _ => panic!("expected the disconnect"),
    }
    assert_eq!(listener.counters().unauthorized_frames, 1);
}

#[test]
fn a_v4_listener_gives_up_on_a_silent_endpoint_within_the_connect_timeout() {
    let identity = identity();
    // A UDP port with nothing listening: no QUIC answer, and quinn ignores
    // the ICMP refusal, so only the connect timeout ends the attempt.
    let dead = std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("probe")
        .local_addr()
        .expect("address")
        .port();
    let endpoint = NetworkEndpoint {
        address: Ipv4Addr::LOCALHOST.into(),
        control_port: dead,
        sync_port: dead,
        audio_port: dead,
    };
    let started = Instant::now();
    let error = QuicListenerTransport::connect(listener_config(endpoint, &identity, 49), clock())
        .expect_err("nothing answers");
    let waited = started.elapsed();
    assert_eq!(error.kind, TransportErrorKind::Connect);
    assert!(
        error.message.contains("no QUIC answer"),
        "{}",
        error.message
    );
    assert!(
        waited >= QUIC_CONNECT_TIMEOUT && waited < QUIC_CONNECT_TIMEOUT + Duration::from_secs(1),
        "gave up after {waited:?}"
    );
}
