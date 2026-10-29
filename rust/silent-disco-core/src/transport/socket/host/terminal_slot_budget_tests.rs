//! Real-socket regression: stalled actor collectors must not create an
//! unbounded population of TCP readers waiting to report disconnect.
use std::io::{ErrorKind, Read};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::protocol::{ControlMessage, ProtocolFrame};
use crate::transport::test_support::{
    id_device, id_session, join_request, wait_for_authorized, wait_for_control_from,
    wait_for_frame, wait_until,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn terminal_backpressure_keeps_peer_budget_until_actor_drains_disconnect() {
    let session = id_session("terminal-peer-slot-budget");
    let first_id = id_device("budget-first");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.max_peers = 1;
    config.event_queue_capacity = 1;
    config.operation_timeout = Duration::from_millis(350);
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let mut first = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), first_id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("first real TCP listener");
    first
        .send_control(&join_request(&session, &first_id, "First"))
        .expect("send first join");
    // PeerAccepted occupies the actor's only slot. The reader refuses the
    // undeliverable join and closes its TCP socket, then must wait to publish
    // the authoritative terminal event without surrendering its peer budget.
    wait_for_frame(first.as_mut(), TransportChannel::Control, |frame| {
        matches!(
            frame,
            ProtocolFrame::Control(ControlMessage::JoinRejection(rejection))
                if rejection.listener_id == first_id && rejection.reason.contains("busy")
        )
    });
    wait_until(Duration::from_secs(5), || {
        host.counters().closed_control_connections == 1 && host.counters().queue_overflows > 0
    });

    // A second raw TCP connection must be visibly closed as busy, not accepted
    // as a new peer while the first reader is parked on its terminal event.
    let endpoint = host.endpoint();
    let mut probe = TcpStream::connect(SocketAddr::new(endpoint.address, endpoint.control_port))
        .expect("probe TCP connection");
    probe
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set probe timeout");
    let mut byte = [0_u8; 1];
    match probe.read(&mut byte) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
            ) => {}
        other => panic!("busy probe must close without accepting a peer: {other:?}"),
    }
    assert_eq!(
        host.counters().accepted_control_connections,
        1,
        "parked terminal reader must still consume the single peer budget"
    );

    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("first PeerAccepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "terminal disconnect was lost");
        if matches!(
            host.recv_event(remaining).expect("terminal event"),
            TransportEvent::PeerDisconnected { peer, .. }
                if peer.device_id.as_ref() == Some(&first_id)
        ) {
            break;
        }
    }
    drop(first.shutdown());

    // Once the collector recovers, the old worker releases its slot and a
    // new generation can join/authorize normally. No ghost route survives.
    let second_id = id_device("budget-second");
    let mut second = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), second_id.clone(), host.endpoint()),
            clock,
        )
        .expect("fresh TCP listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("fresh PeerAccepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    second
        .send_control(&join_request(&session, &second_id, "Second"))
        .expect("fresh join");
    wait_for_control_from(&mut host, &second_id, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&second_id, second.local_routes())
        .expect("fresh peer authorized");
    wait_for_authorized(&mut host, &second_id);
    assert_eq!(host.counters().accepted_control_connections, 2);
    second.shutdown().expect("fresh listener shutdown");
    host.shutdown().expect("host shutdown");
}
