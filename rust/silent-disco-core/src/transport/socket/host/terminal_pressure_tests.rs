//! Real TCP terminal delivery under bounded event-queue pressure.
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::MonotonicMillis;
use crate::protocol::{ControlMessage, Heartbeat, Hello, ProtocolFrame};
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
fn full_host_queue_refuses_join_and_still_reports_peer_disconnect() {
    let session = id_session("host-terminal-pressure");
    let device = id_device("host-pressure-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let mut listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("connect listener");

    // The host's one event slot holds PeerAccepted. Its reader must refuse
    // the JoinRequest rather than claim admission it cannot report to the actor.
    listener
        .send_control(&join_request(&session, &device, "Full host queue"))
        .expect("send join");
    wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
        matches!(
            frame,
            ProtocolFrame::Control(ControlMessage::JoinRejection(rejection))
                if rejection.listener_id == device && rejection.reason.contains("busy")
        )
    });

    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("accepted event"),
        TransportEvent::PeerAccepted { .. }
    ));
    assert!(matches!(
        host.recv_event(Duration::from_secs(5)).expect("terminal event"),
        TransportEvent::PeerDisconnected { peer, .. }
            if peer.device_id.as_ref() == Some(&device)
    ));
    assert!(
        host.authorize_peer(&device, listener.local_routes())
            .is_err(),
        "refused listener must never acquire a route"
    );
    assert!(host.counters().queue_overflows >= 1);
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn full_listener_queue_preserves_hello_before_remote_disconnect() {
    let session = id_session("listener-terminal-pressure");
    let device = id_device("listener-pressure-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let mut config =
        ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint());
    config.event_queue_capacity = 1;
    let mut listener = SocketTransportFactory
        .connect_listener(config, clock)
        .expect("connect listener");
    listener
        .send_control(&join_request(&session, &device, "Listener queue pressure"))
        .expect("send join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let hello = ControlMessage::Hello(Hello {
        session_id: session.clone(),
        session_name: "Terminal pressure".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });
    host.send_pending_control(&device, &hello)
        .expect("deliver pending Hello");
    // Confirm the listener's one-slot queue is occupied before the TCP EOF.
    let deadline = Instant::now() + Duration::from_secs(5);
    while listener.counters().control_frames_received == 0 {
        assert!(Instant::now() < deadline, "listener never received Hello");
        thread::sleep(Duration::from_millis(2));
    }
    // Keep the collector stopped while the host floods real TCP heartbeats.
    // Hello occupies the one-slot queue; telemetry must not block the reader
    // or delay a subsequent authoritative terminal event.
    for index in 0..16 {
        host.send_pending_control(
            &device,
            &ControlMessage::Heartbeat(Heartbeat {
                session_id: session.clone(),
                listener_id: device.clone(),
                sent_at_elapsed_ms: MonotonicMillis::new(index),
            }),
        )
        .expect("send host heartbeat");
    }
    wait_until(Duration::from_secs(5), || {
        listener.counters().control_frames_received >= 17
    });
    assert!(listener.counters().queue_overflows > 0);
    host.shutdown().expect("close host TCP socket");

    assert!(matches!(
        listener
            .recv_event(Duration::from_secs(5))
            .expect("Hello event"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(ControlMessage::Hello(_)),
            ..
        }
    ));
    assert!(matches!(
        listener
            .recv_event(Duration::from_secs(5))
            .expect("disconnect event"),
        TransportEvent::PeerDisconnected { .. }
    ));
    listener.shutdown().expect("listener shutdown");
}

#[test]
fn multi_peer_accept_pressure_refuses_unobservable_sockets_and_allows_clean_reconnect() {
    let session = id_session("multi-peer-pressure");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    config.operation_timeout = Duration::from_millis(150);
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let devices = [
        id_device("pressure-a"),
        id_device("pressure-b"),
        id_device("pressure-c"),
    ];
    let mut listeners = Vec::new();
    for (index, device) in devices.iter().enumerate() {
        let listener = SocketTransportFactory
            .connect_listener(
                ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
                clock.clone(),
            )
            .expect("connect listener");
        if index == 0 {
            // A legitimate join is attempted immediately. The actor queue is
            // still occupied by PeerAccepted, so this join is visibly refused.
            // Waiting until after the pressure timeout would now (correctly)
            // expire an unjoined peer before it can submit any request.
            listener
                .send_control(&join_request(&session, device, "Busy listener"))
                .expect("first join");
        }
        listeners.push(listener);
    }

    // The first accept fills the only actor event slot. With no collector,
    // later TCP accepts must close after a finite deadline, not silently
    // continue as unobservable peers or emit ghost authorization events.
    wait_until(Duration::from_secs(5), || {
        host.counters().accepted_control_connections == devices.len() as u64
            && host.counters().closed_control_connections >= devices.len() as u64
            && host.counters().delivery_failures >= devices.len() as u64
    });
    for (listener, device) in listeners.iter().zip(&devices).skip(1) {
        assert!(
            host.authorize_peer(device, listener.local_routes())
                .is_err(),
            "timed-out TCP peer must not retain an authorizable route"
        );
    }

    // The first reader was started after its PeerAccepted reached the queue.
    // Its immediate JoinRequest cannot fit and receives an explicit busy response.
    wait_for_frame(listeners[0].as_mut(), TransportChannel::Control, |frame| {
        matches!(
            frame,
            ProtocolFrame::Control(ControlMessage::JoinRejection(rejection))
                if rejection.listener_id == devices[0] && rejection.reason.contains("busy")
        )
    });
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("first accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "first terminal event lost");
        if matches!(
            host.recv_event(remaining).expect("terminal event"),
            TransportEvent::PeerDisconnected { peer, .. }
                if peer.device_id.as_ref() == Some(&devices[0])
        ) {
            break;
        }
    }
    for (listener, device) in listeners.iter_mut().zip(&devices) {
        assert!(
            host.authorize_peer(device, listener.local_routes())
                .is_err(),
            "refused peer must not have a live route"
        );
        drop(listener.shutdown());
    }
    assert!(host.counters().queue_overflows >= devices.len() as u64);
    assert!(host.counters().delivery_failures >= devices.len() as u64);

    // Draining the actor queue permits a fresh TCP generation for the same
    // claimed ID. Its authorization is independent of every refused socket.
    let device = &devices[0];
    let mut retry = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("reconnect listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("new peer accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    retry
        .send_control(&join_request(&session, device, "Retried listener"))
        .expect("send retry");
    wait_for_control_from(&mut host, device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(device, retry.local_routes())
        .expect("authorize fresh connection");
    wait_for_authorized(&mut host, device);
    retry.shutdown().expect("shutdown retried listener");
    host.shutdown().expect("shutdown host");
}
#[test]
fn saturated_host_event_queue_rolls_back_real_socket_authorization() {
    let session = id_session("host-admission-event-backpressure");
    let device = id_device("candidate-with-full-actor-queue");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    config.operation_timeout = Duration::from_millis(75);
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let mut candidate = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("candidate TCP connection");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("candidate accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    candidate
        .send_control(&join_request(&session, &device, "Candidate"))
        .expect("join request");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    // A second real TCP connection fills the host's one-slot actor queue
    // while the first listener waits for its authorization decision.
    let spectator = id_device("congesting-connection");
    let mut congesting = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session, spectator, host.endpoint()),
            clock,
        )
        .expect("congesting TCP connection");
    wait_until(Duration::from_secs(5), || {
        host.counters().accepted_control_connections == 2
    });
    let started = Instant::now();
    let error = host
        .authorize_peer(&device, candidate.local_routes())
        .expect_err("unobserved authorization must be rolled back");
    assert_eq!(error.kind, crate::transport::TransportErrorKind::QueueFull);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "finite admission wait"
    );
    assert!(
        !host
            .routes
            .lock()
            .expect("route registry")
            .contains_key(&device),
        "failed authorization cannot retain a datagram route"
    );
    assert!(host.counters().queue_overflows > 0);
    assert!(host.counters().delivery_failures > 0);
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("congesting accept"),
        TransportEvent::PeerAccepted { .. }
    ));
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("candidate disconnect"),
        TransportEvent::PeerDisconnected { peer, .. }
            if peer.device_id.as_ref() == Some(&device)
    ));
    candidate.shutdown().expect("candidate cleanup");
    congesting.shutdown().expect("congesting cleanup");
    host.shutdown().expect("host cleanup");
}

#[test]
fn real_socket_heartbeat_flood_does_not_hide_authoritative_disconnect() {
    let session = id_session("heartbeat-terminal-priority");
    let device = id_device("heartbeat-flood-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let mut listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("connect listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5)).expect("accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    listener
        .send_control(&join_request(&session, &device, "Telemetry flood"))
        .expect("send join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device);

    // Saturate the single actor event slot with real TCP heartbeats while
    // the host actor collector is stopped. They may be dropped, but the TCP
    // reader must continue to EOF and deliver its authoritative terminal.
    for index in 0..64 {
        listener
            .send_control(&ControlMessage::Heartbeat(Heartbeat {
                session_id: session.clone(),
                listener_id: device.clone(),
                sent_at_elapsed_ms: MonotonicMillis::new(index),
            }))
            .expect("heartbeat TCP write");
    }
    wait_until(Duration::from_secs(5), || host.counters().queue_overflows > 0);
    listener.shutdown().expect("close listener TCP connection");

    let mut disconnected = false;
    for _ in 0..4 {
        match host.recv_event(Duration::from_secs(5)).expect("terminal progress") {
            TransportEvent::PeerDisconnected { peer, .. }
                if peer.device_id.as_ref() == Some(&device) =>
            {
                disconnected = true;
                break;
            }
            TransportEvent::FrameReceived {
                frame:
                    ProtocolFrame::Control(ControlMessage::Heartbeat(_) | ControlMessage::Disconnect(_)),
                ..
            } => {}
            _ => panic!("unexpected host event under telemetry pressure"),
        }
    }
    assert!(
        disconnected,
        "telemetry must not consume the queue indefinitely ahead of disconnect"
    );
    host.shutdown().expect("shutdown host");
}
