//! Production real-socket Stop at a prefetched track boundary, followed by
//! same-stream-ID replay through the scheduler and native PCM callback.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{expected_pcm, playback_generation, source};
use crate::socket_pause_resume_pcm_tests::authorized_late_joiner;
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, ProtocolFrame, TrackBoundary,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "socket Stop, cached replay and callback PCM"
)]
fn stop_at_prefetched_boundary_fences_successor_and_replays_gaplessly() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-stop-prefetch-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    fs::write(&first, source_wav_format(3_000, 48_000, 2, 480)).expect("first WAV");
    fs::write(&second, source_wav_format(7_000, 48_000, 2, 480)).expect("second WAV");
    let sources = vec![source("first", &first), source("second", &second)];
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("stop-prefetch-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let initial = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        sources.clone(),
    )
    .expect("production playlist");
    let stopped = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        None,
        &initial,
        &session,
        &stream,
        1_000,
        &[241, 239],
        false,
        true,
    );
    assert!(stopped.failure.is_none());
    assert_eq!(stopped.packets, 2);
    assert!(
        stopped.boundaries.is_empty(),
        "unheard boundary must not broadcast"
    );
    assert_eq!(
        stopped.pcm,
        expected_pcm(&[3_000], stopped.startup_ramp_frames),
        "Stop delivers the entire first track, without successor samples"
    );

    let control_count = host.counters().control_frames_sent;
    let stale_boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "second".to_owned(),
        display_name: "second".to_owned(),
    });
    host.broadcast_control(&stale_boundary)
        .expect("stale boundary");
    assert_eq!(host.counters().control_frames_sent, control_count);
    let stale_audio = AudioDatagram {
        session_id: session.clone(),
        stream_id: stream.clone(),
        sequence: PacketSequence::new(2),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        first_sample_index: SampleIndex::new(480),
        host_presentation_time_ms: MonotonicMillis::new(1_010),
        payload: vec![0; 960],
    };
    assert_eq!(
        host.broadcast_audio(&ProtocolFrame::Audio(stale_audio))
            .expect("retired UDP")
            .report
            .intended_peers,
        0
    );
    let mut late = authorized_late_joiner(host.as_mut(), "stop-prefetch-late");
    assert_eq!(
        host.counters().control_frames_sent,
        control_count,
        "stopped replay cache cannot resurrect a prefetched successor"
    );

    let replay =
        FfiHostPlaylistStreamer::open(session.to_string(), stream.to_string(), 2_000, sources)
            .expect("explicit replay with same stream ID");
    let recovered = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        Some(late.as_mut()),
        &replay,
        &session,
        &stream,
        2_000,
        &[241, 337, 382],
        false,
        false,
    );
    assert_ne!(recovered.output_token, stopped.output_token);
    assert!(recovered.failure.is_none());
    assert_eq!(recovered.packets, 4, "fresh sequence");
    assert_eq!(recovered.boundaries, [480]);
    assert_eq!(
        recovered.pcm,
        expected_pcm(&[3_000, 7_000], recovered.startup_ramp_frames),
        "both sockets feed exact gapless native callback PCM"
    );
    replay.cancel().expect("release replay");
    late.shutdown().expect("late listener shutdown");
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
