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
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig,
};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, ProtocolFrame, Stop, StreamStart, TrackBoundary,
};
use silent_disco_core::transport::{HostTransportNode, ListenerTransportNode};
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const TRACK_FRAMES: u32 = 480;

fn source(id: &str, path: &std::path::Path) -> FfiPlaylistSource {
    FfiPlaylistSource {
        entry_id: id.to_owned(),
        display_name: id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    }
}

struct CallbackPass {
    pcm: Vec<f32>,
    boundaries: Vec<u64>,
    packets: u64,
    failure: Option<String>,
    output_token: u64,
    startup_ramp_frames: usize,
}

/// One production playback generation, with optional second authorized socket.
/// The native output token stays unchanged across every packet and boundary.
#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one real-socket / production scheduler / callback playback generation"
)]
fn playback_generation(
    host: &mut dyn HostTransportNode,
    listener: &mut dyn ListenerTransportNode,
    second: Option<&mut dyn ListenerTransportNode>,
    playlist: &FfiHostPlaylistStreamer,
    session: &SessionId,
    stream: &StreamId,
    start_ms: u64,
    chunks: &[u32],
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
                    host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
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
                    assert_eq!(recv_audio(peer), packet, "second listener PCM exact");
                }
                pump.scheduler_mut()
                    .submit_packet(received)
                    .expect("schedule PCM");
                for _ in 0..64 {
                    if !matches!(
                        pump.tick(host_presentation_time_ms),
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
    assert!(completed || failure.is_some(), "producer did not terminate");
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

fn expected_pcm(samples: &[i16], ramp: usize) -> Vec<f32> {
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
