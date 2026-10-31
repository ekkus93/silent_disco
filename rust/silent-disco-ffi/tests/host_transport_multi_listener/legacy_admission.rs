use super::*;
use silent_disco_ffi::FfiHostTransportError;

/// A legacy plaintext host must not advertise permanent trust on the wire.
/// Until a verified v4 identity exists, pending and rejected peers must not
/// receive the authorized listener's playback or targeted synchronization.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "integrated legacy admission, denied peer isolation, Stop and fresh real-socket PCM"
)]
fn legacy_admission_rejects_durable_trust_and_isolates_pending_and_rejected_peers() {
    let (control_port, sync_port, audio_port) = available_host_ports();
    let session = "ffi-legacy-admission";
    let host = FfiHostTransportHandle::bind(
        HOST_ADDRESS.to_owned(),
        control_port,
        sync_port,
        audio_port,
        session.to_owned(),
    )
    .expect("bind legacy host");
    let endpoint = manual_endpoint_payload(control_port, sync_port, audio_port, session);
    let approved = FfiListenerTransportHandle::connect(
        endpoint.clone(),
        now_wall_clock_ms(),
        "legacy-approved".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect approved listener");
    join_approve_and_authorize(&host, &approved, "Approved listener");

    let pending = FfiListenerTransportHandle::connect(
        endpoint.clone(),
        now_wall_clock_ms(),
        "legacy-pending".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect pending listener");
    pending
        .send_join_request("Pending listener".to_owned(), None)
        .expect("request pending admission");
    let pending_id = match poll_host_until(&host, |event| {
        matches!(event, FfiHostTransportEvent::JoinRequestReceived { .. })
    }) {
        FfiHostTransportEvent::JoinRequestReceived { device_id, .. } => device_id,
        other => panic!("expected pending join, got {other:?}"),
    };
    let denied = host
        .send_join_approval(pending_id.clone(), true)
        .expect_err("legacy plaintext must never grant remembered trust");
    assert!(
        matches!(denied, FfiHostTransportError::Unauthorized(message) if message.contains("verified cryptographic identity")),
        "permanent trust requires a verified key, not a claimed device ID"
    );
    assert!(
        pending.poll_event(150).expect("pending poll").is_none(),
        "rejected durable approval must not emit a misleading JoinApproved"
    );

    assert_eq!(
        host.broadcast_stream_start(
            "admission-stream".to_owned(),
            1_000,
            48_000,
            2,
            240,
            Some("first".to_owned()),
            Some("First".to_owned()),
        )
        .expect("start authorized playback")
        .successful_peers,
        1
    );
    assert!(matches!(
        poll_listener_until(&approved, |event| matches!(
            event,
            FfiListenerTransportEvent::StreamStarted { .. }
        )),
        FfiListenerTransportEvent::StreamStarted { stream_id, .. }
            if stream_id == "admission-stream"
    ));
    assert!(
        pending
            .poll_event(150)
            .expect("pending control poll")
            .is_none(),
        "unapproved listener cannot receive StreamStart"
    );

    let first_pcm: Vec<u8> = (0..960_u32)
        .map(|index| u8::try_from(index % 251).expect("PCM byte"))
        .collect();
    assert_eq!(
        host.broadcast_audio(
            "admission-stream".to_owned(),
            0,
            48_000,
            2,
            240,
            0,
            1_000,
            first_pcm.clone(),
        )
        .expect("send authorized PCM")
        .successful_peers,
        1
    );
    assert!(matches!(
        poll_listener_until(&approved, |event| matches!(
            event,
            FfiListenerTransportEvent::AudioReceived { .. }
        )),
        FfiListenerTransportEvent::AudioReceived {
            stream_id,
            sequence: 0,
            first_sample_index: 0,
            host_presentation_time_ms: 1_000,
            payload,
            ..
        } if stream_id == "admission-stream" && payload == first_pcm
    ));
    assert!(
        pending
            .poll_event(150)
            .expect("pending audio poll")
            .is_none(),
        "unapproved listener cannot receive PCM"
    );
    assert!(
        host.send_sync_response_to_listener(pending_id.clone(), 7, 10, 11, 12)
            .is_err(),
        "unapproved listener cannot receive targeted clock synchronization"
    );

    assert_eq!(
        host.send_join_rejection(pending_id.clone(), "Manual admission denied".to_owned())
            .expect("reject pending listener")
            .successful_peers,
        1
    );
    assert!(matches!(
        poll_listener_until(&pending, |event| matches!(
            event,
            FfiListenerTransportEvent::JoinRejected { .. }
        )),
        FfiListenerTransportEvent::JoinRejected { reason }
            if reason == "Manual admission denied"
    ));
    assert!(
        host.disconnect_peer(pending_id.clone(), "Admission rejected".to_owned())
            .is_err(),
        "a rejected pending peer cannot be promoted or targeted as an admitted route"
    );
    pending.shutdown().expect("close rejected listener");
    assert!(matches!(
        poll_host_until(&host, |event| matches!(
            event,
            FfiHostTransportEvent::PeerDisconnected {
                listener_id: Some(id),
                ..
            } if id == &pending_id
        )),
        FfiHostTransportEvent::PeerDisconnected { .. }
    ));

    assert_eq!(
        host.broadcast_stop("admission-stream".to_owned(), 1_250)
            .expect("stop admitted stream")
            .successful_peers,
        1
    );
    poll_listener_until(&approved, |event| {
        matches!(event, FfiListenerTransportEvent::Stopped { .. })
    });

    let late = FfiListenerTransportHandle::connect(
        endpoint,
        now_wall_clock_ms(),
        "legacy-late".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect after Stop");
    let late_id = join_approve_and_authorize(&host, &late, "Late listener");
    assert!(
        late.poll_event(150)
            .expect("stopped late join poll")
            .is_none(),
        "a new listener must not replay the retired stream after Stop"
    );
    assert_eq!(
        host.broadcast_stream_start(
            "admission-replay".to_owned(),
            2_000,
            48_000,
            2,
            240,
            Some("replay".to_owned()),
            Some("Replay".to_owned()),
        )
        .expect("start fresh stream")
        .successful_peers,
        2
    );
    for listener in [&approved, &late] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::StreamStarted { .. }
            )),
            FfiListenerTransportEvent::StreamStarted { stream_id, .. }
                if stream_id == "admission-replay"
        ));
    }
    let replay_pcm: Vec<u8> = (0..960_u32)
        .map(|index| u8::try_from(index % 239).expect("PCM byte"))
        .collect();
    assert_eq!(
        host.broadcast_audio(
            "admission-replay".to_owned(),
            0,
            48_000,
            2,
            240,
            0,
            2_000,
            replay_pcm.clone(),
        )
        .expect("send fresh PCM")
        .successful_peers,
        2
    );
    for listener in [&approved, &late] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::AudioReceived { .. }
            )),
            FfiListenerTransportEvent::AudioReceived {
                stream_id,
                sequence: 0,
                first_sample_index: 0,
                host_presentation_time_ms: 2_000,
                payload,
                ..
            } if stream_id == "admission-replay" && payload == replay_pcm
        ));
    }
    host.disconnect_peer(late_id, "Test complete".to_owned())
        .expect("disconnect late listener");
    late.shutdown().expect("close late listener");
    approved.shutdown().expect("close approved listener");
    host.shutdown().expect("close host");
}
