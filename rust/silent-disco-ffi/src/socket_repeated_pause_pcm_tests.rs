//! Real TCP/UDP -> scheduler -> two independent native callback outputs.
//! Repeated Pause/Resume at each queued-track boundary must preserve the
//! sample timeline without reopening either output. No physical DAC proof.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{
    expected_pcm, playback_generation_with_pause_repeats, source,
};
use crate::socket_pause_resume_pcm_tests::authorized_late_joiner;
use silent_disco_core::domain::{SessionId, StreamId};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn repeated_boundary_pause_resume_preserves_exact_pcm_on_two_real_socket_outputs() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-socket-repeated-pause-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let mut sources = Vec::new();
    for (index, value) in [3_000, 6_000, 9_000, 12_000].into_iter().enumerate() {
        let path = root.join(format!("track-{index}.wav"));
        fs::write(&path, source_wav_format(value, 48_000, 2, 480))
            .expect("canonical WAV source");
        sources.push(source(&format!("track-{index}"), &path));
    }
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("repeated-pause-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let mut second = authorized_late_joiner(host.as_mut(), "repeated-pause-second");
    let playlist = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        sources,
    )
    .expect("four-track production producer");

    // Three distinct TCP Pause/Resume pairs, one at every successor boundary.
    // Both authorized UDP routes feed separate scheduler/native callback
    // outputs; neither output is replaced across any of the six controls.
    let playback = playback_generation_with_pause_repeats(
        host.as_mut(),
        listener.as_mut(),
        Some(second.as_mut()),
        &playlist,
        &session,
        &stream,
        1_000,
        &[241, 337, 382, 480, 480],
        true,
        false,
        true,
    );
    assert!(playback.failure.is_none(), "four tracks must finish naturally");
    assert_eq!(playback.packets, 8, "all PCM packet indices remain contiguous");
    assert_eq!(playback.boundaries, [480, 960, 1_440]);
    assert_eq!(
        playback.pcm,
        expected_pcm(&[3_000, 6_000, 9_000, 12_000], playback.startup_ramp_frames),
        "repeated Resume must not insert silence, duplicate samples or restart the ramp"
    );

    playlist.cancel().expect("cancel producer");
    second.shutdown().expect("second socket shutdown");
    listener.shutdown().expect("first socket shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("remove fixtures");
}
