//! Production real-socket Stop at a prefetched track boundary, followed by
//! same-stream-ID replay through the scheduler and native PCM callback.
//! Replay reorders the queue, repeats a staged source under a distinct entry ID,
//! and takes over the stopped listener's identity on a fresh TCP/UDP socket.
use crate::audio_abi::registry_test_guard;
use crate::host_playlist::FfiHostPlaylistStreamer;
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{expected_pcm, playback_generation, source};
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_control};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, ProtocolFrame, StreamStart, TrackBoundary,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "socket Stop, cached replay and callback PCM"
)]
fn stop_at_prefetched_boundary_then_same_identity_reconnect_reorders_replay_pcm() {
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

    // After Stop, a second TCP/UDP socket takes over the original device ID
    // while the retired reader is still alive. The new route must not inherit
    // a prefetched boundary, but must receive every frame of the new Play.
    let mut replacement = authorized_late_joiner(host.as_mut(), "pcm-regression-listener");
    assert_eq!(
        host.counters().control_frames_sent,
        control_count,
        "same-device reconnect after Stop cannot resurrect cached controls"
    );
    listener
        .shutdown()
        .expect("retire old same-identity TCP socket");

    // The host is allowed to edit its stopped queue: reorder both entries and
    // append the first source again with a new entry ID. A repeated path must
    // be decoded as a separate track without leaking the retired prefetch.
    let mut replay_sources = vec![sources[1].clone(), sources[0].clone()];
    let mut repeated = sources[0].clone();
    repeated.entry_id = "first-again".to_owned();
    replay_sources.push(repeated);
    assert_eq!(replay_sources[1].staged_path, replay_sources[2].staged_path);
    assert_ne!(replay_sources[1].entry_id, replay_sources[2].entry_id);
    let replay = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        100_000,
        replay_sources,
    )
    .expect("explicit reordered replay with same stream ID");
    let recovered = playback_generation(
        host.as_mut(),
        replacement.as_mut(),
        Some(late.as_mut()),
        &replay,
        &session,
        &stream,
        100_000,
        &[241, 337, 382, 480],
        false,
        false,
    );
    assert_ne!(recovered.output_token, stopped.output_token);
    assert!(recovered.failure.is_none());
    assert_eq!(recovered.packets, 6, "fresh contiguous packet sequence");
    assert_eq!(recovered.boundaries, [480, 960]);
    assert_eq!(
        recovered.pcm,
        expected_pcm(&[7_000, 3_000, 3_000], recovered.startup_ramp_frames),
        "both sockets feed exact reordered/repeated gapless native callback PCM"
    );
    // A delayed control packet from the retired generation must not replace
    // the fresh stream's cached start/boundaries, even when both Plays reuse
    // the same stream ID. A third real socket checks the authoritative replay
    // snapshot after the stale control was rejected.
    let sent_before_stale_start = host.counters().control_frames_sent;
    let retired_start = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        current_entry_id: Some("first".to_owned()),
        current_display_name: Some("first".to_owned()),
    });
    host.broadcast_control(&retired_start)
        .expect("ignore retired-generation StreamStart");
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before_stale_start,
        "retired StreamStart must not reach any active listener"
    );
    let mut post_replay_joiner = authorized_late_joiner(host.as_mut(), "post-replay-joiner");
    let ControlMessage::StreamStart(replayed_start) = recv_control(post_replay_joiner.as_mut())
    else {
        panic!("late join must receive the fresh generation's StreamStart");
    };
    assert_eq!(
        replayed_start.host_start_time_ms,
        MonotonicMillis::new(100_000)
    );
    assert_eq!(replayed_start.current_entry_id.as_deref(), Some("second"));
    for (index, entry_id) in [(480, "first"), (960, "first-again")] {
        let ControlMessage::TrackBoundary(boundary) = recv_control(post_replay_joiner.as_mut())
        else {
            panic!("late join must receive both fresh-generation boundaries");
        };
        assert_eq!(boundary.first_sample_index, SampleIndex::new(index));
        assert_eq!(boundary.entry_id, entry_id);
    }
    post_replay_joiner
        .shutdown()
        .expect("late replay joiner shutdown");
    replay.cancel().expect("release replay");
    late.shutdown().expect("late listener shutdown");
    replacement
        .shutdown()
        .expect("replacement listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
