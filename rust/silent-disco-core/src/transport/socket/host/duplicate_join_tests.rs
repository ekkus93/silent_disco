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
            ListenerTransportConfig::loopback(
                session.clone(),
                device.clone(),
                host.endpoint(),
            ),
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
            ListenerTransportConfig::loopback(
                session.clone(),
                device.clone(),
                host.endpoint(),
            ),
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
    replacement.send_control(&join).expect("reconnected duplicate");
    assert_no_repeated_join(&mut host);

    first.shutdown().expect("close retired TCP");
    replacement.shutdown().expect("close current TCP");
    host.shutdown().expect("close host");
}
