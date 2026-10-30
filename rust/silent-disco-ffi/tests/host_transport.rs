use silent_disco_core::runtime::current_protocol_version;
use silent_disco_ffi::{
    FfiHostTransportEvent, FfiHostTransportHandle, FfiListenerTransportEvent,
    FfiListenerTransportHandle,
};
use std::time::{SystemTime, UNIX_EPOCH};

const POLL_TIMEOUT_MS: u64 = 3_000;
const HOST_ADDRESS: &str = "127.0.0.1";
const SESSION_ID: &str = "host-transport-test-session";

/// Bind three independent ephemeral ports while holding the UDP reservations
/// together, so concurrent workspace test binaries cannot contend for a
/// hard-coded control or audio port.
fn available_host_ports() -> (u16, u16, u16) {
    let control = std::net::TcpListener::bind((HOST_ADDRESS, 0)).expect("reserve control port");
    let sync = std::net::UdpSocket::bind((HOST_ADDRESS, 0)).expect("reserve sync port");
    let audio = std::net::UdpSocket::bind((HOST_ADDRESS, 0)).expect("reserve audio port");
    (
        control.local_addr().unwrap().port(),
        sync.local_addr().unwrap().port(),
        audio.local_addr().unwrap().port(),
    )
}

fn now_wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_millis()
        .try_into()
        .expect("current time fits in a u64 millisecond count")
}

fn manual_endpoint_payload(
    control_port: u16,
    sync_port: u16,
    audio_port: u16,
    session_id: &str,
) -> String {
    format!(
        r#"{{"hostAddress":"{HOST_ADDRESS}","controlPort":{control_port},"syncPort":{sync_port},"audioPort":{audio_port},"sessionId":"{session_id}","protocolVersion":{},"inviteCodeRequired":false,"expiresAtMs":null}}"#,
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

fn assert_track_boundary_broadcast(
    host: &FfiHostTransportHandle,
    listener: &FfiListenerTransportHandle,
) {
    let delivery = host
        .broadcast_track_boundary(
            "stream-1".to_owned(),
            7_680,
            "entry-next".to_owned(),
            "Next Song".to_owned(),
        )
        .expect("broadcast sample-indexed track boundary");
    assert_eq!(delivery.intended_peers, 1);
    assert_eq!(delivery.successful_peers, 1);
    let boundary = poll_listener_until(listener, |event| {
        matches!(event, FfiListenerTransportEvent::TrackBoundary { .. })
    });
    assert_eq!(
        boundary,
        FfiListenerTransportEvent::TrackBoundary {
            stream_id: "stream-1".to_owned(),
            first_sample_index: 7_680,
            entry_id: "entry-next".to_owned(),
            display_name: "Next Song".to_owned(),
        }
    );
}

/// `received_at_elapsed_ms` is a real clock reading taken when the response
/// was pulled off the socket, not a value a test can predict -- the wire
/// fields are checked exactly, and the new one only for sanity (present,
/// and not absurdly large for a test that runs in well under a second).
fn assert_sync_response_matches(event: &FfiListenerTransportEvent) {
    match *event {
        FfiListenerTransportEvent::SyncResponseReceived {
            correlation_id,
            t1_listener_send_elapsed_ms,
            t2_host_receive_elapsed_ms,
            t3_host_send_elapsed_ms,
            received_at_elapsed_ms,
        } => {
            assert_eq!(correlation_id, 42);
            assert_eq!(t1_listener_send_elapsed_ms, 1_000);
            assert_eq!(t2_host_receive_elapsed_ms, 1_005);
            assert_eq!(t3_host_send_elapsed_ms, 1_007);
            assert!(
                received_at_elapsed_ms < 60_000,
                "received_at_elapsed_ms should be a small elapsed-since-connect reading for a \
                 fast test, got {received_at_elapsed_ms}"
            );
        }
        ref other => panic!("expected SyncResponseReceived, got {other:?}"),
    }
}

#[test]
fn host_transport_completes_join_request_through_approval_and_disconnect() {
    let (control_port, sync_port, audio_port) = available_host_ports();
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        control_port,
        sync_port,
        audio_port,
        SESSION_ID.to_owned(),
    )
    .expect("bind host transport");

    let listener = FfiListenerTransportHandle::connect(
        manual_endpoint_payload(control_port, sync_port, audio_port, SESSION_ID),
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
            sync_port,
            audio_port,
        } => {
            assert_eq!(device_id, "listener-device");
            assert_eq!(display_name, "Test Listener");
            assert_eq!(invite_code, None);
            assert_ne!(
                sync_port, 0,
                "listener should report a real bound sync port"
            );
            assert_ne!(
                audio_port, 0,
                "listener should report a real bound audio port"
            );
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
fn host_transport_targets_current_stream_and_boundary_to_late_joiner() {
    let (control_port, sync_port, audio_port) = available_host_ports();
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        control_port,
        sync_port,
        audio_port,
        "late-join-session".to_owned(),
    )
    .expect("bind host transport");
    let endpoint =
        manual_endpoint_payload(control_port, sync_port, audio_port, "late-join-session");
    let listener = FfiListenerTransportHandle::connect(
        endpoint,
        now_wall_clock_ms(),
        "late-listener".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect late listener");
    listener
        .send_join_request("Late Listener".to_owned(), None)
        .expect("send join request");
    let (listener_id, sync_port, audio_port) = match poll_host_until(&host, |event| {
        matches!(event, FfiHostTransportEvent::JoinRequestReceived { .. })
    }) {
        FfiHostTransportEvent::JoinRequestReceived {
            device_id,
            sync_port,
            audio_port,
            ..
        } => (device_id, sync_port, audio_port),
        other => panic!("unexpected event: {other:?}"),
    };
    host.send_join_approval(listener_id.clone(), false)
        .expect("approve listener");
    poll_listener_until(&listener, |event| {
        matches!(event, FfiListenerTransportEvent::JoinApproved { .. })
    });
    host.authorize_listener(listener_id.clone(), sync_port, audio_port)
        .expect("authorize listener datagram routes");

    let start_delivery = host
        .send_stream_start_to_listener(
            listener_id.clone(),
            "active-stream".to_owned(),
            1_000,
            48_000,
            2,
            240,
            Some("entry-current".to_owned()),
            Some("Current song".to_owned()),
        )
        .expect("send current stream state");
    assert_eq!(start_delivery.successful_peers, 1);
    assert_eq!(
        poll_listener_until(&listener, |event| {
            matches!(event, FfiListenerTransportEvent::StreamStarted { .. })
        }),
        FfiListenerTransportEvent::StreamStarted {
            stream_id: "active-stream".to_owned(),
            host_start_time_ms: 1_000,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 240,
            current_entry_id: Some("entry-current".to_owned()),
            current_display_name: Some("Current song".to_owned()),
        }
    );
    let boundary_delivery = host
        .send_track_boundary_to_listener(
            listener_id,
            "active-stream".to_owned(),
            24_000,
            "entry-next".to_owned(),
            "Next song".to_owned(),
        )
        .expect("send upcoming boundary");
    assert_eq!(boundary_delivery.successful_peers, 1);
    assert_eq!(
        poll_listener_until(&listener, |event| {
            matches!(event, FfiListenerTransportEvent::TrackBoundary { .. })
        }),
        FfiListenerTransportEvent::TrackBoundary {
            stream_id: "active-stream".to_owned(),
            first_sample_index: 24_000,
            entry_id: "entry-next".to_owned(),
            display_name: "Next song".to_owned(),
        }
    );

    listener.shutdown().expect("shutdown listener");
    host.shutdown().expect("shutdown host transport");
}

#[test]
fn host_transport_delivers_join_rejection() {
    let (control_port, sync_port, audio_port) = available_host_ports();
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        control_port,
        sync_port,
        audio_port,
        SESSION_ID.to_owned(),
    )
    .expect("bind host transport");

    let payload = manual_endpoint_payload(control_port, sync_port, audio_port, SESSION_ID);
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

/// Drives a join through approval and datagram authorization, returning the
/// approved listener's device ID.
fn join_approve_and_authorize(
    host: &FfiHostTransportHandle,
    listener: &FfiListenerTransportHandle,
    display_name: &str,
) -> String {
    listener
        .send_join_request(display_name.to_owned(), None)
        .expect("send join request");

    let (device_id, sync_port, audio_port) = match poll_host_until(host, |event| {
        matches!(event, FfiHostTransportEvent::JoinRequestReceived { .. })
    }) {
        FfiHostTransportEvent::JoinRequestReceived {
            device_id,
            sync_port,
            audio_port,
            ..
        } => (device_id, sync_port, audio_port),
        other => panic!("unexpected event: {other:?}"),
    };

    host.send_join_approval(device_id.clone(), false)
        .expect("send join approval");
    poll_listener_until(listener, |event| {
        matches!(event, FfiListenerTransportEvent::JoinApproved { .. })
    });

    host.authorize_listener(device_id.clone(), sync_port, audio_port)
        .expect("authorize listener for datagram routing");
    device_id
}

#[test]
fn host_transport_authorizes_listener_and_exchanges_sync_and_audio() {
    let (control_port, sync_port, audio_port) = available_host_ports();
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        control_port,
        sync_port,
        audio_port,
        SESSION_ID.to_owned(),
    )
    .expect("bind host transport");

    let payload = manual_endpoint_payload(control_port, sync_port, audio_port, SESSION_ID);
    let listener = FfiListenerTransportHandle::connect(
        payload,
        now_wall_clock_ms(),
        "sync-audio-listener".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect listener transport");

    let listener_id = join_approve_and_authorize(&host, &listener, "Sync Audio Listener");

    listener
        .send_sync_request(42, 1_000)
        .expect("send sync request");

    let (correlation_id, t1) = match poll_host_until(&host, |event| {
        matches!(event, FfiHostTransportEvent::SyncRequestReceived { .. })
    }) {
        FfiHostTransportEvent::SyncRequestReceived {
            correlation_id,
            t1_listener_send_elapsed_ms,
            ..
        } => (correlation_id, t1_listener_send_elapsed_ms),
        other => panic!("unexpected event: {other:?}"),
    };
    assert_eq!(correlation_id, 42);
    assert_eq!(t1, 1_000);

    // Legacy broadcast must fail closed: a response contains one listener's
    // private correlation/timestamps and may only use its authorized route.
    assert!(
        host.send_sync_response(correlation_id, t1, 1_005, 1_007)
            .is_err(),
        "broadcasting a host-originated sync response must be forbidden"
    );
    let delivery = host
        .send_sync_response_to_listener(listener_id, correlation_id, t1, 1_005, 1_007)
        .expect("send targeted sync response");
    assert_eq!(delivery.intended_peers, 1);
    assert_eq!(delivery.successful_peers, 1);

    let sync_response = poll_listener_until(&listener, |event| {
        matches!(
            event,
            FfiListenerTransportEvent::SyncResponseReceived { .. }
        )
    });
    assert_sync_response_matches(&sync_response);

    // A track boundary is only valid after StreamStart has established the
    // active timeline. Exercise that real protocol sequence rather than
    // relying on the transport to forward an orphan boundary.
    let stream_start_delivery = host
        .broadcast_stream_start("stream-1".to_owned(), 1_000, 48_000, 2, 960, None, None)
        .expect("broadcast stream start");
    assert_eq!(stream_start_delivery.intended_peers, 1);
    assert_eq!(stream_start_delivery.successful_peers, 1);
    poll_listener_until(&listener, |event| {
        matches!(event, FfiListenerTransportEvent::StreamStarted { .. })
    });

    assert_track_boundary_broadcast(&host, &listener);

    // PCM16 stereo payload size must equal samples_per_packet * channels * 2 bytes.
    let payload_bytes: Vec<u8> = (0..3_840_u32)
        .map(|index| u8::try_from(index % 256).unwrap_or(0))
        .collect();
    let delivery = host
        .broadcast_audio(
            "stream-1".to_owned(),
            7,
            48_000,
            2,
            960,
            6_720,
            5_000,
            payload_bytes.clone(),
        )
        .expect("broadcast audio");
    assert_eq!(delivery.intended_peers, 1);
    assert_eq!(delivery.successful_peers, 1);
    assert_eq!(delivery.failed_peers, 0);

    let audio_event = poll_listener_until(&listener, |event| {
        matches!(event, FfiListenerTransportEvent::AudioReceived { .. })
    });
    assert_eq!(
        audio_event,
        FfiListenerTransportEvent::AudioReceived {
            stream_id: "stream-1".to_owned(),
            sequence: 7,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 960,
            first_sample_index: 6_720,
            host_presentation_time_ms: 5_000,
            payload: payload_bytes,
        }
    );

    listener.shutdown().expect("shutdown listener transport");
    host.shutdown().expect("shutdown host transport");
}
