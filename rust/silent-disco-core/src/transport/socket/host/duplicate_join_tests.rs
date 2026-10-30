//! One accepted join per TCP generation, including retransmissions before
//! and after approval. A new TCP connection may submit a fresh join.
//! This does not authenticate a claimed device ID (legacy v3 remains insecure).
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::protocol::{ControlMessage, ProtocolFrame};
use crate::transport::test_support::{
    id_device, id_session, join_request, wait_for_authorized, wait_for_control_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportErrorKind, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

fn assert_no_repeated_join(host: &mut SocketHostTransport) {
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match host.recv_event(remaining) {
            Ok(TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
                ..
            }) => panic!("a duplicate TCP join reached the host actor"),
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => panic!("unexpected transport event error: {error}"),
        }
    }
}

#[test]
fn duplicate_join_frames_never_reenroll_but_new_tcp_generation_can_join() {
    let session = id_session("duplicate-join-generation");
    let device = id_device("duplicate-join-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP host");
    let factory = SocketTransportFactory;
    let mut first = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("first TCP generation");
    let join = join_request(&session, &device, "First generation");
    first.send_control(&join).expect("initial join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    for _ in 0..4 {
        first.send_control(&join).expect("pending retry");
    }
    assert_no_repeated_join(&mut host);
    host.authorize_peer(&device, first.local_routes())
        .expect("authorize initial TCP generation");
    wait_for_authorized(&mut host, &device);
    for _ in 0..4 {
        first.send_control(&join).expect("authorized retry");
    }
    assert_no_repeated_join(&mut host);

    // A replaced TCP connection has its own admission generation; suppressing
    // duplicates must not suppress a legitimate reconnect request.
    let mut replacement = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("replacement TCP generation");
    replacement
        .send_control(&join_request(&session, &device, "Replacement generation"))
        .expect("reconnect join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, replacement.local_routes())
        .expect("authorize replacement");
    wait_for_authorized(&mut host, &device);
    replacement
        .send_control(&join)
        .expect("reconnected duplicate");
    assert_no_repeated_join(&mut host);

    first.shutdown().expect("close retired TCP");
    replacement.shutdown().expect("close current TCP");
    host.shutdown().expect("close host");
}

#[test]
fn simultaneous_pending_same_id_connections_cannot_steal_approval() {
    use crate::transport::test_support::wait_for_host_event;

    let session = id_session("ambiguous-pending-admission");
    let device = id_device("shared-claimed-id");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production host");
    let factory = SocketTransportFactory;
    let mut first = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("first pending connection");
    first
        .send_control(&join_request(&session, &device, "Original pending join"))
        .expect("first join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    let mut second = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("second pending connection");
    second
        .send_control(&join_request(&session, &device, "Conflicting pending join"))
        .expect("second join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    // The host may have approved the first request, but the old
    // device-ID-only lookup would authorize the newest TCP generation.
    let error = host
        .authorize_peer(&device, second.local_routes())
        .expect_err("ambiguous claimed identity must not authorize either peer");
    assert_eq!(error.kind, TransportErrorKind::Unauthorized);
    assert!(
        host.routes
            .lock()
            .expect("route registry")
            .get(&device)
            .is_none(),
        "failed admission must not publish a UDP route"
    );

    // Once the first pending generation actually disconnects, a fresh
    // decision may authorize the remaining generation without poisoning it.
    first.shutdown().expect("retire first pending connection");
    drop(wait_for_host_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerDisconnected { peer, .. }
            if peer.device_id.as_ref() == Some(&device))
    }));
    host.authorize_peer(&device, second.local_routes())
        .expect("unambiguous remaining generation can be authorized");
    wait_for_authorized(&mut host, &device);
    second.shutdown().expect("retire second connection");
    host.shutdown().expect("host shutdown");
}
