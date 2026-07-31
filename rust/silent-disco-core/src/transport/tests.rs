#![allow(clippy::too_many_lines)]
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::{
    DeliverySeverity, DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use crate::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, Heartbeat, Hello, JoinApproval,
    JoinRequest, PROTOCOL_MAGIC, ProtocolFrame, SyncRequest, SyncResponse,
};

use super::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, SystemTransportClock, TransportChannel,
    TransportErrorKind, TransportEvent, TransportFactory, VirtualTransportFactory,
    VirtualTransportNetwork, production_transport_factory,
};

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn production_factory_is_socket_runtime() {
    let _: SocketTransportFactory = production_transport_factory();
}

#[test]
fn socket_runtime_completes_multi_listener_join_sync_and_audio_exchange() {
    let session_id = id_session("socket-session");
    let clock = Arc::new(SystemTransportClock::default());
    let factory = SocketTransportFactory;
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host should bind all three loopback endpoints");
    let endpoint = host.endpoint();
    let device_a = id_device("listener-a");
    let device_b = id_device("listener-b");
    let mut listener_a = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), device_a.clone(), endpoint),
            clock.clone(),
        )
        .expect("first listener should connect");
    let mut listener_b = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), device_b.clone(), endpoint),
            clock,
        )
        .expect("second listener should connect");

    listener_a
        .send_control(&join_request(&session_id, &device_a, "Listener A"))
        .expect("first join request should reach host control socket");
    listener_b
        .send_control(&join_request(&session_id, &device_b, "Listener B"))
        .expect("second join request should reach host control socket");
    wait_for_control_from(&mut *host, &device_a, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    wait_for_control_from(&mut *host, &device_b, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    host.authorize_peer(&device_a, listener_a.local_routes())
        .expect("first peer routes should match authenticated control address");
    host.authorize_peer(&device_b, listener_b.local_routes())
        .expect("second peer routes should match authenticated control address");
    wait_for_authorized(&mut *host, &device_a);
    wait_for_authorized(&mut *host, &device_b);

    let hello = ControlMessage::Hello(Hello {
        session_id: session_id.clone(),
        session_name: "Loopback Session".to_owned(),
        host_name: "Loopback Host".to_owned(),
        approval_required: true,
    });
    let control_delivery = host
        .broadcast_control(&hello)
        .expect("broadcast control writes should report actual delivery");
    assert_eq!(control_delivery.report.intended_peers, 2);
    assert_eq!(control_delivery.report.successful_peers, 2);
    assert_eq!(control_delivery.report.failed_peers, 0);
    assert_eq!(control_delivery.report.severity, DeliverySeverity::Ok);
    wait_for_frame(&mut *listener_a, TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(hello.clone())
    });
    wait_for_frame(&mut *listener_b, TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(hello.clone())
    });

    for device in [&device_a, &device_b] {
        host.send_control(
            device,
            &ControlMessage::JoinApproval(JoinApproval {
                session_id: session_id.clone(),
                listener_id: device.clone(),
                trusted_for_future: false,
            }),
        )
        .expect("targeted approval should be delivered only to its listener");
    }
    wait_for_control_target(&mut *listener_a, &device_a);
    wait_for_control_target(&mut *listener_b, &device_b);

    let sync_request_a = SyncRequest {
        session_id: session_id.clone(),
        correlation_id: 11,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(100),
    };
    let sync_request_b = SyncRequest {
        session_id: session_id.clone(),
        correlation_id: 12,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(101),
    };
    listener_a
        .send_sync_request(&sync_request_a)
        .expect("first authorized listener should send synchronization request");
    listener_b
        .send_sync_request(&sync_request_b)
        .expect("second authorized listener should send synchronization request");
    wait_for_frame_from(
        &mut *host,
        TransportChannel::Synchronization,
        &device_a,
        |frame| frame == &ProtocolFrame::SyncRequest(sync_request_a.clone()),
    );
    wait_for_frame_from(
        &mut *host,
        TransportChannel::Synchronization,
        &device_b,
        |frame| frame == &ProtocolFrame::SyncRequest(sync_request_b.clone()),
    );

    let sync_response = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session_id.clone(),
        correlation_id: 90,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(100),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(110),
        t3_host_send_elapsed_ms: MonotonicMillis::new(111),
    });
    let sync_delivery = host
        .broadcast_sync(&sync_response)
        .expect("synchronization response should reach both registered routes");
    assert_eq!(sync_delivery.report.intended_peers, 2);
    assert_eq!(sync_delivery.report.successful_peers, 2);
    wait_for_frame(
        &mut *listener_a,
        TransportChannel::Synchronization,
        |frame| frame == &sync_response,
    );
    wait_for_frame(
        &mut *listener_b,
        TransportChannel::Synchronization,
        |frame| frame == &sync_response,
    );

    let audio = audio_frame(&session_id, 7);
    let audio_delivery = host
        .broadcast_audio(&audio)
        .expect("audio datagram should reach both registered routes");
    assert_eq!(audio_delivery.report.intended_peers, 2);
    assert_eq!(audio_delivery.report.successful_peers, 2);
    wait_for_frame(&mut *listener_a, TransportChannel::Audio, |frame| {
        frame == &audio
    });
    wait_for_frame(&mut *listener_b, TransportChannel::Audio, |frame| {
        frame == &audio
    });

    let counters = host.counters();
    assert!(counters.control_frames_received >= 2);
    assert!(counters.control_frames_sent >= 4);
    assert!(counters.sync_datagrams_received >= 2);
    assert!(counters.audio_datagrams_sent >= 2);
    assert!(counters.bytes_sent > 0);
    assert!(counters.bytes_received > 0);

    listener_a
        .shutdown()
        .expect("first listener workers should stop and join");
    listener_b
        .shutdown()
        .expect("second listener workers should stop and join");
    host.shutdown()
        .expect("host accept, socket, and peer workers should stop and join");
}

#[test]
fn socket_runtime_rejects_unauthorized_control_and_malformed_headers() {
    let session_id = id_session("rejection-session");
    let factory = SocketTransportFactory;
    let clock = Arc::new(SystemTransportClock::default());
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host should bind");
    let endpoint = host.endpoint();
    let device_id = id_device("unauthorized-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), endpoint),
            clock,
        )
        .expect("listener should connect");
    listener
        .send_control(&join_request(
            &session_id,
            &device_id,
            "Unauthorized Listener",
        ))
        .expect("join request is the one pre-authorization control message");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    listener
        .send_control(&ControlMessage::Heartbeat(Heartbeat {
            session_id: session_id.clone(),
            listener_id: device_id.clone(),
            sent_at_elapsed_ms: MonotonicMillis::new(5),
        }))
        .expect("TCP write may succeed before host authorization check");
    wait_for_rejection(&mut *host, TransportErrorKind::Unauthorized);

    let address = SocketAddr::new(endpoint.address, endpoint.control_port);
    let mut raw = TcpStream::connect(address).expect("raw malformed client should connect");
    raw.write_all(&invalid_version_header())
        .expect("raw malformed header should be written");
    drop(raw.shutdown(Shutdown::Both));
    wait_for_rejection(&mut *host, TransportErrorKind::Protocol);

    let mut oversized = TcpStream::connect(address).expect("raw oversized client should connect");
    oversized
        .write_all(&oversized_control_header())
        .expect("oversized header should be written");
    drop(oversized.shutdown(Shutdown::Both));
    wait_for_rejection(&mut *host, TransportErrorKind::Protocol);

    let mut truncated = TcpStream::connect(address).expect("raw truncated client should connect");
    truncated
        .write_all(&invalid_version_header()[..8])
        .expect("partial header should be written");
    drop(truncated.shutdown(Shutdown::Both));
    wait_for_rejection(&mut *host, TransportErrorKind::Protocol);

    let counters = host.counters();
    assert!(counters.unauthorized_frames >= 1);
    assert!(counters.malformed_frames + counters.oversized_frames >= 3);
    listener.shutdown().expect("listener should stop");
    host.shutdown().expect("host should stop");
}

#[test]
fn bounded_event_queue_reports_pressure_and_zero_peer_delivery_is_not_success() {
    let session_id = id_session("pressure-session");
    let factory = SocketTransportFactory;
    let mut config = HostTransportConfig::loopback(session_id.clone());
    config.event_queue_capacity = 1;
    let clock = Arc::new(SystemTransportClock::default());
    let mut host = factory
        .bind_host(config, clock.clone())
        .expect("host should bind with a one-event queue");
    let zero = host
        .broadcast_control(&ControlMessage::Hello(Hello {
            session_id: session_id.clone(),
            session_name: "Pressure Session".to_owned(),
            host_name: "Pressure Host".to_owned(),
            approval_required: false,
        }))
        .expect("zero-peer broadcast still returns explicit accounting");
    assert_eq!(zero.report.intended_peers, 0);
    assert_eq!(zero.report.severity, DeliverySeverity::ZeroPeers);

    let device_id = id_device("pressure-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("pressure listener should connect");
    listener
        .send_control(&join_request(&session_id, &device_id, "Pressure Listener"))
        .expect("join write should complete");
    wait_until(Duration::from_secs(2), || {
        host.counters().queue_overflows > 0
    });
    assert!(host.counters().queue_overflows > 0);
    listener.shutdown().expect("listener should stop");
    host.shutdown()
        .expect("host should stop under queue pressure");
}

#[test]
fn virtual_transport_is_explicit_isolated_and_uses_injected_clock() {
    let network_a = VirtualTransportNetwork::default();
    let network_b = VirtualTransportNetwork::default();
    let factory_a = VirtualTransportFactory::new(network_a);
    let factory_b = VirtualTransportFactory::new(network_b);
    let session_a = id_session("virtual-a");
    let session_b = id_session("virtual-b");
    let clock_a = Arc::new(ManualTransportClock::new(1_000));
    let clock_b = Arc::new(ManualTransportClock::new(9_000));
    let mut host_a = factory_a
        .bind_host(
            HostTransportConfig::loopback(session_a.clone()),
            clock_a.clone(),
        )
        .expect("first virtual host should bind");
    let mut host_b = factory_b
        .bind_host(HostTransportConfig::loopback(session_b.clone()), clock_b)
        .expect("second isolated virtual host should bind the same synthetic ports");
    assert_eq!(host_a.endpoint(), host_b.endpoint());

    let device_a = id_device("virtual-listener-a");
    let mut listener_a = factory_a
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_a.clone(),
                device_a.clone(),
                host_a.endpoint(),
            ),
            clock_a.clone(),
        )
        .expect("virtual listener should connect only inside its network");
    let accepted = host_a
        .recv_event(EVENT_TIMEOUT)
        .expect("virtual host should receive accepted event");
    assert!(matches!(
        accepted,
        TransportEvent::PeerAccepted {
            received_at,
            ..
        } if received_at == MonotonicMillis::new(1_000)
    ));
    listener_a
        .send_control(&join_request(&session_a, &device_a, "Virtual Listener"))
        .expect("virtual join should exercise canonical protocol round trip");
    wait_for_control_from(&mut *host_a, &device_a, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host_a
        .authorize_peer(&device_a, listener_a.local_routes())
        .expect("virtual peer should authorize against exact routes");
    wait_for_authorized(&mut *host_a, &device_a);
    clock_a.advance(25);
    listener_a
        .send_sync_request(&SyncRequest {
            session_id: session_a.clone(),
            correlation_id: 77,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1_025),
        })
        .expect("authorized virtual listener should send sync request");
    let event = wait_for_frame_from(
        &mut *host_a,
        TransportChannel::Synchronization,
        &device_a,
        |_| true,
    );
    assert!(matches!(
        event,
        TransportEvent::FrameReceived {
            received_at,
            ..
        } if received_at == MonotonicMillis::new(1_025)
    ));

    assert_eq!(
        host_b
            .broadcast_audio(&audio_frame(&session_b, 1))
            .expect("isolated host with no listeners returns explicit zero-peer report")
            .report
            .severity,
        DeliverySeverity::ZeroPeers
    );
    listener_a.shutdown().expect("virtual listener should stop");
    host_a.shutdown().expect("first virtual host should stop");
    host_b.shutdown().expect("second virtual host should stop");
}

fn join_request(
    session_id: &SessionId,
    device_id: &DeviceId,
    display_name: &str,
) -> ControlMessage {
    ControlMessage::JoinRequest(JoinRequest {
        session_id: session_id.clone(),
        device: DeviceIdentity {
            device_id: device_id.clone(),
            display_name: display_name.to_owned(),
        },
        invite_code: None,
    })
}

fn audio_frame(session_id: &SessionId, sequence: u64) -> ProtocolFrame {
    ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id.clone(),
        stream_id: StreamId::new("stream-1").expect("test stream ID is valid"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 2,
        first_sample_index: SampleIndex::new(sequence.saturating_mul(2)),
        host_presentation_time_ms: MonotonicMillis::new(500 + sequence),
        payload: vec![0, 0, 1, 0, 2, 0, 3, 0],
    })
}

fn wait_for_control_from(
    host: &mut dyn HostTransportNode,
    device_id: &DeviceId,
    predicate: impl Fn(&ControlMessage) -> bool,
) -> TransportEvent {
    wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                peer,
                frame: ProtocolFrame::Control(message),
                ..
            } if peer.device_id.as_ref() == Some(device_id) && predicate(message)
        )
    })
}

fn wait_for_authorized(host: &mut dyn HostTransportNode, device_id: &DeviceId) {
    drop(wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::PeerAuthorized { peer, .. }
                if peer.device_id.as_ref() == Some(device_id)
        )
    }));
}

fn wait_for_control_target(listener: &mut dyn ListenerTransportNode, device_id: &DeviceId) {
    drop(wait_for_listener_event(listener, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(value)),
                ..
            } if &value.listener_id == device_id
        )
    }));
}

fn wait_for_frame(
    listener: &mut dyn ListenerTransportNode,
    channel: TransportChannel,
    predicate: impl Fn(&ProtocolFrame) -> bool,
) -> TransportEvent {
    wait_for_listener_event(listener, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: actual,
                frame,
                ..
            } if *actual == channel && predicate(frame)
        )
    })
}

fn wait_for_frame_from(
    host: &mut dyn HostTransportNode,
    channel: TransportChannel,
    device_id: &DeviceId,
    predicate: impl Fn(&ProtocolFrame) -> bool,
) -> TransportEvent {
    wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: actual,
                peer,
                frame,
                ..
            } if *actual == channel
                && peer.device_id.as_ref() == Some(device_id)
                && predicate(frame)
        )
    })
}

fn wait_for_rejection(host: &mut dyn HostTransportNode, kind: TransportErrorKind) {
    drop(wait_for_host_event(
        host,
        |event| matches!(event, TransportEvent::Rejected { error, .. } if error.kind == kind),
    ));
}

fn wait_for_host_event(
    host: &mut dyn HostTransportNode,
    predicate: impl Fn(&TransportEvent) -> bool,
) -> TransportEvent {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for host transport event"
        );
        let event = host
            .recv_event(remaining)
            .expect("host event queue should remain connected");
        if predicate(&event) {
            return event;
        }
    }
}

fn wait_for_listener_event(
    listener: &mut dyn ListenerTransportNode,
    predicate: impl Fn(&TransportEvent) -> bool,
) -> TransportEvent {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for listener transport event"
        );
        let event = listener
            .recv_event(remaining)
            .expect("listener event queue should remain connected");
        if predicate(&event) {
            return event;
        }
    }
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition did not become true");
        thread::sleep(Duration::from_millis(10));
    }
}

fn invalid_version_header() -> [u8; 16] {
    let mut header = [0_u8; 16];
    header[..4].copy_from_slice(&PROTOCOL_MAGIC);
    header[4..6].copy_from_slice(&99_u16.to_be_bytes());
    header[6..8].copy_from_slice(&1_u16.to_be_bytes());
    header[10..12].copy_from_slice(&16_u16.to_be_bytes());
    header
}

fn oversized_control_header() -> [u8; 16] {
    let mut header = [0_u8; 16];
    header[..4].copy_from_slice(&PROTOCOL_MAGIC);
    header[4..6].copy_from_slice(&2_u16.to_be_bytes());
    header[6..8].copy_from_slice(&1_u16.to_be_bytes());
    header[10..12].copy_from_slice(&16_u16.to_be_bytes());
    header[12..16].copy_from_slice(&65_537_u32.to_be_bytes());
    header
}

fn id_session(value: &str) -> SessionId {
    SessionId::new(value).expect("test session ID is valid")
}

fn id_device(value: &str) -> DeviceId {
    DeviceId::new(value).expect("test device ID is valid")
}
