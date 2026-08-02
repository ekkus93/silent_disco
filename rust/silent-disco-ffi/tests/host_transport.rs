use silent_disco_core::runtime::current_protocol_version;
use silent_disco_ffi::{
    FfiHostTransportEvent, FfiHostTransportHandle, FfiListenerTransportEvent,
    FfiListenerTransportHandle,
};
use std::time::{SystemTime, UNIX_EPOCH};

const POLL_TIMEOUT_MS: u64 = 3_000;
const HOST_ADDRESS: &str = "127.0.0.1";
const CONTROL_PORT: u16 = 45_100;
const SYNC_PORT: u16 = 45_101;
const AUDIO_PORT: u16 = 45_102;
const SESSION_ID: &str = "host-transport-test-session";

fn now_wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_millis()
        .try_into()
        .expect("current time fits in a u64 millisecond count")
}

fn manual_endpoint_payload() -> String {
    format!(
        r#"{{"hostAddress":"{HOST_ADDRESS}","controlPort":{CONTROL_PORT},"syncPort":{SYNC_PORT},"audioPort":{AUDIO_PORT},"sessionId":"{SESSION_ID}","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
        current_protocol_version(),
    )
}

fn poll_host_until(
    host: &FfiHostTransportHandle,
    mut select: impl FnMut(&FfiHostTransportEvent) -> bool,
) -> FfiHostTransportEvent {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for a matching host transport event"
        );
        if let Some(event) = host
            .poll_event(POLL_TIMEOUT_MS)
            .expect("poll host transport event")
            && select(&event)
        {
            return event;
        }
    }
}

fn poll_listener_until(
    listener: &FfiListenerTransportHandle,
    mut select: impl FnMut(&FfiListenerTransportEvent) -> bool,
) -> FfiListenerTransportEvent {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for a matching listener transport event"
        );
        if let Some(event) = listener
            .poll_event(POLL_TIMEOUT_MS)
            .expect("poll listener transport event")
            && select(&event)
        {
            return event;
        }
    }
}

#[test]
fn host_transport_completes_join_request_through_approval_and_disconnect() {
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        CONTROL_PORT,
        SYNC_PORT,
        AUDIO_PORT,
        SESSION_ID.to_owned(),
    )
    .expect("bind host transport");

    let listener = FfiListenerTransportHandle::connect(
        manual_endpoint_payload(),
        now_wall_clock_ms(),
        "listener-device".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect listener transport");

    listener
        .send_join_request("Test Listener".to_owned(), None)
        .expect("send join request");

    let device_id = match poll_host_until(&host, |event| {
        matches!(event, FfiHostTransportEvent::JoinRequestReceived { .. })
    }) {
        FfiHostTransportEvent::JoinRequestReceived {
            device_id,
            display_name,
            invite_code,
        } => {
            assert_eq!(device_id, "listener-device");
            assert_eq!(display_name, "Test Listener");
            assert_eq!(invite_code, None);
            device_id
        }
        other => panic!("unexpected event: {other:?}"),
    };

    let approval_delivery = host
        .send_join_approval(device_id.clone(), true)
        .expect("send join approval");
    assert_eq!(approval_delivery.intended_peers, 1);
    assert_eq!(approval_delivery.successful_peers, 1);
    assert_eq!(approval_delivery.failed_peers, 0);

    let approved = poll_listener_until(&listener, |event| {
        matches!(event, FfiListenerTransportEvent::JoinApproved { .. })
    });
    assert_eq!(
        approved,
        FfiListenerTransportEvent::JoinApproved {
            trusted_for_future: true,
        }
    );

    let disconnect_delivery = host
        .disconnect_peer(device_id, "test complete".to_owned())
        .expect("disconnect peer");
    assert_eq!(disconnect_delivery.successful_peers, 1);

    let disconnected = poll_listener_until(&listener, |event| {
        matches!(
            event,
            FfiListenerTransportEvent::HostDisconnected { .. }
                | FfiListenerTransportEvent::ConnectionClosed { .. }
        )
    });
    match disconnected {
        FfiListenerTransportEvent::HostDisconnected { reason } => {
            assert_eq!(reason, "test complete");
        }
        other => panic!("unexpected disconnect event: {other:?}"),
    }

    listener.shutdown().expect("shutdown listener transport");
    host.shutdown().expect("shutdown host transport");
}

#[test]
fn host_transport_delivers_join_rejection() {
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        CONTROL_PORT + 10,
        SYNC_PORT + 10,
        AUDIO_PORT + 10,
        SESSION_ID.to_owned(),
    )
    .expect("bind host transport");

    let payload = format!(
        r#"{{"hostAddress":"{HOST_ADDRESS}","controlPort":{},"syncPort":{},"audioPort":{},"sessionId":"{SESSION_ID}","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
        CONTROL_PORT + 10,
        SYNC_PORT + 10,
        AUDIO_PORT + 10,
        current_protocol_version(),
    );
    let listener = FfiListenerTransportHandle::connect(
        payload,
        now_wall_clock_ms(),
        "rejected-listener".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect listener transport");

    listener
        .send_join_request("Rejected Listener".to_owned(), None)
        .expect("send join request");

    let device_id = match poll_host_until(&host, |event| {
        matches!(event, FfiHostTransportEvent::JoinRequestReceived { .. })
    }) {
        FfiHostTransportEvent::JoinRequestReceived { device_id, .. } => device_id,
        other => panic!("unexpected event: {other:?}"),
    };

    host.send_join_rejection(device_id, "session is full".to_owned())
        .expect("send join rejection");

    let rejected = poll_listener_until(&listener, |event| {
        matches!(event, FfiListenerTransportEvent::JoinRejected { .. })
    });
    assert_eq!(
        rejected,
        FfiListenerTransportEvent::JoinRejected {
            reason: "session is full".to_owned(),
        }
    );

    listener.shutdown().expect("shutdown listener transport");
    host.shutdown().expect("shutdown host transport");
}
