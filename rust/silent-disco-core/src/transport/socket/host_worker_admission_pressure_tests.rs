use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn worker_start_failure_after_accept_emits_terminal_and_releases_peer_slot() {
    use crate::domain::MonotonicMillis;

    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let _client = TcpStream::connect(listener.local_addr().unwrap()).expect("client");
    let (stream, remote) = listener.accept().expect("accepted peer");
    let shutdown_copy = stream.try_clone().expect("shutdown socket");
    let (write_sender, _write_receiver) = sync_channel(1);
    let counters = Arc::new(CounterState::default());
    let peer = Arc::new(PeerState {
        number: 17,
        remote,
        identity: Mutex::new(None),
        authorized: AtomicBool::new(false),
        admission_denied: AtomicBool::new(false),
        join_forwarded: AtomicBool::new(false),
        active: AtomicBool::new(true),
        consecutive_failures: AtomicU32::new(0),
        sender: ControlSender::new(write_sender, Duration::from_millis(20), counters.clone()),
        stop: Arc::new(AtomicBool::new(false)),
        shutdown_stream: Mutex::new(shutdown_copy),
        last_inbound_millis: AtomicU64::new(1),
        silence_armed: AtomicBool::new(false),
    });
    let registry = Mutex::new(HashMap::from([(peer.number, peer.clone())]));
    let (sender, receiver) = sync_channel(2);
    let stop = AtomicBool::new(false);
    sender
        .send(TransportEvent::PeerAccepted {
            peer: peer.transport_peer(),
            received_at: MonotonicMillis::new(1),
        })
        .expect("accepted event");

    let failure = TransportError::new(
        TransportErrorKind::Listen,
        TransportChannel::Control,
        "injected TCP reader startup failure",
    );
    rollback_failed_peer_start(
        &peer,
        &registry,
        &sender,
        &stop,
        &counters,
        MonotonicMillis::new(2),
        Duration::from_millis(20),
        &failure,
    );

    assert!(matches!(
        receiver.try_recv(),
        Ok(TransportEvent::PeerAccepted { .. })
    ));
    match receiver.try_recv().expect("terminal after accepted") {
        TransportEvent::PeerDisconnected {
            peer: disconnected,
            error: Some(reason),
            ..
        } => {
            assert_eq!(disconnected.control_address, remote);
            assert_eq!(reason.message, failure.message);
        }
        _ => panic!("expected terminal disconnect after PeerAccepted"),
    }
    assert!(!peer.active.load(Ordering::Acquire));
    assert!(peer.stop.load(Ordering::Acquire));
    assert!(registry.lock().unwrap().is_empty());
    assert_eq!(counters.snapshot().closed_control_connections, 1);
    assert_eq!(counters.snapshot().delivery_failures, 0);
}

#[test]
fn saturated_join_queue_is_refused_without_silent_acceptance() {
    let (sender, receiver) = sync_channel(1);
    let counters = CounterState::default();
    sender.send(1_u8).expect("fill queue");
    let refused = AtomicBool::new(false);
    assert!(!forward_join_or_refuse(&sender, &counters, 2_u8, || {
        refused.store(true, Ordering::Release);
    }));
    assert!(refused.load(Ordering::Acquire));
    assert_eq!(receiver.try_recv().expect("original event"), 1);
    assert!(receiver.try_recv().is_err());
    assert_eq!(counters.snapshot().queue_overflows, 1);
    assert_eq!(counters.snapshot().delivery_failures, 1);
}

#[test]
fn disconnected_join_collector_refuses_instead_of_silent_acceptance() {
    let (sender, receiver) = sync_channel::<u8>(1);
    drop(receiver);
    let counters = CounterState::default();
    let refused = AtomicBool::new(false);
    assert!(!forward_join_or_refuse(&sender, &counters, 2, || {
        refused.store(true, Ordering::Release);
    }));
    assert!(refused.load(Ordering::Acquire));
    assert_eq!(counters.snapshot().delivery_failures, 1);
}

#[test]
fn open_join_queue_delivers_once_without_refusing() {
    let (sender, receiver) = sync_channel(1);
    let counters = CounterState::default();

    assert!(forward_join_or_refuse(&sender, &counters, 17_u8, || {
        panic!("join was delivered, must not send busy response");
    }));
    assert_eq!(receiver.try_recv().expect("delivered join"), 17);
    assert_eq!(counters.snapshot().queue_overflows, 0);
    assert_eq!(counters.snapshot().delivery_failures, 0);
}

#[test]
fn stalled_actor_does_not_permanently_hold_retired_tcp_peer_capacity() {
    use crate::transport::ManualTransportClock;

    let session = SessionId::new("stalled-actor-peer-capacity").expect("session");
    let mut config = HostTransportConfig::loopback(session);
    config.event_queue_capacity = 1;
    config.max_peers = 1;
    config.io_timeout = Duration::from_millis(10);
    config.operation_timeout = Duration::from_millis(80);

    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback TCP");
    let address = listener.local_addr().expect("bound address");
    let first_client = TcpStream::connect(address).expect("first client");
    let (first_stream, first_address) = listener.accept().expect("first accept");
    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(CounterState::default());
    let peers = Arc::new(Mutex::new(HashMap::new()));
    let routes = Arc::new(Mutex::new(HashMap::new()));
    let workers = Arc::new(Mutex::new(Vec::new()));
    let (sender, receiver) = sync_channel(1);
    let clock: Arc<dyn TransportClock> = Arc::new(ManualTransportClock::new(0));
    let next_peer = AtomicU64::new(1);

    register_peer(
        first_stream,
        first_address,
        &config,
        &stop,
        &counters,
        &peers,
        &routes,
        &workers,
        &sender,
        &clock,
        &next_peer,
    )
    .expect("register first TCP peer");
    assert_eq!(peers.lock().unwrap().len(), 1);
    // Leave PeerAccepted queued while the peer disconnects. The reader
    // cannot enqueue PeerDisconnected until the bounded deadline expires.
    drop(first_client);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !peers.lock().unwrap().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "retired peer held the sole admission slot indefinitely"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(counters.snapshot().accepted_control_connections, 1);
    assert_eq!(counters.snapshot().closed_control_connections, 1);
    assert!(
        counters.snapshot().delivery_failures >= 1,
        "lost terminal event must be counted, not silently discarded"
    );
    assert!(matches!(
        receiver.try_recv(),
        Ok(TransportEvent::PeerAccepted { .. })
    ));

    // The same bounded host can accept a replacement after the actor resumes.
    let second_client = TcpStream::connect(address).expect("replacement client");
    let (second_stream, second_address) = listener.accept().expect("replacement accept");
    register_peer(
        second_stream,
        second_address,
        &config,
        &stop,
        &counters,
        &peers,
        &routes,
        &workers,
        &sender,
        &clock,
        &next_peer,
    )
    .expect("released capacity accepts replacement");
    assert_eq!(peers.lock().unwrap().len(), 1);
    assert!(matches!(
        receiver.try_recv(),
        Ok(TransportEvent::PeerAccepted { .. })
    ));
    assert_eq!(counters.snapshot().accepted_control_connections, 2);
    stop.store(true, Ordering::Release);
    drop(second_client);
    for peer in peers.lock().unwrap().values() {
        drop(peer.close());
    }
}
