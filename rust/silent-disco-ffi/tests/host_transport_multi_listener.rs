use silent_disco_core::runtime::current_protocol_version;
use silent_disco_ffi::{
    FfiHostTransportEvent, FfiHostTransportHandle, FfiListenerTransportEvent,
    FfiListenerTransportHandle,
};
use std::time::{SystemTime, UNIX_EPOCH};

const POLL_TIMEOUT_MS: u64 = 3_000;
const HOST_ADDRESS: &str = "127.0.0.1";

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

fn assert_stream_start_and_boundary(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    listener_b: &FfiListenerTransportHandle,
) {
    let start = host
        .broadcast_stream_start(
            "ffi-multi-stream".to_owned(),
            1_000,
            48_000,
            2,
            240,
            Some("first".to_owned()),
            Some("First track".to_owned()),
        )
        .expect("broadcast authoritative stream");
    assert_eq!(start.successful_peers, 2);
    for listener in [listener_a, listener_b] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::StreamStarted { .. }
            )),
            FfiListenerTransportEvent::StreamStarted {
                stream_id,
                current_entry_id: Some(entry),
                ..
            } if stream_id == "ffi-multi-stream" && entry == "first"
        ));
    }
    assert_eq!(
        host.broadcast_track_boundary(
            "ffi-multi-stream".to_owned(),
            240,
            "second".to_owned(),
            "Second track".to_owned(),
        )
        .expect("announce sample-indexed boundary")
        .successful_peers,
        2
    );
    for listener in [listener_a, listener_b] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::TrackBoundary { .. }
            )),
            FfiListenerTransportEvent::TrackBoundary {
                first_sample_index: 240,
                ..
            }
        ));
    }

}

fn assert_targeted_clock_sync(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    listener_b: &FfiListenerTransportHandle,
    a_id: &str,
) {
    // The FFI production transport must preserve per-listener correlation:
    // B must not observe A's synchronization datagram.
    listener_a
        .send_sync_request(71, 7_100)
        .expect("send A's clock probe");
    let request = poll_host_until(host, |event| {
        matches!(event, FfiHostTransportEvent::SyncRequestReceived { .. })
    });
    assert!(matches!(
        request,
        FfiHostTransportEvent::SyncRequestReceived {
            listener_id: Some(ref id),
            correlation_id: 71,
            ..
        } if id.as_str() == a_id
    ));
    assert_eq!(
        host.send_sync_response_to_listener(a_id.to_owned(), 71, 7_100, 7_101, 7_102)
            .expect("target A only")
            .successful_peers,
        1
    );
    assert!(matches!(
        poll_listener_until(listener_a, |event| matches!(
            event,
            FfiListenerTransportEvent::SyncResponseReceived { .. }
        )),
        FfiListenerTransportEvent::SyncResponseReceived {
            correlation_id: 71,
            ..
        }
    ));
    assert!(
        listener_b
            .poll_event(150)
            .expect("poll unrelated listener")
            .is_none(),
        "listener B must not receive A's private sync response"
    );
    assert!(
        host.send_sync_response_to_listener("unknown-ffi-peer".to_owned(), 72, 7_100, 7_101, 7_102)
            .is_err(),
        "unknown peer cannot receive a targeted synchronization response"
    );

}

fn assert_pause_recovery(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    listener_b: &FfiListenerTransportHandle,
    a_id: &str,
) {
    // A pause and explicit recovery replay must preserve the same stream,
    // boundary sample index, and paused timeline.
    assert_eq!(
        host.broadcast_pause("ffi-multi-stream".to_owned(), 1_250)
            .expect("pause both listeners")
            .successful_peers,
        2
    );
    for listener in [listener_a, listener_b] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::Paused { .. }
            )),
            FfiListenerTransportEvent::Paused {
                host_pause_time_ms: 1_250,
                ..
            }
        ));
    }
    host.replay_cached_playback_to_listener(a_id.to_owned())
        .expect("replay authoritative paused state");
    assert!(matches!(
        poll_listener_until(listener_a, |event| matches!(
            event,
            FfiListenerTransportEvent::StreamStarted { .. }
        )),
        FfiListenerTransportEvent::StreamStarted { stream_id, .. }
            if stream_id == "ffi-multi-stream"
    ));
    assert!(matches!(
        poll_listener_until(listener_a, |event| matches!(
            event,
            FfiListenerTransportEvent::TrackBoundary { .. }
        )),
        FfiListenerTransportEvent::TrackBoundary {
            first_sample_index: 240,
            ..
        }
    ));
    assert!(matches!(
        poll_listener_until(listener_a, |event| matches!(
            event,
            FfiListenerTransportEvent::Paused { .. }
        )),
        FfiListenerTransportEvent::Paused {
            host_pause_time_ms: 1_250,
            ..
        }
    ));

}

fn assert_terminal_stop(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    listener_b: &FfiListenerTransportHandle,
) {
    let stopped = host
        .broadcast_stop("ffi-multi-stream".to_owned(), 1_300)
        .expect("stop original stream");
    assert_eq!(stopped.successful_peers, 2);
    for listener in [listener_a, listener_b] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::Stopped { .. }
            )),
            FfiListenerTransportEvent::Stopped {
                host_stop_time_ms: 1_300,
                ..
            }
        ));
    }

}

fn assert_fresh_pcm(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    listener_b: &FfiListenerTransportHandle,
) {
    // A new Play creates a fresh stream identity; the new PCM must arrive
    // exactly at its new sample origin rather than from the stopped cache.
    assert_eq!(
        host.broadcast_stream_start(
            "ffi-fresh-stream".to_owned(),
            2_000,
            48_000,
            2,
            240,
            Some("replay".to_owned()),
            Some("Replay".to_owned()),
        )
        .expect("fresh Play")
        .successful_peers,
        2
    );
    for listener in [listener_a, listener_b] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::StreamStarted { .. }
            )),
            FfiListenerTransportEvent::StreamStarted { stream_id, .. }
                if stream_id == "ffi-fresh-stream"
        ));
    }
    let pcm: Vec<u8> = (0..960_u32)
        .map(|index| u8::try_from((index * 13) % 256).expect("byte"))
        .collect();
    assert_eq!(
        host.broadcast_audio(
            "ffi-fresh-stream".to_owned(),
            0,
            48_000,
            2,
            240,
            0,
            2_000,
            pcm.clone(),
        )
        .expect("send fresh PCM")
        .successful_peers,
        2
    );
    for listener in [listener_a, listener_b] {
        assert_eq!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::AudioReceived { .. }
            )),
            FfiListenerTransportEvent::AudioReceived {
                stream_id: "ffi-fresh-stream".to_owned(),
                sequence: 0,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 240,
                first_sample_index: 0,
                host_presentation_time_ms: 2_000,
                payload: pcm.clone(),
            }
        );
    }
}

/// A third listener joins after playback has already started and receives
/// the cached authoritative stream state before the next exact PCM packet.
fn assert_late_join_recovery(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    listener_b: &FfiListenerTransportHandle,
    endpoint: &str,
) {
    let late = FfiListenerTransportHandle::connect(
        endpoint.to_owned(),
        now_wall_clock_ms(),
        "ffi-peer-late".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect late listener");
    let late_id = join_approve_and_authorize(host, &late, "Late Listener");
    host.replay_cached_playback_to_listener(late_id.clone())
        .expect("send cached stream to late listener");
    assert!(matches!(
        poll_listener_until(&late, |event| matches!(
            event,
            FfiListenerTransportEvent::StreamStarted { .. }
        )),
        FfiListenerTransportEvent::StreamStarted { stream_id, .. }
            if stream_id == "ffi-fresh-stream"
    ));

    let pcm: Vec<u8> = (0..960_u32)
        .map(|index| u8::try_from((index * 17) % 256).expect("byte"))
        .collect();
    assert_eq!(
        host.broadcast_audio(
            "ffi-fresh-stream".to_owned(),
            1,
            48_000,
            2,
            240,
            240,
            2_005,
            pcm.clone(),
        )
        .expect("broadcast next exact PCM to late joiner")
        .successful_peers,
        3
    );
    for listener in [listener_a, listener_b, &late] {
        assert_eq!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::AudioReceived { .. }
            )),
            FfiListenerTransportEvent::AudioReceived {
                stream_id: "ffi-fresh-stream".to_owned(),
                sequence: 1,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 240,
                first_sample_index: 240,
                host_presentation_time_ms: 2_005,
                payload: pcm.clone(),
            }
        );
    }
    assert_eq!(
        host.disconnect_peer(late_id, "late listener leaving".to_owned())
            .expect("remove late listener")
            .successful_peers,
        1
    );
    poll_listener_until(&late, |event| matches!(
        event,
        FfiListenerTransportEvent::HostDisconnected { .. }
    ));
    late.shutdown().expect("close late listener");
}

/// A replaced TCP socket with the same logical ID must receive a fresh
/// authorization and explicit cache replay; surviving listeners remain live.
fn assert_reconnect_and_resume_pcm(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    endpoint: &str,
    reconnect_id: &str,
) {
    let replacement = FfiListenerTransportHandle::connect(
        endpoint.to_owned(),
        now_wall_clock_ms(),
        reconnect_id.to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("reconnect previous listener");
    let admitted = join_approve_and_authorize(host, &replacement, "Listener B Reconnected");
    assert_eq!(admitted, reconnect_id);
    host.replay_cached_playback_to_listener(admitted)
        .expect("replay current stream to reconnected listener");
    assert!(matches!(
        poll_listener_until(&replacement, |event| matches!(
            event,
            FfiListenerTransportEvent::StreamStarted { .. }
        )),
        FfiListenerTransportEvent::StreamStarted { stream_id, .. }
            if stream_id == "ffi-fresh-stream"
    ));

    let pcm: Vec<u8> = (0..960_u32)
        .map(|index| u8::try_from((index * 19) % 256).expect("byte"))
        .collect();
    assert_eq!(
        host.broadcast_audio(
            "ffi-fresh-stream".to_owned(),
            2,
            48_000,
            2,
            240,
            480,
            2_010,
            pcm.clone(),
        )
        .expect("send post-reconnect PCM")
        .successful_peers,
        2
    );
    for listener in [listener_a, &replacement] {
        assert_eq!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::AudioReceived { .. }
            )),
            FfiListenerTransportEvent::AudioReceived {
                stream_id: "ffi-fresh-stream".to_owned(),
                sequence: 2,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 240,
                first_sample_index: 480,
                host_presentation_time_ms: 2_010,
                payload: pcm.clone(),
            }
        );
    }
    replacement.shutdown().expect("close reconnected listener");
}

#[test]
fn ffi_two_listener_targeted_sync_pause_recovery_stop_and_fresh_pcm() {
    let (control_port, sync_port, audio_port) = available_host_ports();
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        control_port,
        sync_port,
        audio_port,
        "ffi-multi-listener-session".to_owned(),
    )
    .expect("bind host");
    let endpoint = manual_endpoint_payload(
        control_port,
        sync_port,
        audio_port,
        "ffi-multi-listener-session",
    );
    let listener_a = FfiListenerTransportHandle::connect(
        endpoint.clone(),
        now_wall_clock_ms(),
        "ffi-peer-a".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect first listener");
    let listener_b = FfiListenerTransportHandle::connect(
        endpoint.clone(),
        now_wall_clock_ms(),
        "ffi-peer-b".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect second listener");
    let a_id = join_approve_and_authorize(&host, &listener_a, "Listener A");
    let b_id = join_approve_and_authorize(&host, &listener_b, "Listener B");

    assert_stream_start_and_boundary(&host, &listener_a, &listener_b);
    assert_targeted_clock_sync(&host, &listener_a, &listener_b, &a_id);
    assert_pause_recovery(&host, &listener_a, &listener_b, &a_id);
    assert_terminal_stop(&host, &listener_a, &listener_b);
    assert_fresh_pcm(&host, &listener_a, &listener_b);
    assert_late_join_recovery(&host, &listener_a, &listener_b, &endpoint);
    assert_eq!(
        host.disconnect_peer(b_id.clone(), "peer B leaving".to_owned())
            .expect("disconnect B")
            .successful_peers,
        1
    );
    assert!(matches!(
        poll_listener_until(&listener_b, |event| matches!(
            event,
            FfiListenerTransportEvent::HostDisconnected { .. }
        )),
        FfiListenerTransportEvent::HostDisconnected { .. }
    ));
    listener_b.shutdown().expect("close listener B");
    assert_reconnect_and_resume_pcm(&host, &listener_a, &endpoint, &b_id);
    listener_a.shutdown().expect("close listener A");
    host.shutdown().expect("close host");
}
