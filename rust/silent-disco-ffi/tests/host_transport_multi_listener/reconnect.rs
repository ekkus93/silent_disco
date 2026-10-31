use super::*;

/// A new boundary after same-identity replacement must be authoritative for
/// both existing sockets and a *different* listener joining mid-track.
fn assert_post_reconnect_late_join_pcm(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    replacement: &FfiListenerTransportHandle,
    late: &FfiListenerTransportHandle,
) {
    let pcm: Vec<u8> = (0..960_u32)
        .map(|index| u8::try_from((index * 29) % 256).expect("byte"))
        .collect();
    assert_eq!(
        host.broadcast_audio(
            "ffi-fresh-stream".to_owned(),
            3,
            48_000,
            2,
            240,
            720,
            2_115,
            pcm.clone(),
        )
        .expect("send first PCM of third track")
        .successful_peers,
        3
    );
    for listener in [listener_a, replacement, late] {
        assert_eq!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::AudioReceived { .. }
            )),
            FfiListenerTransportEvent::AudioReceived {
                stream_id: "ffi-fresh-stream".to_owned(),
                sequence: 3,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 240,
                first_sample_index: 720,
                host_presentation_time_ms: 2_115,
                payload: pcm.clone(),
            }
        );
    }
}

/// The reconnect cache must retain a newly announced sample-index boundary,
/// not the pre-reconnect playlist or a stale Pause. A new TCP peer receives
/// that snapshot before exact next-track UDP reaches all three live peers.
pub(super) fn assert_post_reconnect_boundary_and_late_join(
    host: &FfiHostTransportHandle,
    listener_a: &FfiListenerTransportHandle,
    replacement: &FfiListenerTransportHandle,
    endpoint: &str,
) {
    assert_eq!(
        host.broadcast_track_boundary(
            "ffi-fresh-stream".to_owned(),
            720,
            "third".to_owned(),
            "Third track".to_owned(),
        )
        .expect("announce third track after reconnect")
        .successful_peers,
        2
    );
    for listener in [listener_a, replacement] {
        assert!(matches!(
            poll_listener_until(listener, |event| matches!(
                event,
                FfiListenerTransportEvent::TrackBoundary { .. }
            )),
            FfiListenerTransportEvent::TrackBoundary {
                first_sample_index: 720,
                entry_id,
                ..
            } if entry_id == "third"
        ));
    }

    let late = FfiListenerTransportHandle::connect(
        endpoint.to_owned(),
        now_wall_clock_ms(),
        "ffi-peer-late-after-reconnect".to_owned(),
        HOST_ADDRESS.to_owned(),
    )
    .expect("connect a third listener after reconnect boundary");
    let late_id = join_approve_and_authorize(host, &late, "Post-reconnect Late Listener");
    host.replay_cached_playback_to_listener(late_id.clone())
        .expect("replay latest track boundary to late listener");
    assert!(matches!(
        poll_listener_until(&late, |event| matches!(
            event,
            FfiListenerTransportEvent::StreamStarted { .. }
        )),
        FfiListenerTransportEvent::StreamStarted { stream_id, .. }
            if stream_id == "ffi-fresh-stream"
    ));
    assert!(matches!(
        poll_listener_until(&late, |event| matches!(
            event,
            FfiListenerTransportEvent::TrackBoundary { .. }
        )),
        FfiListenerTransportEvent::TrackBoundary {
            first_sample_index: 720,
            entry_id,
            ..
        } if entry_id == "third"
    ));
    assert_post_reconnect_late_join_pcm(host, listener_a, replacement, &late);
    assert_eq!(
        host.disconnect_peer(late_id, "third listener leaving".to_owned())
            .expect("remove post-reconnect late listener")
            .successful_peers,
        1
    );
    poll_listener_until(&late, |event| {
        matches!(event, FfiListenerTransportEvent::HostDisconnected { .. })
    });
    late.shutdown().expect("close post-reconnect late listener");
}
