use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use silent_disco_core::domain::{DeviceId, SessionId};
use silent_disco_core::protocol::{ControlMessage, Disconnect, JoinRejection};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, SystemTransportClock, TransportFactory,
    production_transport_factory,
};
use silent_disco_ffi::{
    FfiListenerTransportError, FfiListenerTransportEvent, FfiListenerTransportHandle,
    parse_manual_host_endpoint,
};

const EVENT_TIMEOUT_MS: u64 = 5_000;

fn manual_payload(
    address: &str,
    control_port: u16,
    sync_port: u16,
    audio_port: u16,
    session_id: &str,
    protocol_version: u16,
) -> String {
    format!(
        r#"{{"hostAddress":"{address}","controlPort":{control_port},"syncPort":{sync_port},"audioPort":{audio_port},"sessionId":"{session_id}","protocolVersion":{protocol_version},"inviteCodeRequired":false,"expiresAtMs":null}}"#
    )
}

fn bind_loopback_host(session_id: &SessionId) -> Box<dyn HostTransportNode> {
    let clock = Arc::new(SystemTransportClock::default());
    production_transport_factory()
        .bind_host(HostTransportConfig::loopback(session_id.clone()), clock)
        .expect("loopback host should bind")
}

fn poll_until(
    handle: &FfiListenerTransportHandle,
    deadline: Instant,
    mut accept: impl FnMut(&FfiListenerTransportEvent) -> bool,
) -> FfiListenerTransportEvent {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for listener event");
        let remaining_ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
        if let Some(event) = handle
            .poll_event(remaining_ms.min(500))
            .expect("poll_event should not fail")
            && accept(&event)
        {
            return event;
        }
    }
}

fn current_protocol_version() -> u16 {
    silent_disco_core::runtime::current_protocol_version()
}

#[test]
fn parse_rejects_wrong_protocol_version_before_connecting() {
    let payload = manual_payload("127.0.0.1", 1, 2, 3, "session-x", 65_535);
    let error = parse_manual_host_endpoint(payload, 0).expect_err("version mismatch");
    assert!(matches!(
        error,
        FfiListenerTransportError::UnsupportedProtocolVersion(_)
    ));
}

#[test]
fn parse_rejects_malformed_address() {
    let payload = manual_payload(
        "not-an-ip",
        1,
        2,
        3,
        "session-x",
        current_protocol_version(),
    );
    let error = parse_manual_host_endpoint(payload, 0).expect_err("malformed address");
    assert!(matches!(
        error,
        FfiListenerTransportError::InvalidEndpoint(_)
    ));
}

#[test]
fn connect_fails_clearly_for_unroutable_endpoint() {
    // Bind and immediately drop a listener socket so the port is refused.
    let reserved = TcpListener::bind("127.0.0.1:0").expect("reserve a closed port");
    let port = reserved.local_addr().expect("local addr").port();
    drop(reserved);

    let payload = manual_payload(
        "127.0.0.1",
        port,
        port + 1,
        port + 2,
        "session-x",
        current_protocol_version(),
    );
    let error = FfiListenerTransportHandle::connect(
        payload,
        0,
        "listener-device".to_owned(),
        "0.0.0.0".to_owned(),
    )
    .expect_err("closed port should refuse connection");
    assert!(matches!(
        error,
        FfiListenerTransportError::ConnectionFailed(_)
    ));
}

#[test]
fn full_join_and_approval_flow_delivers_typed_events() {
    let session_id = SessionId::new("listener-transport-approval").expect("session id");
    let mut host = bind_loopback_host(&session_id);
    let endpoint = host.endpoint();

    let payload = manual_payload(
        &endpoint.address.to_string(),
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        session_id.as_str(),
        current_protocol_version(),
    );
    let listener = FfiListenerTransportHandle::connect(
        payload,
        0,
        "listener-approval-device".to_owned(),
        "127.0.0.1".to_owned(),
    )
    .expect("loopback connect should succeed");

    listener
        .send_join_request("Approval Listener".to_owned(), None)
        .expect("join request should send");

    let device_id = DeviceId::new("listener-approval-device").expect("device id");
    let deadline = Instant::now() + Duration::from_millis(EVENT_TIMEOUT_MS);
    wait_for_host_join_request(&mut *host, &device_id, deadline);

    let hello = ControlMessage::Hello(silent_disco_core::protocol::Hello {
        session_id: session_id.clone(),
        session_name: "Approval Session".to_owned(),
        host_name: "Desktop Host".to_owned(),
        approval_required: true,
    });
    host.send_pending_control(&device_id, &hello)
        .expect("host should deliver hello to the pending peer");
    let hello_event = poll_until(&listener, deadline, |event| {
        matches!(event, FfiListenerTransportEvent::Hello { .. })
    });
    assert!(matches!(
        hello_event,
        FfiListenerTransportEvent::Hello {
            approval_required: true,
            ..
        }
    ));

    let routes = silent_disco_core::transport::ListenerDatagramRoutes {
        synchronization: std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            listener
                .local_routes()
                .expect("routes")
                .local_synchronization_port,
        ),
        audio: std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            listener.local_routes().expect("routes").local_audio_port,
        ),
    };
    host.authorize_peer(&device_id, routes)
        .expect("host should authorize the peer");
    let approval = ControlMessage::JoinApproval(silent_disco_core::protocol::JoinApproval {
        session_id: session_id.clone(),
        listener_id: device_id.clone(),
        trusted_for_future: true,
    });
    host.send_control(&device_id, &approval)
        .expect("approval should be deliverable to the authorized peer");
    let approved = poll_until(&listener, deadline, |event| {
        matches!(event, FfiListenerTransportEvent::JoinApproved { .. })
    });
    assert_eq!(
        approved,
        FfiListenerTransportEvent::JoinApproved {
            trusted_for_future: true
        }
    );

    let disconnect = ControlMessage::Disconnect(Disconnect {
        session_id: session_id.clone(),
        listener_id: device_id.clone(),
        reason: "session_ended".to_owned(),
    });
    host.send_control(&device_id, &disconnect)
        .expect("disconnect should be deliverable");
    let disconnected = poll_until(&listener, deadline, |event| {
        matches!(event, FfiListenerTransportEvent::HostDisconnected { .. })
    });
    assert_eq!(
        disconnected,
        FfiListenerTransportEvent::HostDisconnected {
            reason: "session_ended".to_owned()
        }
    );

    listener.shutdown().expect("first shutdown should succeed");
    listener
        .shutdown()
        .expect("repeated shutdown should be idempotent");
    assert!(matches!(
        listener.poll_event(50),
        Err(FfiListenerTransportError::Closed(_))
    ));
    host.shutdown().expect("host should stop");
}

#[test]
fn rejection_is_delivered_as_a_distinct_event() {
    let session_id = SessionId::new("listener-transport-rejection").expect("session id");
    let mut host = bind_loopback_host(&session_id);
    let endpoint = host.endpoint();

    let payload = manual_payload(
        &endpoint.address.to_string(),
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        session_id.as_str(),
        current_protocol_version(),
    );
    let listener = FfiListenerTransportHandle::connect(
        payload,
        0,
        "listener-rejected-device".to_owned(),
        "127.0.0.1".to_owned(),
    )
    .expect("loopback connect should succeed");
    listener
        .send_join_request("Rejected Listener".to_owned(), None)
        .expect("join request should send");

    let device_id = DeviceId::new("listener-rejected-device").expect("device id");
    let deadline = Instant::now() + Duration::from_millis(EVENT_TIMEOUT_MS);
    wait_for_host_join_request(&mut *host, &device_id, deadline);

    let rejection = ControlMessage::JoinRejection(JoinRejection {
        session_id: session_id.clone(),
        listener_id: device_id.clone(),
        reason: "invite_code_invalid".to_owned(),
    });
    host.send_pending_control(&device_id, &rejection)
        .expect("rejection should reach the pending peer");
    let rejected = poll_until(&listener, deadline, |event| {
        matches!(event, FfiListenerTransportEvent::JoinRejected { .. })
    });
    assert_eq!(
        rejected,
        FfiListenerTransportEvent::JoinRejected {
            reason: "invite_code_invalid".to_owned()
        }
    );

    listener.shutdown().expect("shutdown should succeed");
    host.shutdown().expect("host should stop");
}

#[test]
fn host_closing_the_connection_surfaces_as_connection_closed() {
    let session_id = SessionId::new("listener-transport-closed").expect("session id");
    let mut host = bind_loopback_host(&session_id);
    let endpoint = host.endpoint();

    let payload = manual_payload(
        &endpoint.address.to_string(),
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        session_id.as_str(),
        current_protocol_version(),
    );
    let listener = FfiListenerTransportHandle::connect(
        payload,
        0,
        "listener-closed-device".to_owned(),
        "127.0.0.1".to_owned(),
    )
    .expect("loopback connect should succeed");

    host.shutdown()
        .expect("host shutdown should close the socket");
    let deadline = Instant::now() + Duration::from_millis(EVENT_TIMEOUT_MS);
    let closed = poll_until(&listener, deadline, |event| {
        matches!(event, FfiListenerTransportEvent::ConnectionClosed { .. })
    });
    assert!(matches!(
        closed,
        FfiListenerTransportEvent::ConnectionClosed { .. }
    ));

    listener
        .shutdown()
        .expect("listener shutdown should succeed");
}

fn wait_for_host_join_request(
    host: &mut dyn HostTransportNode,
    device_id: &DeviceId,
    deadline: Instant,
) {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for join request");
        if let Ok(silent_disco_core::transport::TransportEvent::FrameReceived {
            frame:
                silent_disco_core::protocol::ProtocolFrame::Control(ControlMessage::JoinRequest(
                    request,
                )),
            ..
        }) = host.recv_event(remaining.min(Duration::from_millis(200)))
            && &request.device.device_id == device_id
        {
            return;
        }
    }
}
