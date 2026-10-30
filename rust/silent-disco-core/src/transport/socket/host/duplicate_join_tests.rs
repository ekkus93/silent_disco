//! One accepted join per TCP generation, including retransmissions before
//! and after approval. A new TCP connection may submit a fresh join.
//! This does not authenticate a claimed device ID (legacy v3 remains insecure).
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{ControlMessage, Hello, Pause, ProtocolFrame, Stop, TrackBoundary};
use crate::transport::test_support::{
    id_device, id_session, join_request, wait_for_authorized, wait_for_control_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportErrorKind, TransportEvent, TransportFactory,
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

#[test]
fn pending_socket_cannot_receive_playback_controls_before_authorization() {
    let session = id_session("pending-playback-isolation");
    let device = id_device("pending-playback-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("pending TCP connection");
    listener
        .send_control(&join_request(&session, &device, "Pending listener"))
        .expect("join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = crate::transport::test_support::stream_start(&session, 1_000);
    let ControlMessage::StreamStart(ref active) = start else {
        panic!("StreamStart fixture");
    };
    let prohibited = [
        start.clone(),
        ControlMessage::TrackBoundary(TrackBoundary {
            session_id: session.clone(),
            stream_id: active.stream_id.clone(),
            first_sample_index: SampleIndex::new(240),
            entry_id: "private-entry".to_owned(),
            display_name: "Private track".to_owned(),
        }),
        ControlMessage::Pause(Pause {
            session_id: session.clone(),
            stream_id: active.stream_id.clone(),
            host_pause_time_ms: MonotonicMillis::new(1_005),
        }),
        ControlMessage::Stop(Stop {
            session_id: session.clone(),
            stream_id: active.stream_id.clone(),
            host_stop_time_ms: MonotonicMillis::new(1_010),
        }),
    ];
    let before = host.counters().control_frames_sent;
    for message in prohibited {
        let error = host
            .send_pending_control(&device, &message)
            .expect_err("pending TCP peer cannot receive playback controls");
        assert_eq!(error.kind, TransportErrorKind::Unauthorized);
    }
    assert_eq!(host.counters().control_frames_sent, before);

    // Live controls remain cached for the eventual authorized generation;
    // they are not sent to a pending TCP socket.
    host.broadcast_control(&start).expect("cache playback");
    assert_eq!(host.counters().control_frames_sent, before);
    let hello = ControlMessage::Hello(Hello {
        session_id: session,
        session_name: "Pending session".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });
    host.send_pending_control(&device, &hello)
        .expect("pending Hello is permitted");
    host.authorize_peer(&device, listener.local_routes())
        .expect("explicitly authorize pending peer");
    wait_for_authorized(&mut host, &device);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while received.len() < 2 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing admission or cached playback");
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(message),
            ..
        } = listener
            .recv_event(remaining)
            .expect("TCP admission and playback replay")
        {
            received.push(message);
        }
    }
    assert!(
        received == vec![hello, start],
        "pending socket must see only Hello, then cached Start after admission"
    );
    listener.shutdown().expect("close listener");
    host.shutdown().expect("close host");
}

#[test]
fn same_id_join_arriving_while_authorization_waits_cannot_steal_approval() {
    use crate::transport::test_support::wait_for_host_event;

    let session = id_session("pending-approval-race");
    let device = id_device("same-id-approval-race");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let endpoint = host.endpoint();
    let factory = SocketTransportFactory;
    let mut first = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), endpoint),
            clock.clone(),
        )
        .expect("first pending TCP generation");
    first
        .send_control(&join_request(&session, &device, "Original join"))
        .expect("first join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    let original = host
        .peers
        .lock()
        .expect("peer registry")
        .values()
        .find(|peer| peer.device_id().as_ref() == Some(&device))
        .cloned()
        .expect("first pending peer");
    let initial_refs = Arc::strong_count(&original);
    let peers = Arc::clone(&host.peers);
    let gate = Arc::clone(&host.control_delivery);
    let guard = gate.lock().expect("hold replay ordering gate");
    let first_routes = first.local_routes();

    let mut host = thread::scope(|scope| {
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let target = device.clone();
        scope.spawn(move || {
            started_tx.send(()).expect("authorization started");
            let result = host.authorize_peer(&target, first_routes);
            assert!(done_tx.send((host, result)).is_ok());
        });
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("authorization started");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&original) <= initial_refs {
            assert!(Instant::now() < deadline, "first peer not selected");
            thread::yield_now();
        }

        // A competing TCP generation claims the same device ID AFTER the
        // original was selected but BEFORE its approval acquires the gate.
        let mut second = factory
            .connect_listener(
                ListenerTransportConfig::loopback(session.clone(), device.clone(), endpoint),
                clock,
            )
            .expect("competing pending TCP connection");
        second
            .send_control(&join_request(&session, &device, "Competing join"))
            .expect("second join");
        let arrival_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pending = peers
                .lock()
                .expect("peer registry")
                .values()
                .filter(|peer| {
                    peer.active.load(Ordering::Acquire)
                        && !peer.authorized.load(Ordering::Acquire)
                        && peer.device_id().as_ref() == Some(&device)
                })
                .count();
            if pending == 2 {
                break;
            }
            assert!(
                Instant::now() < arrival_deadline,
                "competing TCP generation not pending before gate release"
            );
            thread::yield_now();
        }
        drop(guard);
        let (mut host, result) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("blocked authorization completes");
        let error = result.expect_err("ambiguous pending generation must not be admitted");
        assert_eq!(error.kind, TransportErrorKind::Unauthorized);
        assert!(
            host.routes.lock().expect("routes").get(&device).is_none(),
            "failed approval must not publish an authorized UDP route"
        );
        assert!(!original.authorized.load(Ordering::Acquire));

        first.shutdown().expect("retire original generation");
        drop(wait_for_host_event(&mut host, |event| {
            matches!(event, TransportEvent::PeerDisconnected { peer, .. }
                if peer.device_id.as_ref() == Some(&device))
        }));
        host.authorize_peer(&device, second.local_routes())
            .expect("fresh approval of sole remaining generation");
        wait_for_authorized(&mut host, &device);
        second.shutdown().expect("close replacement generation");
        host
    });
    host.shutdown().expect("close host");
}
