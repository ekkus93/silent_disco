//! End-to-end software regression for an unavailable prefetched source:
//! canonical WAV -> FFI producer -> real TCP/UDP -> listener scheduler ->
//! native callback. Stop must fence the failed generation; a valid retry
//! must deliver gapless PCM without accepting stale source metadata.
//! Physical audio output is a separate, deferred qualification.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, register_render_ring, registry_test_guard,
    release_render_ring, silent_disco_audio_callback_count,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_silence_filled_frames,
    silent_disco_audio_underrun_count,
};
use crate::callback_capture::CallbackCaptureTap;
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_audio, recv_control};
use crate::socket_secondary_pcm_output_tests::SecondaryPcmOutput;
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig,
};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, Pause, ProtocolFrame, Stop, StreamStart,
    TrackBoundary,
};
use silent_disco_core::transport::{HostTransportNode, ListenerTransportNode};
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const TRACK_FRAMES: u32 = 480;

pub(super) fn source(id: &str, path: &std::path::Path) -> FfiPlaylistSource {
    FfiPlaylistSource {
        entry_id: id.to_owned(),
        display_name: id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    }
}

pub(super) struct CallbackPass {
    pub(super) pcm: Vec<f32>,
    pub(super) boundaries: Vec<u64>,
    pub(super) packets: u64,
    pub(super) failure: Option<String>,
    pub(super) output_token: u64,
    pub(super) startup_ramp_frames: usize,
}

/// One production playback generation, with optional second authorized socket.
/// The native output token stays unchanged across every packet and boundary.
#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one real-socket / production scheduler / callback playback generation"
)]
pub(super) fn playback_generation(
    host: &mut dyn HostTransportNode,
    listener: &mut dyn ListenerTransportNode,
    second: Option<&mut dyn ListenerTransportNode>,
    playlist: &FfiHostPlaylistStreamer,
    session: &SessionId,
    stream: &StreamId,
    start_ms: u64,
    chunks: &[u32],
    pause_at_boundary: bool,
    stop_at_boundary: bool,
) -> CallbackPass {
    let metadata = playlist.stream_start();
    let start = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(start_ms),
        sample_rate: metadata.sample_rate,
        channels: metadata.channels,
        samples_per_packet: metadata.samples_per_packet,
        current_entry_id: metadata.current_entry_id,
        current_display_name: metadata.current_display_name,
    });
    let peer_count = if second.is_some() { 2 } else { 1 };
    assert_eq!(
        host.broadcast_control(&start)
            .expect("send start")
            .report
            .successful_peers,
        peer_count
    );
    assert!(recv_control(listener) == start);
    let mut second = second;
    let mut second_output = second
        .is_some()
        .then(|| SecondaryPcmOutput::new(session, stream, start_ms, metadata.samples_per_packet));
    if let Some(peer) = second.as_deref_mut() {
        assert!(recv_control(peer) == start);
    }

    let mut config = SchedulerConfig::new(
        session.clone(),
        stream.clone(),
        48_000,
        start_ms,
        metadata.samples_per_packet,
        2,
    );
    config.startup_buffer_target_ms = 0;
    let startup_ramp_frames = usize::try_from(
        u64::from(config.sample_rate) * u64::from(config.concealment_ramp_ms) / 1_000,
    )
    .expect("ramp");
    let (producer, output_token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 4_800,
    })
    .expect("native output");
    if let Some(output) = &second_output {
        assert_ne!(
            output.token(),
            output_token,
            "listeners must own distinct native outputs"
        );
    }
    let mut pump = PlaybackPump::new(
        PlaybackScheduler::new(config, 0.0).expect("scheduler"),
        producer,
        PlaybackPumpConfig {
            volume: 1.0,
            write_lead_ms: 0,
            max_prefill_ms: 0,
            target_depth_frames: 4_800,
        },
    )
    .expect("listener pump");
    pump.apply_sync_offset(0.0);
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("capture"));
    attach_callback_capture(output_token, Arc::clone(&tap)).expect("attach tap");

    let mut packets = 0_u64;
    let mut sample = 0_u64;
    let mut boundaries = Vec::new();
    let mut failure = None;
    let mut completed = false;
    let mut paused_boundary = false;
    let mut stopped = false;
    let mut presentation_shift_ms = 0_u64;
    for _ in 0..64 {
        match playlist.next_frame(5_000) {
            Ok(Some(FfiPlaylistFrame::Audio {
                sequence,
                sample_rate,
                channels,
                samples_per_packet,
                first_sample_index,
                host_presentation_time_ms,
                payload,
                ..
            })) => {
                assert_eq!(sequence, packets, "no packet skip/duplication");
                assert_eq!(first_sample_index, sample, "no PCM gap or overlap");
                let packet = AudioDatagram {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    sequence: PacketSequence::new(sequence),
                    codec: AudioCodec::PcmS16Le,
                    sample_rate,
                    channels,
                    samples_per_packet,
                    first_sample_index: SampleIndex::new(first_sample_index),
                    host_presentation_time_ms: MonotonicMillis::new(
                        host_presentation_time_ms + presentation_shift_ms,
                    ),
                    payload,
                };
                assert_eq!(
                    host.broadcast_audio(&ProtocolFrame::Audio(packet.clone()))
                        .expect("UDP")
                        .report
                        .successful_peers,
                    peer_count
                );
                let received = recv_audio(listener);
                assert_eq!(received, packet, "socket PCM metadata/payload exact");
                if let Some(peer) = second.as_deref_mut() {
                    let second_packet = recv_audio(peer);
                    assert_eq!(second_packet, packet, "second listener PCM exact");
                    second_output
                        .as_mut()
                        .expect("second socket owns a native playback runtime")
                        .submit(
                            second_packet,
                            host_presentation_time_ms + presentation_shift_ms,
                        );
                }
                pump.scheduler_mut()
                    .submit_packet(received)
                    .expect("schedule PCM");
                for _ in 0..64 {
                    if !matches!(
                        pump.tick(host_presentation_time_ms + presentation_shift_ms),
                        PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
                    ) {
                        break;
                    }
                }
                packets += 1;
                sample += u64::from(samples_per_packet);
            }
            Ok(Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index,
                entry_id,
                display_name,
                ..
            })) => {
                assert_eq!(first_sample_index, sample);
                if stop_at_boundary {
                    // The successor was prefetched but never became audible.
                    let stop = ControlMessage::Stop(Stop {
                        session_id: session.clone(),
                        stream_id: stream.clone(),
                        host_stop_time_ms: MonotonicMillis::new(start_ms + sample * 1_000 / 48_000),
                    });
                    assert_eq!(
                        host.broadcast_control(&stop)
                            .expect("Stop before boundary")
                            .report
                            .successful_peers,
                        peer_count
                    );
                    assert!(recv_control(listener) == stop);
                    if let Some(peer) = second.as_deref_mut() {
                        assert!(recv_control(peer) == stop);
                    }
                    playlist.cancel().expect("cancel prefetched successor");
                    stopped = true;
                    break;
                }
                let control = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id,
                    display_name,
                });
                assert_eq!(
                    host.broadcast_control(&control)
                        .expect("TCP boundary")
                        .report
                        .successful_peers,
                    peer_count
                );
                assert!(recv_control(listener) == control);
                if let Some(peer) = second.as_deref_mut() {
                    assert!(recv_control(peer) == control);
                }
                boundaries.push(first_sample_index);
                if pause_at_boundary && !paused_boundary {
                    // Pause only at the first boundary. Subsequent boundaries
                    // must remain contiguous after Resume in a longer queue.
                    // A producer may have decoded successor PCM, but the paused
                    // host must not leak it over UDP or advance the output token.
                    let pause = ControlMessage::Pause(Pause {
                        session_id: session.clone(),
                        stream_id: stream.clone(),
                        host_pause_time_ms: MonotonicMillis::new(start_ms + 10),
                    });
                    assert_eq!(
                        host.broadcast_control(&pause)
                            .expect("Pause TCP")
                            .report
                            .successful_peers,
                        peer_count
                    );
                    assert!(recv_control(listener) == pause);
                    if let Some(peer) = second.as_deref_mut() {
                        assert!(recv_control(peer) == pause);
                    }
                    let suppressed = AudioDatagram {
                        session_id: session.clone(),
                        stream_id: stream.clone(),
                        sequence: PacketSequence::new(packets),
                        codec: AudioCodec::PcmS16Le,
                        sample_rate: 48_000,
                        channels: 2,
                        samples_per_packet: 240,
                        first_sample_index: SampleIndex::new(sample),
                        host_presentation_time_ms: MonotonicMillis::new(start_ms + 10),
                        payload: vec![0; 240 * 2 * 2],
                    };
                    assert_eq!(
                        host.broadcast_audio(&ProtocolFrame::Audio(suppressed))
                            .expect("paused UDP is suppressed")
                            .report
                            .intended_peers,
                        0
                    );
                    let count = host.counters().control_frames_sent;
                    host.broadcast_control(&start)
                        .expect("ignore stale pre-pause start");
                    assert_eq!(host.counters().control_frames_sent, count);
                    let ControlMessage::StreamStart(mut reanchored) = start.clone() else {
                        panic!("initial stream start");
                    };
                    reanchored.host_start_time_ms = MonotonicMillis::new(start_ms + 200);
                    let resume = ControlMessage::StreamStart(reanchored);
                    assert_eq!(
                        host.broadcast_control(&resume)
                            .expect("Resume TCP")
                            .report
                            .successful_peers,
                        peer_count
                    );
                    assert!(recv_control(listener) == resume);
                    if let Some(peer) = second.as_deref_mut() {
                        assert!(recv_control(peer) == resume);
                    }
                    pump.scheduler_mut().set_host_start_time_ms(start_ms + 200);
                    if let Some(output) = second_output.as_mut() {
                        output.reanchor(start_ms + 200);
                    }
                    presentation_shift_ms = 200;
                    paused_boundary = true;
                }
            }
            Ok(Some(FfiPlaylistFrame::End)) => {
                completed = true;
                break;
            }
            Ok(None) => {}
            Err(error) => {
                failure = Some(error.to_string());
                break;
            }
        }
    }
    assert!(
        completed || failure.is_some() || stopped,
        "producer did not terminate"
    );
    assert_eq!(
        paused_boundary, pause_at_boundary,
        "expected pause/boundary coverage"
    );
    assert_eq!(
        chunks.iter().copied().map(u64::from).sum::<u64>(),
        sample,
        "callback must drain all valid PCM, without a masked tail"
    );
    let native = usize::try_from(output_token).expect("token") as *mut core::ffi::c_void;
    let mut pcm = Vec::new();
    for &frames in chunks {
        let mut returned = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
        let mut supplied = 0_u32;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                native,
                returned.as_mut_ptr(),
                frames,
                2,
                &raw mut supplied,
            ),
            0,
            "native callback must not underrun or synthesize silence"
        );
        assert_eq!(supplied, frames);
        assert_eq!(
            tap.drain(usize::try_from(frames).expect("frames")),
            returned,
            "callback-returned PCM and nonblocking tap must match"
        );
        pcm.extend(returned);
    }
    assert_eq!(pump.diagnostics().packets_emitted, packets);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    assert_eq!(
        silent_disco_audio_callback_count(native.cast_const()),
        u64::try_from(chunks.len()).expect("callbacks")
    );
    assert_eq!(silent_disco_audio_underrun_count(native.cast_const()), 0);
    assert_eq!(
        silent_disco_audio_silence_filled_frames(native.cast_const()),
        0
    );
    assert_eq!(tap.counters(), (sample, 0, chunks.len() as u64));
    if let Some(output) = second_output {
        output.assert_and_release(chunks, &pcm, packets);
    }
    detach_callback_capture(output_token);
    release_render_ring(output_token).expect("release output");
    CallbackPass {
        pcm,
        boundaries,
        packets,
        failure,
        output_token,
        startup_ramp_frames,
    }
}

pub(super) fn expected_pcm(samples: &[i16], ramp: usize) -> Vec<f32> {
    samples
        .iter()
        .enumerate()
        .flat_map(|(track, &value)| {
            (0..TRACK_FRAMES as usize).flat_map(move |frame| {
                let adjusted = if track == 0 && frame < ramp {
                    i32::from(value) * i32::try_from(frame).expect("frame")
                        / i32::try_from(ramp).expect("ramp")
                } else {
                    i32::from(value)
                };
                std::iter::repeat_n(
                    f32::from(i16::try_from(adjusted).expect("fixture sample")) / 32768.0,
                    2,
                )
            })
        })
        .collect()
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one integrated failure/Stop/retry network and callback lifecycle"
)]
fn missing_prefetched_source_stops_after_valid_callback_pcm_then_replays_gaplessly() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-socket-failure-recovery-{unique}"));
    fs::create_dir_all(&root).expect("fixture dir");
    let first = root.join("first.wav");
    let fixed = root.join("fixed.wav");
    let unavailable = root.join("unavailable.wav");
    fs::write(&first, source_wav_format(3_000, 48_000, 2, 480)).expect("first canonical fixture");
    fs::write(&fixed, source_wav_format(7_000, 48_000, 2, 480))
        .expect("recovered canonical fixture");

    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("failure-recovery-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let failing = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![source("good", &first), source("unavailable", &unavailable)],
    )
    .expect("open lazy successor");
    let before = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        None,
        &failing,
        &session,
        &stream,
        1_000,
        &[241, 239],
        false,
        false,
    );
    assert_eq!(before.packets, 2, "deliver entire valid track");

    assert!(
        before.boundaries.is_empty(),
        "never announce failed successor"
    );
    assert!(
        before
            .failure
            .as_deref()
            .is_some_and(|e| e.contains("unavailable")),
        "source-specific failure must be surfaced"
    );
    assert_eq!(
        before.pcm,
        expected_pcm(&[3_000], before.startup_ramp_frames)
    );
    // Stop at the final valid packet's presentation tail (480 / 48k = 10 ms).
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_010),
    });
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("Stop on failure")
            .report
            .successful_peers,
        1
    );
    assert!(recv_control(listener.as_mut()) == stop);
    let after_stop_count = host.counters().control_frames_sent;
    let mut late = authorized_late_joiner(host.as_mut(), "recovery-late");
    assert_eq!(
        host.counters().control_frames_sent,
        after_stop_count,
        "late listener may not replay failed/retired track or Stop"
    );
    failing.cancel().expect("release failed producer");

    // Retry retains the valid first entry and appends the fixed successor.
    // Fresh Play reuses the stream ID intentionally, resetting sequence/sample
    // position and opening exactly one NEW output generation after Stop.
    let retry = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        2_000,
        vec![source("good", &first), source("fixed", &fixed)],
    )
    .expect("retry producer");
    let recovered = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        Some(late.as_mut()),
        &retry,
        &session,
        &stream,
        2_000,
        &[241, 337, 382],
        false,
        false,
    );
    assert_ne!(recovered.output_token, before.output_token);
    assert!(recovered.failure.is_none(), "retry must finish cleanly");
    assert_eq!(recovered.packets, 4);
    assert_eq!(recovered.boundaries, [480], "one exact recovery transition");
    assert_eq!(
        recovered.pcm,
        expected_pcm(&[3_000, 7_000], recovered.startup_ramp_frames),
        "retry callback must contain every frame exactly, without boundary fade"
    );
    retry.cancel().expect("close retry producer");
    late.shutdown().expect("close late listener");
    listener.shutdown().expect("close original listener");
    host.shutdown().expect("close host");
    fs::remove_dir_all(root).expect("fixture cleanup");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "integrated two-boundary failure, socket Stop, late join and native retry"
)]
fn failure_after_audible_successor_stops_without_skipping_then_retries_three_tracks() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-socket-late-successor-failure-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    let repaired = root.join("repaired.wav");
    let unavailable = root.join("unavailable.wav");
    fs::write(&first, source_wav_format(3_000, 48_000, 2, 480)).expect("first source");
    fs::write(&second, source_wav_format(6_000, 44_100, 1, 441)).expect("mixed-rate successor");
    fs::write(&repaired, source_wav_format(9_000, 48_000, 2, 480)).expect("repaired third source");

    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("failure-after-boundary-stream").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let failing = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![
            source("first", &first),
            source("second", &second),
            source("missing-third", &unavailable),
        ],
    )
    .expect("lazy third source is not opened before playback");

    // The second track is fully audible before the third decoder fails.
    // Failure must not erase its committed boundary or inject any PCM for
    // the missing track into the same native output generation.
    let before = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        None,
        &failing,
        &session,
        &stream,
        1_000,
        &[241, 337, 382],
        true,
        false,
    );
    assert_eq!(before.packets, 4, "two complete 480-frame sources");
    assert_eq!(before.boundaries, [480], "only the audible boundary");
    assert!(
        before
            .failure
            .as_deref()
            .is_some_and(|error| error.contains("missing-third")),
        "failed third source must be identified"
    );
    assert_eq!(
        before.pcm,
        expected_pcm(&[3_000, 6_000], before.startup_ramp_frames),
        "mixed-rate second track must reach the callback without a gap"
    );

    // The host waits for the final valid sample (960 / 48 kHz = 20 ms),
    // plus the 200 ms Pause/Resume shift, then retires the failed stream. A post-Stop listener must not
    // receive the old start or the already-audible second-track boundary.
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_220),
    });
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("Stop after last valid audio")
            .report
            .successful_peers,
        1
    );
    assert!(recv_control(listener.as_mut()) == stop);
    let control_count = host.counters().control_frames_sent;
    let mut late = authorized_late_joiner(host.as_mut(), "late-after-second-track-failure");
    assert_eq!(
        host.counters().control_frames_sent,
        control_count,
        "failed generation must not be replayed to late join"
    );
    let stale_boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "second".to_owned(),
        display_name: "second".to_owned(),
    });
    host.broadcast_control(&stale_boundary)
        .expect("retired boundary is safely ignored");
    assert_eq!(
        host.counters().control_frames_sent,
        control_count,
        "late boundary cannot resurrect the stopped stream"
    );
    let stale_audio = AudioDatagram {
        session_id: session.clone(),
        stream_id: stream.clone(),
        sequence: PacketSequence::new(4),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        first_sample_index: SampleIndex::new(960),
        host_presentation_time_ms: MonotonicMillis::new(1_221),
        payload: vec![0; 240 * 2 * 2],
    };
    assert_eq!(
        host.broadcast_audio(&ProtocolFrame::Audio(stale_audio))
            .expect("retired producer audio is ignored")
            .report
            .intended_peers,
        0
    );
    failing.cancel().expect("release failed producer");

    // Repair and explicitly Play again, reusing the same stream ID with a
    // newer anchor. Both sockets must receive the complete new generation,
    // including two boundaries and all 1,440 native callback frames.
    let retry = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        2_000,
        vec![
            source("first", &first),
            source("second", &second),
            source("repaired-third", &repaired),
        ],
    )
    .expect("retry repaired queue");
    let recovered = playback_generation(
        host.as_mut(),
        listener.as_mut(),
        Some(late.as_mut()),
        &retry,
        &session,
        &stream,
        2_000,
        &[241, 337, 382, 480],
        false,
        false,
    );
    assert_ne!(recovered.output_token, before.output_token);
    assert!(
        recovered.failure.is_none(),
        "repaired queue reaches natural end"
    );
    assert_eq!(recovered.packets, 6, "fresh sequence starts at zero");
    assert_eq!(recovered.boundaries, [480, 960]);
    assert_eq!(
        recovered.pcm,
        expected_pcm(&[3_000, 6_000, 9_000], recovered.startup_ramp_frames),
        "retry must contain no old PCM, silence, or extra transition fade"
    );
    retry.cancel().expect("release retry producer");
    late.shutdown().expect("late listener shutdown");
    listener.shutdown().expect("original listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("remove fixture directory");
}
