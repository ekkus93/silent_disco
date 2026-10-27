//! Production live-queue edit before a prefetched successor commits:
//! decoded WAV -> bounded packetizer -> authenticated TCP/UDP -> listener
//! scheduler/render ring -> native callback. The removed song must never
//! leak a boundary or PCM; a Pause/Resume at the replacement boundary must
//! preserve the same native output and exact sample timeline.
//! Physical Oboe/DAC output is intentionally outside this software test.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{expected_pcm, playback_generation, source};
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_control};
use silent_disco_core::domain::{MonotonicMillis, SampleIndex};
use silent_disco_core::domain::{SessionId, StreamId};
use silent_disco_core::protocol::{ControlMessage, Stop, StreamStart, TrackBoundary};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn removing_prefetched_successor_before_commit_renders_only_retargeted_live_queue() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-live-edit-pcm-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let current = root.join("long-current.wav");
    let removed = root.join("removed.wav");
    let replacement = root.join("replacement.wav");
    let final_track = root.join("final.wav");
    // 40 packets exceed the bounded producer's 32-frame output queue.
    // Thus its successor boundary cannot already be enqueued when the
    // authoritative upcoming-list update is submitted below.
    fs::write(&current, source_wav_format(3_000, 48_000, 2, 40 * 240))
        .expect("long current source");
    fs::write(&removed, source_wav_format(11_000, 48_000, 2, 480)).expect("removed successor");
    fs::write(&replacement, source_wav_format(7_000, 44_100, 1, 441))
        .expect("mixed-rate replacement");
    fs::write(&final_track, source_wav_format(9_000, 48_000, 2, 480)).expect("last source");

    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("live-edit-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let playlist = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![
            source("current", &current),
            source("removed", &removed),
            source("replacement", &replacement),
            source("final", &final_track),
        ],
    )
    .expect("production playlist");
    // The host accepted an edit while the current source was still playing.
    // The producer must discard its speculative successor and emit only
    // replacement/final, including their exact contiguous boundaries.
    playlist
        .replace_upcoming(vec![
            source("replacement", &replacement),
            source("final", &final_track),
        ])
        .expect("retarget before the bounded output queue reaches the boundary");

    // 9,600 + 480 + 480 frames: 44 exact UDP packets, two boundaries,
    // one 200 ms pause at the first replacement boundary and one native
    // callback output for the entire generation. Non-packet-aligned reads
    // include a callback crossing the first edited track boundary.
    let chunks = [241, 337, 382, 4_800, 4_800];
    assert_eq!(chunks.iter().sum::<u32>(), 10_560);
    let pass = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        None,
        &playlist,
        &session,
        &stream,
        1_000,
        &chunks,
        true,
        false,
    );
    assert!(
        pass.failure.is_none(),
        "retargeted producer must finish cleanly"
    );
    assert_eq!(pass.packets, 44, "no old or duplicated packet");
    assert_eq!(
        pass.boundaries,
        [9_600, 10_080],
        "removed successor cannot commit; replacement boundaries are exact"
    );
    let expected: Vec<f32> = (0..10_560_usize)
        .flat_map(|frame| {
            let level = if frame < 9_600 {
                3_000_i32
            } else if frame < 10_080 {
                7_000_i32
            } else {
                9_000_i32
            };
            let sample = if frame < pass.startup_ramp_frames {
                level * i32::try_from(frame).expect("frame")
                    / i32::try_from(pass.startup_ramp_frames).expect("ramp")
            } else {
                level
            };
            std::iter::repeat_n(
                f32::from(i16::try_from(sample).expect("sample")) / 32768.0,
                2,
            )
        })
        .collect();
    assert_eq!(
        pass.pcm, expected,
        "real-socket callback PCM must contain no removed source, gap, overlap, or second ramp"
    );
    playlist.cancel().expect("release producer");
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one real-socket edited generation, two native outputs, Stop, reconnect and replay"
)]
fn edited_live_queue_stops_and_replays_reordered_sources_to_two_native_outputs() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-edit-stop-replay-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let current = root.join("current.wav");
    let removed = root.join("removed.wav");
    let replacement = root.join("replacement.wav");
    let last = root.join("last.wav");
    fs::write(&current, source_wav_format(3_000, 48_000, 2, 40 * 240)).expect("current source");
    fs::write(&removed, source_wav_format(11_000, 48_000, 2, 480)).expect("removed source");
    fs::write(&replacement, source_wav_format(7_000, 44_100, 1, 441)).expect("replacement source");
    fs::write(&last, source_wav_format(9_000, 48_000, 2, 480)).expect("final source");

    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("edit-stop-replay").expect("stream");
    let (mut host, mut primary) = authorized_pcm_socket_pair();
    let mut secondary = authorized_late_joiner(host.as_mut(), "edited-second-listener");
    let edited = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![
            source("current", &current),
            source("removed", &removed),
            source("replacement", &replacement),
            source("last", &last),
        ],
    )
    .expect("production edit playlist");
    let metadata = edited.stream_start();
    edited
        .replace_upcoming(vec![
            source("replacement", &replacement),
            source("last", &last),
        ])
        .expect("remove prefetched successor before boundary reservation");
    let first = playback_generation(
        host.as_mut(),
        primary.as_mut(),
        Some(secondary.as_mut()),
        &edited,
        &session,
        &stream,
        1_000,
        &[241, 337, 382, 4_800, 4_800],
        true,
        false,
    );
    assert!(first.failure.is_none());
    assert_eq!(first.packets, 44);
    assert_eq!(first.boundaries, [9_600, 10_080]);
    assert_eq!(first.pcm.len(), 10_560 * 2);
    for (begin, end, value) in [(9_600, 10_080, 7_000_i16), (10_080, 10_560, 9_000)] {
        let expected = f32::from(value) / 32768.0;
        assert!(
            first.pcm[begin * 2..end * 2]
                .iter()
                .all(|sample| sample.to_bits() == expected.to_bits()),
            "edited successors must render exact source PCM without a second ramp"
        );
    }
    edited.cancel().expect("release first generation");

    // Stop retires the edited boundary and the pause/resume cache before a
    // second listener reconnects using its original device identity.
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_500),
    });
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("Stop after edited queue")
            .report
            .successful_peers,
        2
    );
    assert!(recv_control(primary.as_mut()) == stop);
    assert!(recv_control(secondary.as_mut()) == stop);
    let stopped_controls = host.counters().control_frames_sent;
    let mut replacement_socket = authorized_late_joiner(host.as_mut(), "edited-second-listener");
    assert_eq!(
        host.counters().control_frames_sent,
        stopped_controls,
        "reconnected peer must receive no completed edited queue or Pause"
    );
    secondary.shutdown().expect("retire original second socket");

    // Delayed old-generation controls must not resurrect a removed source.
    let old_start = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: metadata.sample_rate,
        channels: metadata.channels,
        samples_per_packet: metadata.samples_per_packet,
        current_entry_id: metadata.current_entry_id,
        current_display_name: metadata.current_display_name,
    });
    let old_boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(9_600),
        entry_id: "removed".to_owned(),
        display_name: "removed".to_owned(),
    });
    host.broadcast_control(&old_start)
        .expect("ignore retired stream start");
    host.broadcast_control(&old_boundary)
        .expect("ignore removed successor");
    assert_eq!(host.counters().control_frames_sent, stopped_controls);

    // Replay the two accepted successors in reversed order on the SAME
    // stream ID, with fresh packet/sample indices and new native outputs.
    let replay = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        3_000,
        vec![
            source("replay-last", &last),
            source("replay-replacement", &replacement),
        ],
    )
    .expect("replay reversed accepted sources");
    let second = playback_generation(
        host.as_mut(),
        primary.as_mut(),
        Some(replacement_socket.as_mut()),
        &replay,
        &session,
        &stream,
        3_000,
        &[241, 337, 382],
        false,
        false,
    );
    assert!(second.failure.is_none());
    assert_eq!(second.packets, 4, "replay sequence resets to packet zero");
    assert_eq!(
        second.boundaries,
        [480],
        "new generation starts at sample zero"
    );
    assert_ne!(second.output_token, first.output_token);
    assert_eq!(
        second.pcm,
        expected_pcm(&[9_000, 7_000], second.startup_ramp_frames),
        "both independent listener outputs must replay exact reversed PCM"
    );
    replay.cancel().expect("release replay producer");
    replacement_socket
        .shutdown()
        .expect("replacement socket shutdown");
    primary.shutdown().expect("primary shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
