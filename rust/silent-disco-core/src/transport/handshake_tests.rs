#![allow(clippy::too_many_lines)]
use std::sync::Arc;

use crate::domain::{DeliverySeverity, MonotonicMillis};
use crate::protocol::{
    ControlMessage, Hello, JoinApproval, ProtocolFrame, SyncRequest, SyncResponse,
};

use super::test_support::{
    audio_frame, id_device, id_session, join_request, wait_for_authorized, wait_for_control_from,
    wait_for_control_target, wait_for_frame, wait_for_frame_from,
};
use super::{
    HostTransportConfig, ListenerTransportConfig, SocketTransportFactory, SystemTransportClock,
    TransportChannel, TransportFactory, production_transport_factory,
};

#[test]
fn production_factory_is_socket_runtime() {
    let _: SocketTransportFactory = production_transport_factory();
}

#[test]
fn pending_control_peer_receives_hello_before_datagram_authorization() {
    let session_id = id_session("manual-endpoint-session");
    let device_id = id_device("manual-endpoint-listener");
    let factory = SocketTransportFactory;
    let clock = Arc::new(SystemTransportClock::default());
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("manual endpoint host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("manual endpoint listener should connect");

    listener
        .send_control(&join_request(
            &session_id,
            &device_id,
            "Manual Endpoint Listener",
        ))
        .expect("join request should reach the host");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let hello = ControlMessage::Hello(Hello {
        session_id: session_id.clone(),
        session_name: "Manual Endpoint Session".to_owned(),
        host_name: "Desktop Host".to_owned(),
        approval_required: true,
    });
    let delivery = host
        .send_pending_control(&device_id, &hello)
        .expect("identified pending peer should receive TCP Hello");
    assert_eq!(delivery.report.intended_peers, 1);
    assert_eq!(delivery.report.successful_peers, 1);
    wait_for_frame(&mut *listener, TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(hello.clone())
    });

    assert_eq!(host.counters().audio_datagrams_sent, 0);
    assert_eq!(listener.counters().audio_datagrams_received, 0);
    listener.shutdown().expect("listener should stop");
    host.shutdown().expect("host should stop");
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
    wait_for_authorized(&mut *host, &device_a);
    host.authorize_peer(&device_b, listener_b.local_routes())
        .expect("second peer routes should match authenticated control address");
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
