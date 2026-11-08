use super::super::shared::ControlSender;
use super::join_delivery::forward_join_or_refuse;
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::sync_channel;

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
        verified: None,
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
    assert!(
        !forward_join_or_refuse(&sender, &counters, 2_u8, || {
            refused.store(true, Ordering::Release);
            Ok(())
        })
        .expect("writer acknowledged busy refusal")
    );
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
    assert!(
        !forward_join_or_refuse(&sender, &counters, 2, || {
            refused.store(true, Ordering::Release);
            Ok(())
        })
        .expect("writer acknowledged disconnected collector refusal")
    );
    assert!(refused.load(Ordering::Acquire));
    assert_eq!(counters.snapshot().delivery_failures, 1);
}

#[test]
fn open_join_queue_delivers_once_without_refusing() {
    let (sender, receiver) = sync_channel(1);
    let counters = CounterState::default();

    assert!(
        forward_join_or_refuse(&sender, &counters, 17_u8, || {
            panic!("join was delivered, must not send busy response");
        })
        .expect("join reaches actor")
    );
    assert_eq!(receiver.try_recv().expect("delivered join"), 17);
    assert_eq!(counters.snapshot().queue_overflows, 0);
    assert_eq!(counters.snapshot().delivery_failures, 0);
}

#[test]
fn failed_tcp_busy_reply_is_reported_not_counted_as_delivered() {
    let (sender, receiver) = sync_channel(1);
    let counters = CounterState::default();
    sender.send(1_u8).expect("fill actor queue");
    let error = forward_join_or_refuse(&sender, &counters, 2_u8, || {
        Err(TransportError::new(
            TransportErrorKind::Timeout,
            TransportChannel::Control,
            "TCP control writer did not acknowledge busy reply",
        ))
    })
    .expect_err("failed writer cannot be treated as delivered refusal");
    assert_eq!(error.kind, TransportErrorKind::Delivery);
    assert!(error.message.contains("busy refusal"));
    assert_eq!(receiver.try_recv().expect("previous event retained"), 1);
    assert!(receiver.try_recv().is_err(), "join was never admitted");
    assert_eq!(counters.snapshot().queue_overflows, 1);
    assert_eq!(counters.snapshot().busy_admission_refusals, 1);
    assert_eq!(counters.snapshot().delivery_failures, 2);
}

#[test]
fn failed_tcp_busy_reply_after_actor_disconnect_is_visible() {
    let (sender, receiver) = sync_channel::<u8>(1);
    drop(receiver);
    let counters = CounterState::default();
    let result = forward_join_or_refuse(&sender, &counters, 9, || {
        Err(TransportError::new(
            TransportErrorKind::ShuttingDown,
            TransportChannel::Control,
            "control socket retired",
        ))
    });
    assert_eq!(
        result.expect_err("refusal failed").kind,
        TransportErrorKind::Delivery
    );
    assert_eq!(counters.snapshot().queue_overflows, 0);
    assert_eq!(counters.snapshot().delivery_failures, 2);
    assert_eq!(counters.snapshot().busy_admission_refusals, 1);
}
