//! `QuicHostTransport` driven through `HostTransportNode`, with real loopback
//! QUIC listeners (`QuicListenerWorker`) on the other end.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePrivateKey;

use super::quic_datagram::QUIC_IDLE_TIMEOUT;
use super::quic_host_transport::QuicHostTransport;
use super::quic_listener_worker::{QuicListenerConfig, QuicListenerEvent, QuicListenerWorker};
use super::secure_admission::SECURE_ADMISSION_GENERATION;
use super::secure_control_tls::{HostTlsIdentity, host_tls_identity_from_pkcs8};
use super::secure_identity::device_id_for_listener_key;
use super::secure_listener_proof::ListenerProofSigner;
use super::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, stream_stop,
    wait_for_host_event,
};
use super::{
    HostTransportConfig, HostTransportNode, SystemTransportClock, TransportChannel,
    TransportErrorKind, TransportEvent,
};
use crate::domain::{DeviceId, MonotonicMillis, SessionId};
use crate::protocol::{
    ControlMessage, Disconnect, Hello, JoinApproval, JoinRejection, ProtocolFrame, SyncRequest,
    SyncResponse, decode_frame, encode_frame,
};

const WAIT: Duration = Duration::from_secs(10);

fn session() -> SessionId {
    id_session("session-quic-host")
}

fn identity() -> Arc<HostTlsIdentity> {
    let key = SigningKey::from_slice(&[11; 32]).expect("fixture key");
    Arc::new(
        host_tls_identity_from_pkcs8(key.to_pkcs8_der().expect("pkcs8").as_bytes())
            .expect("identity"),
    )
}

fn host(identity: &Arc<HostTlsIdentity>) -> QuicHostTransport {
    let mut config = HostTransportConfig::loopback(session());
    config.secure_control = Some(Arc::clone(identity));
    QuicHostTransport::bind(config, Arc::new(SystemTransportClock::starting_at(1_000)))
        .expect("QUIC host binds")
}

struct Listener {
    worker: QuicListenerWorker,
    device: DeviceId,
}

fn connect(host: &QuicHostTransport, identity: &HostTlsIdentity, seed: u8) -> Listener {
    let key = SigningKey::from_slice(&[seed; 32]).expect("listener key");
    let device = device_id_for_listener_key(&key.spki_der().expect("spki"));
    let worker = QuicListenerWorker::connect(
        QuicListenerConfig {
            host: SocketAddr::from((Ipv4Addr::LOCALHOST, host.local_addr().port())),
            host_spki: identity.spki.clone(),
            session: session(),
            generation: SECURE_ADMISSION_GENERATION,
            event_capacity: 64,
        },
        Arc::new(key),
    )
    .expect("listener admitted");
    Listener { worker, device }
}

impl Listener {
    fn send(&self, message: &ControlMessage) {
        let bytes = encode_frame(&ProtocolFrame::Control(message.clone())).expect("encodes");
        self.worker.send_control(bytes).expect("listener sends");
    }

    fn join(&self) {
        self.send(&join_request(&session(), &self.device, "listener"));
    }

    fn next(&self) -> QuicListenerEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(event) = self
                .worker
                .recv_event(Duration::from_millis(50))
                .expect("listener running")
            {
                return event;
            }
            assert!(Instant::now() < deadline, "no listener event in time");
        }
    }

    fn next_control(&self) -> ControlMessage {
        match self.next() {
            QuicListenerEvent::Control(bytes) => match decode_frame(&bytes).expect("decodes") {
                ProtocolFrame::Control(message) => message,
                _ => panic!("expected a control message"),
            },
            other => panic!("expected a control frame, got {other:?}"),
        }
    }

    /// Asserts nothing arrives for a short while.
    fn assert_quiet(&self) {
        let event = self
            .worker
            .recv_event(Duration::from_millis(300))
            .expect("listener running");
        assert!(event.is_none(), "unexpected listener event {event:?}");
    }
}

fn hello() -> ControlMessage {
    ControlMessage::Hello(Hello {
        session_id: session(),
        session_name: "Party".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    })
}

fn approval(device: &DeviceId, trusted: bool) -> ControlMessage {
    ControlMessage::JoinApproval(JoinApproval {
        session_id: session(),
        listener_id: device.clone(),
        trusted_for_future: trusted,
    })
}

fn wait_accepted(host: &mut QuicHostTransport) -> SocketAddr {
    match wait_for_host_event(host, |e| matches!(e, TransportEvent::PeerAccepted { .. })) {
        TransportEvent::PeerAccepted { peer, .. } => {
            assert_eq!(
                peer.device_id, None,
                "identity is only known after the join"
            );
            peer.control_address
        }
        _ => unreachable!(),
    }
}

fn wait_join(host: &mut QuicHostTransport, device: &DeviceId) {
    wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                peer,
                frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
                ..
            } if peer.device_id.as_ref() == Some(device)
        )
    });
}

/// Connects, joins and approves one listener; returns it after approval.
fn admit(host: &mut QuicHostTransport, identity: &HostTlsIdentity, seed: u8) -> Listener {
    let listener = connect(host, identity, seed);
    wait_accepted(host);
    listener.join();
    wait_join(host, &listener.device);
    host.send_pending_control(&listener.device, &approval(&listener.device, false))
        .expect("approval sent");
    assert!(matches!(
        listener.next_control(),
        ControlMessage::JoinApproval(_)
    ));
    host.authorize_peer_ports(&listener.device, 1, 1)
        .expect("authorized");
    wait_for_host_event(host, |e| matches!(e, TransportEvent::PeerAuthorized { .. }));
    listener
}

fn wait_disconnected(host: &mut QuicHostTransport) -> TransportEvent {
    wait_for_host_event(host, |e| {
        matches!(e, TransportEvent::PeerDisconnected { .. })
    })
}

/// The listener probes over a datagram; the host replies to that peer only.
fn exchange_clock_probe(host: &mut QuicHostTransport, listener: &Listener) {
    let request = ProtocolFrame::SyncRequest(SyncRequest {
        session_id: session(),
        correlation_id: 7,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(5),
    });
    listener
        .worker
        .send_datagram(encode_frame(&request).expect("encodes"))
        .expect("probe sent");
    let event = wait_for_host_event(host, |e| {
        matches!(
            e,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                ..
            }
        )
    });
    let TransportEvent::FrameReceived { peer, frame, .. } = event else {
        unreachable!()
    };
    assert_eq!(peer.device_id.as_ref(), Some(&listener.device));
    assert!(frame == request);

    let response = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session(),
        correlation_id: 7,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(5),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(1_001),
        t3_host_send_elapsed_ms: MonotonicMillis::new(1_002),
    });
    host.send_sync_to_peer(&listener.device, &response)
        .expect("reply sent");
    match listener.next() {
        QuicListenerEvent::Datagram(bytes) => {
            assert!(decode_frame(&bytes).expect("decodes") == response);
        }
        other => panic!("expected the clock reply, got {other:?}"),
    }
}

#[test]
fn a_v4_host_requires_a_tls_identity() {
    let error = QuicHostTransport::bind(
        HostTransportConfig::loopback(session()),
        Arc::new(SystemTransportClock::starting_at(0)),
    )
    .expect_err("no identity, no v4 host");
    assert_eq!(error.kind, TransportErrorKind::InvalidConfiguration);
}

#[test]
fn a_listener_joins_is_approved_and_then_exchanges_playback_and_clock_frames() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = connect(&host, &identity, 21);
    wait_accepted(&mut host);
    listener.join();
    wait_join(&mut host, &listener.device);

    host.send_pending_control(&listener.device, &hello())
        .expect("hello to a pending peer");
    assert!(matches!(listener.next_control(), ControlMessage::Hello(_)));

    // Before approval: no playback controls and no audio.
    let early = host
        .broadcast_control(&stream_start(&session(), 2_000))
        .expect("cached");
    assert_eq!(early.report.intended_peers, 0);
    let audio = host
        .broadcast_audio(&audio_frame(&session(), 0))
        .expect("no recipients");
    assert_eq!(audio.report.intended_peers, 0);
    listener.assert_quiet();

    // v4 identities are proven, so durable trust may be granted.
    host.send_pending_control(&listener.device, &approval(&listener.device, true))
        .expect("trusted approval to a proven key");
    assert!(matches!(
        listener.next_control(),
        ControlMessage::JoinApproval(JoinApproval {
            trusted_for_future: true,
            ..
        })
    ));
    host.authorize_peer_ports(&listener.device, 1, 1)
        .expect("authorized");
    match wait_for_host_event(&mut host, |e| {
        matches!(e, TransportEvent::PeerAuthorized { .. })
    }) {
        TransportEvent::PeerAuthorized { peer, routes, .. } => {
            assert_eq!(peer.device_id.as_ref(), Some(&listener.device));
            assert_eq!(
                routes.audio, peer.control_address,
                "audio rides the QUIC connection"
            );
        }
        _ => unreachable!(),
    }
    // The cached StreamStart is replayed on authorization.
    assert!(matches!(
        listener.next_control(),
        ControlMessage::StreamStart(_)
    ));

    let frame = audio_frame(&session(), 1);
    let delivery = host.broadcast_audio(&frame).expect("audio sent");
    assert_eq!(
        (
            delivery.report.intended_peers,
            delivery.report.successful_peers
        ),
        (1, 1)
    );
    match listener.next() {
        QuicListenerEvent::Datagram(bytes) => {
            assert!(decode_frame(&bytes).expect("decodes") == frame);
        }
        other => panic!("expected audio, got {other:?}"),
    }

    exchange_clock_probe(&mut host, &listener);
    let counters = host.counters();
    assert_eq!(counters.audio_datagrams_sent, 1);
    assert_eq!(counters.sync_datagrams_received, 1);
}

#[test]
fn a_join_claiming_another_device_is_rejected_and_the_connection_closed() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = connect(&host, &identity, 22);
    wait_accepted(&mut host);
    listener.send(&join_request(
        &session(),
        &id_device("someone-else"),
        "impostor",
    ));
    match wait_for_host_event(&mut host, |e| matches!(e, TransportEvent::Rejected { .. })) {
        TransportEvent::Rejected { error, .. } => {
            assert_eq!(error.kind, TransportErrorKind::Unauthorized);
        }
        _ => unreachable!(),
    }
    wait_disconnected(&mut host);
    assert!(matches!(listener.next(), QuicListenerEvent::Closed(_)));
    assert_eq!(host.counters().unauthorized_frames, 1);
}

#[test]
fn a_pending_listener_gets_admission_controls_only() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = connect(&host, &identity, 23);
    wait_accepted(&mut host);
    listener.join();
    wait_join(&mut host, &listener.device);

    let refused = host
        .send_pending_control(&listener.device, &stream_start(&session(), 2_000))
        .expect_err("playback control to a pending peer");
    assert_eq!(refused.kind, TransportErrorKind::Unauthorized);
    let missing = host
        .send_control(&listener.device, &hello())
        .expect_err("authorized-only send to a pending peer");
    assert_eq!(missing.kind, TransportErrorKind::PeerNotFound);

    // A probe from an unapproved listener is refused, not forwarded.
    let probe = ProtocolFrame::SyncRequest(SyncRequest {
        session_id: session(),
        correlation_id: 1,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(1),
    });
    listener
        .worker
        .send_datagram(encode_frame(&probe).expect("encodes"))
        .expect("sent");
    match wait_for_host_event(&mut host, |e| matches!(e, TransportEvent::Rejected { .. })) {
        TransportEvent::Rejected { channel, error, .. } => {
            assert_eq!(channel, TransportChannel::Synchronization);
            assert_eq!(error.kind, TransportErrorKind::Unauthorized);
        }
        _ => unreachable!(),
    }
    listener.assert_quiet();
}

#[test]
fn a_repeated_join_on_one_connection_is_suppressed() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = connect(&host, &identity, 24);
    wait_accepted(&mut host);
    listener.join();
    listener.join();
    wait_join(&mut host, &listener.device);
    host.send_pending_control(&listener.device, &hello())
        .expect("hello");
    assert!(matches!(listener.next_control(), ControlMessage::Hello(_)));
    // The second join produced no event; the next one is a timeout.
    let next = host.recv_event(Duration::from_millis(300));
    assert_eq!(
        next.map(|_| ()).expect_err("no second join").kind,
        TransportErrorKind::Timeout
    );
    assert_eq!(host.counters().duplicate_join_suppressions, 1);
}

#[test]
fn a_listener_sending_a_host_only_control_is_disconnected() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = admit(&mut host, &identity, 25);
    listener.send(&stream_stop(&session(), 9));
    wait_for_host_event(&mut host, |event| {
        matches!(event, TransportEvent::Rejected { error, .. }
            if error.kind == TransportErrorKind::Unauthorized)
    });
    wait_disconnected(&mut host);
    let audio = host
        .broadcast_audio(&audio_frame(&session(), 0))
        .expect("no recipients");
    assert_eq!(audio.report.intended_peers, 0);
}

#[test]
fn a_refused_join_receives_its_rejection_before_the_connection_closes() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = connect(&host, &identity, 26);
    let address = wait_accepted(&mut host);
    listener.join();
    wait_join(&mut host, &listener.device);
    let rejection = ControlMessage::JoinRejection(JoinRejection {
        session_id: session(),
        listener_id: listener.device.clone(),
        reason: "not tonight".to_owned(),
    });
    host.reject_pending_peer_at(address, &listener.device, &rejection)
        .expect("rejection sent");
    assert!(matches!(
        listener.next_control(),
        ControlMessage::JoinRejection(JoinRejection { reason, .. }) if reason == "not tonight"
    ));
    assert!(matches!(listener.next(), QuicListenerEvent::Closed(_)));
    let late = host
        .authorize_peer_ports(&listener.device, 1, 1)
        .expect_err("a refused connection cannot be approved later");
    assert_eq!(late.kind, TransportErrorKind::PeerNotFound);
    match wait_disconnected(&mut host) {
        TransportEvent::PeerDisconnected { error, .. } => {
            assert_eq!(error, None, "the host closed it; not a peer failure");
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_kicked_listener_receives_its_disconnect_and_stops_receiving_audio() {
    let identity = identity();
    let mut host = host(&identity);
    let kicked = admit(&mut host, &identity, 27);
    let staying = admit(&mut host, &identity, 28);
    let disconnect = ControlMessage::Disconnect(Disconnect {
        session_id: session(),
        listener_id: kicked.device.clone(),
        reason: "bye".to_owned(),
    });
    // A listener-specific control cannot be broadcast or misaddressed.
    assert_eq!(
        host.broadcast_control(&disconnect)
            .expect_err("targeted only")
            .kind,
        TransportErrorKind::Unauthorized
    );
    assert_eq!(
        host.send_disconnect(&staying.device, &disconnect)
            .expect_err("addressed to someone else")
            .kind,
        TransportErrorKind::Unauthorized
    );
    host.send_disconnect(&kicked.device, &disconnect)
        .expect("kicked");
    assert!(matches!(
        kicked.next_control(),
        ControlMessage::Disconnect(_)
    ));
    assert!(matches!(kicked.next(), QuicListenerEvent::Closed(_)));

    let delivery = host
        .broadcast_audio(&audio_frame(&session(), 0))
        .expect("sent");
    assert_eq!(
        (
            delivery.report.intended_peers,
            delivery.report.successful_peers
        ),
        (1, 1)
    );
    assert!(matches!(staying.next(), QuicListenerEvent::Datagram(_)));
}

#[test]
fn a_late_joiner_gets_the_cached_start_before_any_live_control() {
    let identity = identity();
    let mut host = host(&identity);
    let early = admit(&mut host, &identity, 29);
    host.broadcast_control(&stream_start(&session(), 2_000))
        .expect("start");
    assert!(matches!(
        early.next_control(),
        ControlMessage::StreamStart(_)
    ));

    let late = connect(&host, &identity, 30);
    wait_accepted(&mut host);
    late.join();
    wait_join(&mut host, &late.device);
    host.send_pending_control(&late.device, &approval(&late.device, false))
        .expect("approval");
    host.authorize_peer_ports(&late.device, 1, 1)
        .expect("authorized");
    host.broadcast_control(&stream_stop(&session(), 3_000))
        .expect("stop");

    assert!(matches!(
        late.next_control(),
        ControlMessage::JoinApproval(_)
    ));
    assert!(matches!(
        late.next_control(),
        ControlMessage::StreamStart(_)
    ));
    assert!(matches!(late.next_control(), ControlMessage::Stop(_)));
    assert!(matches!(early.next_control(), ControlMessage::Stop(_)));
}

#[test]
fn an_approved_listener_stays_connected_while_waiting_longer_than_the_idle_timeout() {
    let identity = identity();
    let mut host = host(&identity);
    let listener = admit(&mut host, &identity, 31);
    // No application traffic at all; only QUIC keep-alives.
    let quiet = host.recv_event(QUIC_IDLE_TIMEOUT + Duration::from_secs(2));
    assert_eq!(
        quiet.map(|_| ()).expect_err("no event while idle").kind,
        TransportErrorKind::Timeout,
        "the idle listener must not be disconnected"
    );
    let delivery = host
        .broadcast_control(&stream_start(&session(), 2_000))
        .expect("start");
    assert_eq!(delivery.report.successful_peers, 1);
    assert!(matches!(
        listener.next_control(),
        ControlMessage::StreamStart(_)
    ));
}
