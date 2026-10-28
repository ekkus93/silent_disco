//! Real TCP/UDP multi-peer congestion: telemetry is lossy but Stop, disconnect,
//! admission and fresh Play must remain authoritative across reconnect.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::MonotonicMillis;
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop, SyncRequest};
use crate::transport::test_support::{
    audio_frame, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame, wait_until,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn telemetry_flood_cannot_hide_stop_disconnect_or_reconnected_play() {
    let session = id_session("telemetry-pressure-stop-reconnect");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host_config = HostTransportConfig::loopback(session.clone());
    host_config.event_queue_capacity = 2;
    let mut host =
        SocketHostTransport::bind(host_config, clock.clone()).expect("bind real host sockets");
    let first_id = id_device("pressure-playback-first");
    let second_id = id_device("pressure-playback-second");
    let mut first_config =
        ListenerTransportConfig::loopback(session.clone(), first_id.clone(), host.endpoint());
    first_config.event_queue_capacity = 1;
    let mut first = SocketTransportFactory
        .connect_listener(first_config, clock.clone())
        .expect("first real listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("first peer accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    let mut second = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), second_id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("second real listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("second peer accepted"),
        TransportEvent::PeerAccepted { .. }
    ));

    for (listener, device) in [(first.as_mut(), &first_id), (second.as_mut(), &second_id)] {
        listener
            .send_control(&join_request(&session, device, "Pressure playback"))
            .expect("join");
        wait_for_control_from(&mut host, device, |message| {
            matches!(message, ControlMessage::JoinRequest(_))
        });
        host.authorize_peer(device, listener.local_routes())
            .expect("authorize");
        wait_for_authorized(&mut host, device);
    }

    // Host-side UDP sync telemetry is intentionally lossy. Fill the tiny
    // actor queue with real datagrams while the listener's one-slot TCP
    // control queue is also stalled. Neither pressure source may lose Stop.
    for correlation_id in 0..1_000_u64 {
        second
            .send_sync_request(&SyncRequest {
                session_id: session.clone(),
                correlation_id,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(correlation_id),
            })
            .expect("send real UDP sync telemetry");
    }
    wait_until(Duration::from_secs(5), || {
        host.counters().sync_datagrams_received > 10 && host.counters().queue_overflows > 0
    });
    let start = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(ref started) = start else {
        panic!("stream start fixture");
    };
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_250),
    });
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_500),
    });
    for control in [&start, &pause, &stop] {
        assert_eq!(
            host.broadcast_control(control)
                .expect("send authoritative TCP control under UDP flood")
                .report
                .successful_peers,
            2,
        );
    }
    wait_until(Duration::from_secs(5), || {
        first.counters().queue_overflows > 0
    });
    for control in [&start, &pause, &stop] {
        wait_for_frame(first.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(control.clone())
        });
        wait_for_frame(second.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(control.clone())
        });
    }
    assert_eq!(first.counters().delivery_failures, 0);
    assert_eq!(second.counters().delivery_failures, 0);

    // The terminal event must survive a saturated host queue, and retiring
    // the old TCP reader must not delete the next generation's UDP route.
    first.shutdown().expect("retire first TCP generation");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing first peer disconnect");
        if let TransportEvent::PeerDisconnected { peer, .. } = host
            .recv_event(remaining)
            .expect("drain pressured actor queue")
        {
            if peer.device_id.as_ref() == Some(&first_id) {
                break;
            }
        }
    }

    // Clear any residual lossy UDP telemetry before the new TCP accept.
    // Otherwise its best-effort PeerAccepted notification could be dropped
    // behind the intentionally tiny actor queue and refuse the new join.
    loop {
        match host.recv_event(Duration::from_millis(25)) {
            Ok(_) => {}
            Err(error) if error.kind == crate::transport::TransportErrorKind::Timeout => break,
            Err(error) => panic!("unexpected actor drain failure: {error}"),
        }
    }
    let mut replacement = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), first_id.clone(), host.endpoint()),
            clock,
        )
        .expect("reconnect after Stop");
    // Drain the new connection's accepted telemetry before delivering join
    // to the deliberately small host queue.
    wait_until(Duration::from_secs(5), || {
        host.counters().accepted_control_connections >= 3
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing replacement accept");
        if let TransportEvent::PeerAccepted { peer, .. } =
            host.recv_event(remaining).expect("drain accepted event")
        {
            if peer.control_address.port() != 0 {
                break;
            }
        }
    }
    replacement
        .send_control(&join_request(&session, &first_id, "Reconnected"))
        .expect("replacement join");
    wait_for_control_from(&mut host, &first_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    let stopped_control_count = host.counters().control_frames_sent;
    host.authorize_peer(&first_id, replacement.local_routes())
        .expect("authorize replacement");
    wait_for_authorized(&mut host, &first_id);
    assert_eq!(
        host.counters().control_frames_sent,
        stopped_control_count,
        "Stop must clear cached Start/Pause for a reconnecting listener",
    );

    let replay = stream_start(&session, 2_000);
    assert_eq!(
        host.broadcast_control(&replay)
            .expect("fresh Play after Stop")
            .report
            .successful_peers,
        2,
    );
    for listener in [replacement.as_mut(), second.as_mut()] {
        wait_for_frame(listener, TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(replay.clone())
        });
    }
    replacement.shutdown().expect("close replacement");
    second.shutdown().expect("close second listener");
    host.shutdown().expect("close host");
}

#[test]
fn tcp_accept_survives_saturated_udp_telemetry_and_delivers_pcm() {
    let session = id_session("accept-pressure-playback");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("host");
    let first_id = id_device("pressure-accept-first");
    let mut first = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), first_id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("first listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("first accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    first
        .send_control(&join_request(&session, &first_id, "First"))
        .expect("join");
    wait_for_control_from(&mut host, &first_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&first_id, first.local_routes())
        .expect("authorize first");
    wait_for_authorized(&mut host, &first_id);

    for correlation_id in 0..300 {
        first
            .send_sync_request(&SyncRequest {
                session_id: session.clone(),
                correlation_id,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(correlation_id),
            })
            .expect("UDP telemetry");
    }
    wait_until(Duration::from_secs(5), || {
        host.counters().queue_overflows > 0
    });
    let second_id = id_device("pressure-accept-second");
    let mut second = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), second_id.clone(), host.endpoint()),
            clock,
        )
        .expect("second listener");
    wait_until(Duration::from_secs(5), || {
        host.counters().accepted_control_connections >= 2
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "PeerAccepted lost under UDP pressure");
        if matches!(
            host.recv_event(remaining).expect("drain telemetry"),
            TransportEvent::PeerAccepted { .. }
        ) {
            break;
        }
    }
    // Clear residual lossy telemetry before the join, which has a separate
    // explicit busy/refusal policy when its actor queue is saturated.
    loop {
        match host.recv_event(Duration::from_millis(25)) {
            Ok(_) => {}
            Err(error) if error.kind == crate::transport::TransportErrorKind::Timeout => break,
            Err(error) => panic!("unexpected event drain error: {error}"),
        }
    }
    second
        .send_control(&join_request(&session, &second_id, "Second"))
        .expect("second join");
    wait_for_control_from(&mut host, &second_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&second_id, second.local_routes())
        .expect("authorize second");
    wait_for_authorized(&mut host, &second_id);

    let start = stream_start(&session, 1_000);
    let pcm = audio_frame(&session, 0);
    assert_eq!(
        host.broadcast_control(&start)
            .expect("start")
            .report
            .successful_peers,
        2
    );
    for listener in [first.as_mut(), second.as_mut()] {
        wait_for_frame(listener, TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(start.clone())
        });
    }
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("PCM")
            .report
            .successful_peers,
        2
    );
    for listener in [first.as_mut(), second.as_mut()] {
        wait_for_frame(listener, TransportChannel::Audio, |frame| frame == &pcm);
    }
    assert_eq!(host.counters().delivery_failures, 0);
    first.shutdown().expect("first shutdown");
    second.shutdown().expect("second shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn stalled_actor_refuses_tcp_peer_without_ghost_route_or_harming_live_peer() {
    let session = id_session("accept-pressure-timeout");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.event_queue_capacity = 1;
    config.operation_timeout = Duration::from_millis(150);
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("host");
    let first_id = id_device("timeout-accept-first");
    let mut first = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), first_id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("first listener");
    assert!(matches!(
        host.recv_event(Duration::from_secs(5))
            .expect("first accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    first
        .send_control(&join_request(&session, &first_id, "First"))
        .expect("join");
    wait_for_control_from(&mut host, &first_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&first_id, first.local_routes())
        .expect("authorize first");
    wait_for_authorized(&mut host, &first_id);

    for correlation_id in 0..300 {
        first
            .send_sync_request(&SyncRequest {
                session_id: session.clone(),
                correlation_id,
                t1_listener_send_elapsed_ms: MonotonicMillis::new(correlation_id),
            })
            .expect("fill actor queue");
    }
    wait_until(Duration::from_secs(5), || {
        host.counters().queue_overflows > 0
    });
    let second_id = id_device("timeout-accept-second");
    let mut refused = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), second_id.clone(), host.endpoint()),
            clock,
        )
        .expect("second TCP connect");
    wait_until(Duration::from_secs(5), || {
        host.counters().accepted_control_connections >= 2
            && host.counters().closed_control_connections >= 1
            && host.counters().delivery_failures >= 1
    });
    assert_eq!(
        host.authorize_peer(&second_id, refused.local_routes())
            .expect_err("unobservable socket cannot be authorized")
            .kind,
        crate::transport::TransportErrorKind::PeerNotFound
    );
    let start = stream_start(&session, 1_000);
    assert_eq!(
        host.broadcast_control(&start)
            .expect("live peer remains authorized")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(first.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(start.clone())
    });
    drop(refused.shutdown());
    first.shutdown().expect("first shutdown");
    host.shutdown().expect("host shutdown");
}
