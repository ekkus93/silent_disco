//! Real TCP/UDP natural queue end, explicit refill Play, and native callback
//! PCM continuity. Physical output/no-reopen measurement is deferred.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{expected_pcm, playback_generation, source};
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_control};
use silent_disco_core::domain::{MonotonicMillis, SampleIndex, SessionId, StreamId};
use silent_disco_core::protocol::{ControlMessage, Stop, TrackBoundary};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "production natural-end, explicit queue refill and socket-to-callback replay"
)]
fn natural_queue_end_requires_new_play_and_restarts_callback_pcm_without_stale_cache() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-socket-queue-refill-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    let third = root.join("third.wav");
    for (path, value) in [(&first, 3_000), (&second, 7_000), (&third, 9_000)] {
        fs::write(path, source_wav_format(value, 48_000, 2, 480)).expect("canonical WAV");
    }
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("queue-refill-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let initial = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![source("completed", &first)],
    )
    .expect("initial producer");
    let finished = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        None,
        &initial,
        &session,
        &stream,
        1_000,
        &[241, 239],
        false,
        false,
    );
    assert!(finished.failure.is_none());
    assert_eq!(finished.packets, 2);
    assert!(finished.boundaries.is_empty());
    assert_eq!(
        finished.pcm,
        expected_pcm(&[3_000], finished.startup_ramp_frames),
        "natural queue end must drain every final native callback sample"
    );
    initial.cancel().expect("release finished producer");

    // End of the queue retires playback at its audible tail, but leaves
    // the TCP session alive. A late joiner must not hear a completed track.
    let end = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_010),
    });
    assert_eq!(
        host.broadcast_control(&end)
            .expect("natural end Stop")
            .report
            .successful_peers,
        1
    );
    assert!(recv_control(listener.as_mut()) == end);
    let sent_after_end = host.counters().control_frames_sent;
    let mut late = authorized_late_joiner(host.as_mut(), "queue-refill-late");
    assert_eq!(host.counters().control_frames_sent, sent_after_end);
    let stale = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "obsolete-next".to_owned(),
        display_name: "obsolete-next".to_owned(),
    });
    host.broadcast_control(&stale)
        .expect("ignore retired boundary");
    assert_eq!(host.counters().control_frames_sent, sent_after_end);

    // Adding songs after natural end must not auto-start. Explicit Play
    // creates a new output token and resets sequence/sample indices, while
    // the original and late-joining real sockets receive identical PCM.
    let refill = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        2_000,
        vec![
            source("refill-second", &second),
            source("refill-third", &third),
        ],
    )
    .expect("explicit refill Play");
    let replayed = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        Some(late.as_mut()),
        &refill,
        &session,
        &stream,
        2_000,
        &[241, 337, 382],
        true, // Pause/Resume at the first refill boundary, without reopening output.
        false,
    );
    assert!(replayed.failure.is_none());
    assert_ne!(replayed.output_token, finished.output_token);
    assert_eq!(replayed.packets, 4);
    assert_eq!(replayed.boundaries, [480]);
    assert_eq!(
        replayed.pcm,
        expected_pcm(&[7_000, 9_000], replayed.startup_ramp_frames),
        "new Play must not insert old PCM, a boundary gap or another ramp"
    );
    refill.cancel().expect("release refill producer");

    // The refilled queue also ends naturally after a Pause/Resume-shifted
    // boundary. Neither existing socket may retain its old paused state or
    // cached successor when the host stays ready for another explicit Play.
    let second_end = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_220),
    });
    assert_eq!(
        host.broadcast_control(&second_end)
            .expect("second natural queue end")
            .report
            .successful_peers,
        2
    );
    assert!(recv_control(listener.as_mut()) == second_end);
    assert!(recv_control(late.as_mut()) == second_end);
    let after_second_end = host.counters().control_frames_sent;
    host.broadcast_control(&stale)
        .expect("ignore stale boundary after second queue end");
    assert_eq!(host.counters().control_frames_sent, after_second_end);

    // Refill a second time, reusing a previously completed staged source.
    // The same two real sockets must receive a fresh generation starting at
    // sample zero; the native callback must render both tracks exactly once.
    let third_play = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        3_000,
        vec![source("again-first", &first), source("again-third", &third)],
    )
    .expect("second explicit refill Play");
    let third_pass = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        Some(late.as_mut()),
        &third_play,
        &session,
        &stream,
        3_000,
        &[241, 337, 382],
        false,
        false,
    );
    assert!(third_pass.failure.is_none());
    assert_ne!(third_pass.output_token, replayed.output_token);
    assert_ne!(third_pass.output_token, finished.output_token);
    assert_eq!(third_pass.packets, 4);
    assert_eq!(third_pass.boundaries, [480]);
    assert_eq!(
        third_pass.pcm,
        expected_pcm(&[3_000, 9_000], third_pass.startup_ramp_frames),
        "second refill must restart at sample zero with one ramp and no stale PCM"
    );
    third_play.cancel().expect("release second refill producer");
    late.shutdown().expect("late listener shutdown");
    listener.shutdown().expect("original listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("remove fixture directory");
}
