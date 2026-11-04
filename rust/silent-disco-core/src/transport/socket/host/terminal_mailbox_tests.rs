//! Live TCP regressions for a stopped host-actor collector.
//! Terminal delivery must remain eventual and bounded, not park workers.
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::transport::test_support::id_session;
use crate::transport::{
    HostTransportConfig, HostTransportNode, ManualTransportClock, TransportEvent,
};

use super::SocketHostTransport;

fn one_peer_host() -> SocketHostTransport {
    let mut config = HostTransportConfig::loopback(id_session("terminal-mailbox-pressure"));
    config.max_peers = 1;
    config.event_queue_capacity = 1;
    config.io_timeout = Duration::from_millis(20);
    config.operation_timeout = Duration::from_millis(100);
    SocketHostTransport::bind(config, Arc::new(ManualTransportClock::new(0)))
        .expect("bind loopback host")
}

fn connect(host: &SocketHostTransport) -> TcpStream {
    let endpoint = host.endpoint();
    TcpStream::connect(SocketAddr::new(endpoint.address, endpoint.control_port))
        .expect("connect actual TCP socket")
}

fn wait_for(label: &str, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "deadline waiting for {label}");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn full_actor_queue_retains_terminal_and_reclaims_slot_without_polling() {
    let mut host = one_peer_host();
    let raw = connect(&host);
    wait_for("peer accepted", || {
        host.counters().accepted_control_connections == 1
    });
    drop(raw);
    // The PeerAccepted event still occupies the ONLY normal event slot.
    // No collector is running; a blocking terminal send would deadlock
    // teardown and keep the host's max_peers=1 admission slot forever.
    wait_for("retired peer slot", || {
        host.peers.lock().expect("peer registry").is_empty() && host.terminal_mailbox.pending() == 1
    });
    assert!(matches!(
        host.recv_event(Duration::from_secs(1))
            .expect("first event"),
        TransportEvent::PeerAccepted { .. }
    ));
    assert!(matches!(
        host.recv_event(Duration::from_secs(1))
            .expect("retained disconnect"),
        TransportEvent::PeerDisconnected { .. }
    ));
    assert_eq!(host.counters().authoritative_event_failures, 0);
    assert_eq!(host.terminal_mailbox.pending(), 0);
    host.shutdown().expect("host shutdown");
}

#[test]
fn terminal_backlog_caps_reconnect_churn_and_drains_before_new_admission() {
    let mut host = one_peer_host();
    let first = connect(&host);
    wait_for("first peer accepted", || {
        host.counters().accepted_control_connections == 1
    });
    drop(first);
    wait_for("first terminal retained", || {
        host.terminal_mailbox.pending() == 1
    });
    let refused = connect(&host);
    // With the collector stopped, the retired terminal consumes the
    // admission budget, preventing infinitely many new workers/tombstones.
    wait_for("busy capacity refusal", || {
        host.counters().busy_admission_refusals > 0
    });
    assert_eq!(host.counters().accepted_control_connections, 1);
    drop(refused);
    assert!(matches!(
        host.recv_event(Duration::from_secs(1)).expect("accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    assert!(matches!(
        host.recv_event(Duration::from_secs(1)).expect("terminal"),
        TransportEvent::PeerDisconnected { .. }
    ));
    let second = connect(&host);
    wait_for("new admission after drain", || {
        host.counters().accepted_control_connections == 2
    });
    assert!(matches!(
        host.recv_event(Duration::from_secs(1))
            .expect("second accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    drop(second);
    assert!(matches!(
        host.recv_event(Duration::from_secs(2))
            .expect("second terminal"),
        TransportEvent::PeerDisconnected { .. }
    ));
    assert_eq!(host.counters().authoritative_event_failures, 0);
    host.shutdown().expect("host shutdown");
}
