//! Production real TCP/UDP -> scheduler -> independent native callback outputs.
//! Multiple queued-track Pause/Resume cycles, terminal Stop, late admission,
//! and explicit same-stream-ID replay must not leak retired PCM or reopen
//! either callback output within its generation. Physical DAC proof is deferred.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{
    expected_pcm, playback_generation, playback_generation_with_pause_repeats, source,
};
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_control};
use silent_disco_core::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use silent_disco_core::protocol::{ControlMessage, Stop, TrackBoundary};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one integrated multi-pause, Stop, late-join and replay callback acceptance"
)]
fn repeated_boundary_pauses_then_stop_and_late_join_replay_exact_pcm() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-multi-pause-stop-replay-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let mut tracks = Vec::new();
    for (index, value) in [3_000, 6_000, 9_000].into_iter().enumerate() {
        let path = root.join(format!("track-{index}.wav"));
        fs::write(&path, source_wav_format(value, 48_000, 2, 480))
            .expect("canonical WAV source");
        tracks.push(source(&format!("track-{index}"), &path));
    }
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("multi-pause-stop-replay-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();

    let initial = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        tracks.clone(),
    )
    .expect("three-track production producer");
    let first = playback_generation_with_pause_repeats(
        host.as_mut(),
        listener.as_mut(),
        None,
        &initial,
        &session,
        &stream,
        1_000,
        &[241, 337, 382, 480],
        true,
        false,
        true,
    );
    assert!(first.failure.is_none());
    assert_eq!(first.packets, 6);
    assert_eq!(first.boundaries, [480, 960]);
    assert_eq!(
        first.pcm,
        expected_pcm(&[3_000, 6_000, 9_000], first.startup_ramp_frames),
        "two Pause/Resume cycles must preserve every callback sample"
    );

    // Two 200-ms Resume reanchors shift the last sample tail to 1430 ms.
    // Retire the completed generation before a new listener is admitted.
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_430),
    });
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("terminal Stop")
            .report
            .successful_peers,
        1
    );
    assert!(recv_control(listener.as_mut()) == stop);
    let sent_after_stop = host.counters().control_frames_sent;
    let mut late = authorized_late_joiner(host.as_mut(), "multi-pause-replay-late");
    assert_eq!(
        host.counters().control_frames_sent,
        sent_after_stop,
        "late join must not replay stopped controls or cached boundaries"
    );
    let retired_boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(960),
        entry_id: "track-2".to_owned(),
        display_name: "track-2".to_owned(),
    });
    host.broadcast_control(&retired_boundary)
        .expect("stale post-Stop boundary is ignored");
    assert_eq!(
        host.counters().control_frames_sent,
        sent_after_stop,
        "retired boundary must not reach either socket"
    );
    initial.cancel().expect("retire initial producer");

    // A new Play on the same stream ID reorders the queue. Both the original
    // and newly admitted socket must receive exact replay PCM on independent
    // native callback outputs, including a fresh Pause/Resume boundary.
    let replay = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        3_000,
        vec![tracks[2].clone(), tracks[0].clone()],
    )
    .expect("reordered replay producer");
    let second = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        Some(late.as_mut()),
        &replay,
        &session,
        &stream,
        3_000,
        &[241, 337, 382],
        true,
        false,
    );
    assert!(second.failure.is_none());
    assert_ne!(first.output_token, second.output_token);
    assert_eq!(second.packets, 4, "replay sequence must restart at zero");
    assert_eq!(second.boundaries, [480]);
    assert_eq!(
        second.pcm,
        expected_pcm(&[9_000, 3_000], second.startup_ramp_frames),
        "replay cannot contain retired PCM, duplicated samples or gaps"
    );
    replay.cancel().expect("retire replay producer");
    late.shutdown().expect("late socket shutdown");
    listener.shutdown().expect("original socket shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
