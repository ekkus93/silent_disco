//! Production live-queue edit before a prefetched successor commits:
//! decoded WAV -> bounded packetizer -> authenticated TCP/UDP -> listener
//! scheduler/render ring -> native callback. The removed song must never
//! leak a boundary or PCM; a Pause/Resume at the replacement boundary must
//! preserve the same native output and exact sample timeline.
//! Physical Oboe/DAC output is intentionally outside this software test.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{playback_generation, source};
use silent_disco_core::domain::{SessionId, StreamId};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one production-path edit, pause and exact native callback acceptance"
)]
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
    fs::write(&removed, source_wav_format(11_000, 48_000, 2, 480))
        .expect("removed successor");
    fs::write(&replacement, source_wav_format(7_000, 44_100, 1, 441))
        .expect("mixed-rate replacement");
    fs::write(&final_track, source_wav_format(9_000, 48_000, 2, 480))
        .expect("last source");

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
    assert!(pass.failure.is_none(), "retargeted producer must finish cleanly");
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
