//! Real TCP terminal delivery under bounded event-queue pressure.
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{ControlMessage, Hello, ProtocolFrame};
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
        session_id: session,
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
fn multi_peer_join_pressure_refuses_each_request_and_allows_clean_reconnect() {
    let session = id_session("multi-peer-pressure");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let devices = [
        id_device("pressure-a"),
        id_device("pressure-b"),
        id_device("pressure-c"),
    ];
    let mut listeners: Vec<_> = devices
        .iter()
        .map(|device| {
            SocketTransportFactory
                .connect_listener(
                    ListenerTransportConfig::loopback(
                        session.clone(),
                        device.clone(),
                        host.endpoint(),
                    ),
                    clock.clone(),
                )
                .expect("connect listener")
        })
        .collect();

    // Every TCP connection is accepted before any JoinRequest. With one
    // application event slot and no collector, the first PeerAccepted fills
    // the queue and the other accepts are telemetry-only. No join can fit.
    wait_until(Duration::from_secs(5), || {
        host.counters().accepted_control_connections == devices.len() as u64
    });
    for (listener, device) in listeners.iter_mut().zip(&devices) {
        listener
            .send_control(&join_request(&session, device, "Busy listener"))
            .expect("send join");
    }
    for (listener, device) in listeners.iter_mut().zip(&devices) {
        wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
            matches!(
                frame,
                ProtocolFrame::Control(ControlMessage::JoinRejection(rejection))
                    if rejection.listener_id == *device && rejection.reason.contains("busy")
            )
        });
    }

    // The TCP readers must send the explicit busy reply and retire their
    // routes. Their disconnect events are authoritative and remain queued
    // until the host drains capacity, rather than being silently discarded.
    wait_until(Duration::from_secs(5), || {
        host.counters().closed_control_connections == devices.len() as u64
    });
    let mut disconnected = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while disconnected.len() < devices.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "missing terminal event under pressure"
        );
        match host.recv_event(remaining).expect("bounded event delivery") {
            TransportEvent::PeerDisconnected { peer, .. } => {
                let device = peer.device_id.expect("identified refused peer");
                assert!(devices.contains(&device), "unexpected disconnected peer");
                assert!(!disconnected.contains(&device), "duplicate disconnect");
                disconnected.push(device);
            }
            TransportEvent::PeerAccepted { .. } => {}
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
                ..
            } => panic!("a refused join reached the admission actor"),
            _ => {}
        }
    }
    for (listener, device) in listeners.iter_mut().zip(&devices) {
        assert!(
            host.authorize_peer(device, listener.local_routes())
                .is_err(),
            "a refused peer must not have a live route"
        );
        listener.shutdown().expect("shutdown refused listener");
    }
    assert!(host.counters().queue_overflows >= devices.len() as u64);
    assert!(host.counters().delivery_failures >= devices.len() as u64);

    // Draining pressure permits a fresh connection for the same claimed ID.
    // The old refused socket cannot authorize it or leave a ghost listener.
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
