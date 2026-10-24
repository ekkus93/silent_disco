//! Pending control frames share the late-join replay ordering gate.
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{ControlMessage, Hello, Pause, ProtocolFrame, TrackBoundary};
use crate::transport::test_support::{
    id_device, id_session, join_request, stream_start, wait_for_authorized, wait_for_control_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn pending_hello_cannot_overtake_or_follow_authorization_replay() {
    let session_id = id_session("pending-hello-replay-order");
    let device_id = id_device("pending-hello-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Pending listener"))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = stream_start(&session_id, 1_000);
    host.broadcast_control(&start).expect("cache stream start");
    let hello = ControlMessage::Hello(Hello {
        session_id,
        session_name: "Pending admission".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });

    // Hold the exact mutex used by authorization and live broadcasts. The
    // pending sender must remain blocked until it can acquire this guard.
    let delivery_gate = Arc::clone(&host.control_delivery);
    let guard = delivery_gate.lock().expect("ordering mutex");
    // Move the Send-only host into the sender thread. Borrowing it across
    // threads would incorrectly require its event receiver to be Sync.
    let mut host = thread::scope(|scope| {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let send_device_id = device_id.clone();
        let send_hello = hello.clone();
        scope.spawn(move || {
            ready_tx.send(()).expect("signal sender start");
            let result = host.send_pending_control(&send_device_id, &send_hello);
            assert!(
                done_tx.send((host, result)).is_ok(),
                "return host and pending delivery result"
            );
        });
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pending sender starts");
        assert!(
            matches!(
                done_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "pending control delivery must hold the replay ordering gate"
        );
        drop(guard);
        let (host, result) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pending send completes");
        result.expect("pending Hello delivered");
        host
    });

    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    let expected = [hello, start];
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while received.len() < expected.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing ordered admission controls");
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(message),
            ..
        } = listener
            .recv_event(remaining)
            .expect("listener control event")
        {
            received.push(message);
        }
    }
    assert!(
        received == expected,
        "pending Hello must precede cached StreamStart on the TCP channel"
    );

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn pending_control_is_rejected_after_authorization_replay() {
    let session_id = id_session("post-authorization-pending");
    let device_id = id_device("post-authorization-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Pending listener"))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = stream_start(&session_id, 1_000);
    host.broadcast_control(&start).expect("cache stream start");
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    // Authorization already replayed StreamStart. A delayed pending Hello
    // must not follow it and regress the listener to a pre-admission state.
    let hello = ControlMessage::Hello(Hello {
        session_id,
        session_name: "Stale pending admission".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });
    assert!(
        host.send_pending_control(&device_id, &hello).is_err(),
        "a pending-only message must not be delivered after authorization"
    );

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn disconnected_pending_peer_cannot_be_authorized_after_selection() {
    let session_id = id_session("pending-disconnect-before-authorization");
    let device_id = id_device("pending-disconnect-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(
            &session_id,
            &device_id,
            "Disconnecting listener",
        ))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let pending_peer = host
        .peers
        .lock()
        .expect("peer registry")
        .values()
        .find(|peer| peer.device_id().as_ref() == Some(&device_id))
        .cloned()
        .expect("identified pending peer");
    let initial_references = Arc::strong_count(&pending_peer);
    let delivery_gate = Arc::clone(&host.control_delivery);
    let guard = delivery_gate.lock().expect("ordering mutex");
    let routes = listener.local_routes();

    let mut host = thread::scope(|scope| {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let target = device_id.clone();
        scope.spawn(move || {
            ready_tx.send(()).expect("signal authorization start");
            let result = host.authorize_peer(&target, routes);
            assert!(
                done_tx.send((host, result)).is_ok(),
                "return host and authorization result"
            );
        });
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("authorization starts");
        // The extra Arc reference proves the authorization method already
        // selected this live peer and is now blocked by the delivery gate.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&pending_peer) <= initial_references {
            assert!(
                Instant::now() < deadline,
                "authorization did not select the pending peer"
            );
            thread::yield_now();
        }
        assert!(
            matches!(
                done_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "authorization must wait behind the replay ordering gate"
        );
        pending_peer
            .close()
            .expect("disconnect selected pending peer");
        drop(guard);
        let (host, result) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("authorization completes");
        assert!(
            result.is_err(),
            "a disconnected peer must not become authorized after selection"
        );
        assert!(
            !pending_peer
                .authorized
                .load(std::sync::atomic::Ordering::Acquire),
            "disconnected peer must remain unauthorized"
        );
        assert!(
            !host.routes.lock().expect("routes").contains_key(&device_id),
            "no dead authorized route may be published"
        );
        host
    });

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn targeted_control_rechecks_authorized_route_after_delivery_gate() {
    let session_id = id_session("targeted-disconnect-during-delivery");
    let device_id = id_device("targeted-disconnect-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Targeted listener"))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    let targeted = ControlMessage::Hello(Hello {
        session_id,
        session_name: "Expired target".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: false,
    });
    // Hold the same mutex that serializes targeted sends and playback replay.
    // Simulate independent route retirement while targeted delivery waits.
    // The sender must look up the route only after acquiring the ordering
    // gate, rather than caching an Arc before authorization changes.
    let route_registry = Arc::clone(&host.routes);
    let gate = Arc::clone(&host.control_delivery);
    let guard = gate.lock().expect("delivery ordering lock");
    let mut host = thread::scope(|scope| {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let target = device_id.clone();
        scope.spawn(move || {
            ready_tx.send(()).expect("targeted send starts");
            let result = host.send_control(&target, &targeted);
            assert!(
                done_tx.send((host, result)).is_ok(),
                "return transport and targeted result"
            );
        });
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("targeted send starts");
        assert!(
            matches!(
                done_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "targeted control must wait for the replay ordering mutex"
        );
        let retired = route_registry
            .lock()
            .expect("authorized routes")
            .remove(&device_id)
            .expect("authorized route exists");
        retired.peer.close().expect("disconnect retired peer");
        drop(guard);
        let (host, result) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("targeted send completes");
        assert!(
            matches!(result, Err(error) if error.to_string().contains("authorized peer is not connected")),
            "a retired route must be rejected by a fresh post-gate lookup"
        );
        assert!(
            !host
                .routes
                .lock()
                .expect("authorized routes")
                .contains_key(&device_id),
            "retired authorized route must stay absent"
        );
        host
    });

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn disconnect_waits_for_control_delivery_gate_before_removing_route() {
    let session_id = id_session("disconnect-delivery-serialization");
    let device_id = id_device("disconnect-delivery-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(
            &session_id,
            &device_id,
            "Disconnect listener",
        ))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    // Hold the delivery gate while a concurrent disconnect is attempted.
    let gate = Arc::clone(&host.control_delivery);
    let routes = Arc::clone(&host.routes);
    let guard = gate.lock().expect("delivery ordering lock");
    let mut host = thread::scope(|scope| {
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let target = device_id.clone();
        scope.spawn(move || {
            started_tx.send(()).expect("signal disconnect start");
            let result = host.disconnect_peer(&target);
            assert!(
                done_tx.send((host, result)).is_ok(),
                "return host and disconnect result"
            );
        });
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("disconnect starts");
        assert!(
            matches!(
                done_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "disconnect must wait for in-flight control delivery"
        );
        assert!(
            routes
                .lock()
                .expect("authorized routes")
                .contains_key(&device_id),
            "disconnect must not remove the route before the gate is released"
        );
        drop(guard);
        let (host, result) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("disconnect completes after delivery");
        result.expect("disconnect authorized peer");
        host
    });

    assert!(
        !host
            .routes
            .lock()
            .expect("authorized routes")
            .contains_key(&device_id),
        "disconnect must remove the route"
    );
    let stale = ControlMessage::Hello(Hello {
        session_id,
        session_name: "Disconnected target".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: false,
    });
    assert!(
        host.send_control(&device_id, &stale).is_err(),
        "targeted control after disconnect must fail"
    );
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn stale_replay_controls_are_not_sent_to_live_authorized_listeners() {
    let session_id = id_session("live-stale-control-session");
    let device_id = id_device("live-stale-control-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Live listener"))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    let start = stream_start(&session_id, 1_000);
    let ControlMessage::StreamStart(details) = &start else {
        panic!("test fixture must create StreamStart");
    };
    let pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: details.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_500),
    });
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session_id.clone(),
        stream_id: details.stream_id.clone(),
        first_sample_index: SampleIndex::new(48_000),
        entry_id: "following".to_owned(),
        display_name: "Following song".to_owned(),
    });
    let later_pause = ControlMessage::Pause(Pause {
        session_id: session_id.clone(),
        stream_id: details.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(2_500),
    });
    host.broadcast_control(&start).expect("start stream");
    host.broadcast_control(&boundary)
        .expect("schedule boundary");
    host.broadcast_control(&pause).expect("pause stream");
    // A retransmitted StreamStart with the original timeline is not Resume.
    // It must neither unpause the cache nor reach the live TCP listener.
    let count_while_paused = host.counters().control_frames_sent;
    assert_eq!(
        host.broadcast_control(&start)
            .expect("reject old start while paused"),
        crate::transport::TransportDelivery::new(0, 0, 0, 0).expect("empty delivery")
    );
    assert_eq!(host.counters().control_frames_sent, count_while_paused);

    let ControlMessage::StreamStart(mut resumed) = start.clone() else {
        panic!("expected StreamStart");
    };
    resumed.host_start_time_ms = MonotonicMillis::new(1_100);
    host.broadcast_control(&ControlMessage::StreamStart(resumed))
        .expect("deliver re-anchored Resume");

    let sent_before = host.counters().control_frames_sent;
    let stale_pause = host.broadcast_control(&pause).expect("reject old Pause");
    let stale_boundary = host
        .broadcast_control(&boundary)
        .expect("reject repeated boundary");
    assert_eq!(
        stale_pause,
        crate::transport::TransportDelivery::new(0, 0, 0, 0).expect("empty delivery")
    );
    assert_eq!(
        stale_boundary,
        crate::transport::TransportDelivery::new(0, 0, 0, 0).expect("empty delivery")
    );
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before,
        "rejected replay events must not reach the active listener writer"
    );
    host.broadcast_control(&later_pause)
        .expect("subsequent new Pause must be broadcast");
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before + 1,
        "a genuinely later Pause must still reach the connected listener"
    );

    // A delayed Stop from a replaced stream must not tear down the new
    // stream for connected listeners. The matching Stop must still pass.
    let replacement = ControlMessage::StreamStart(crate::protocol::StreamStart {
        stream_id: crate::domain::StreamId::new("stream-2").expect("replacement stream ID"),
        host_start_time_ms: MonotonicMillis::new(2_000),
        ..details.clone()
    });
    host.broadcast_control(&replacement)
        .expect("replace active stream");
    let sent_before_stale_stop = host.counters().control_frames_sent;
    let stale_stop = ControlMessage::Stop(crate::protocol::Stop {
        session_id: session_id.clone(),
        stream_id: details.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(3_000),
    });
    assert_eq!(
        host.broadcast_control(&stale_stop)
            .expect("reject Stop from replaced stream"),
        crate::transport::TransportDelivery::new(0, 0, 0, 0).expect("empty delivery")
    );
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before_stale_stop,
        "a stale Stop must not reach the new stream's listeners"
    );
    let ControlMessage::StreamStart(new_details) = &replacement else {
        panic!("replacement fixture must create StreamStart");
    };
    host.broadcast_control(&ControlMessage::Stop(crate::protocol::Stop {
        session_id,
        stream_id: new_details.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(3_500),
    }))
    .expect("stop active replacement stream");
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before_stale_stop + 1,
        "a matching Stop must still reach the live listener"
    );

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}
