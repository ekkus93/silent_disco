//! Production real-socket same-identity route takeover with a paused
//! playlist boundary, scheduler PCM, and one persistent native callback token.
//! This is a deterministic software proof, not physical Oboe/DAC acceptance.
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
    SchedulerConfig, StreamingDecodeConfig, StreamingDecodeHandle,
};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, Pause, ProtocolFrame, Stop, StreamStart,
    TrackBoundary,
};
use silent_disco_core::transport::ListenerTransportNode;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one real-socket route takeover, source decode, scheduler and native callback proof"
)]
fn same_identity_reconnect_at_paused_boundary_keeps_one_native_output_and_exact_pcm() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-same-id-pcm-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let formats = [
        (48_000_u32, 2_u16, 480_usize, 1_024_i16),
        (44_100, 1, 441, 2_048),
        (96_000, 2, 960, 4_096),
    ];
    let mut reference = Vec::<i16>::new();
    let mut sources = Vec::new();
    for (index, &(rate, channels, frames, sample)) in formats.iter().enumerate() {
        let path = root.join(format!("track-{index}.wav"));
        fs::write(&path, source_wav_format(sample, rate, channels, frames))
            .expect("stage mixed-format WAV");
        let decoder = StreamingDecodeHandle::open(&path, StreamingDecodeConfig::default())
            .expect("independent canonical decoder");
        loop {
            let chunk = decoder
                .recv_timeout(Duration::from_secs(5))
                .expect("decode canonical PCM");
            let end = chunk.end_of_stream;
            reference.extend(chunk.frames);
            if end {
                break;
            }
        }
        decoder.join().expect("join canonical decoder");
        sources.push(FfiPlaylistSource {
            entry_id: format!("entry-{index}"),
            display_name: format!("Track {index}"),
            staged_path: path.to_string_lossy().into_owned(),
        });
    }
    assert_eq!(reference.len(), 1_440 * 2);
    let (mut host, mut first) = authorized_pcm_socket_pair();
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("same-identity-pcm").expect("stream");
    let playlist =
        FfiHostPlaylistStreamer::open(session.to_string(), stream.to_string(), 1_000, sources)
            .expect("production playlist");
    let metadata = playlist.stream_start();
    let start = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: metadata.sample_rate,
        channels: metadata.channels,
        samples_per_packet: metadata.samples_per_packet,
        current_entry_id: metadata.current_entry_id,
        current_display_name: metadata.current_display_name,
    });
    assert_eq!(
        host.broadcast_control(&start)
            .expect("start TCP")
            .report
            .successful_peers,
        1
    );
    assert!(recv_control(first.as_mut()) == start);

    let mut config = SchedulerConfig::new(
        session.clone(),
        stream.clone(),
        48_000,
        1_000,
        metadata.samples_per_packet,
        2,
    );
    config.startup_buffer_target_ms = 0;
    let ramp = usize::try_from(
        u64::from(config.sample_rate) * u64::from(config.concealment_ramp_ms) / 1_000,
    )
    .expect("startup ramp");
    let (producer, token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 4_800,
    })
    .expect("single native ring");
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
    .expect("production pump");
    pump.apply_sync_offset(0.0);
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("capture tap"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach tap");
    let native = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
    let captured = std::cell::RefCell::new(Vec::new());
    let mut callback = |frames: u32| {
        let mut output = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
        let mut supplied = 0;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                native,
                output.as_mut_ptr(),
                frames,
                2,
                &raw mut supplied,
            ),
            0,
            "one output token must render without gaps"
        );
        assert_eq!(supplied, frames);
        assert_eq!(
            tap.drain(usize::try_from(frames).expect("frames")),
            output,
            "native callback and capture tap must agree"
        );
        captured.borrow_mut().extend(output);
    };

    let mut replacement: Option<Box<dyn ListenerTransportNode>> = None;
    let mut active_start = start.clone();
    let mut boundary_controls = Vec::new();
    let mut next_sample = 0_u64;
    let mut next_sequence = 0_u64;
    let mut boundaries = Vec::new();
    let mut finished = false;
    for _ in 0..32 {
        match playlist.next_frame(5_000).expect("playlist event") {
            Some(FfiPlaylistFrame::Audio {
                sequence,
                sample_rate,
                channels,
                samples_per_packet,
                first_sample_index,
                host_presentation_time_ms,
                payload,
                ..
            }) => {
                assert_eq!(sequence, next_sequence, "no lost/repeated packets");
                assert_eq!(first_sample_index, next_sample, "no PCM gap/overlap");
                let shift = if first_sample_index >= 960 {
                    400
                } else if replacement.is_some() {
                    200
                } else {
                    0
                };
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
                        host_presentation_time_ms + shift,
                    ),
                    payload,
                };
                assert_eq!(
                    host.broadcast_audio(&ProtocolFrame::Audio(packet.clone()))
                        .expect("UDP delivery")
                        .report
                        .successful_peers,
                    1,
                    "route replacement must not duplicate audio delivery"
                );
                let received = if let Some(ref mut new_peer) = replacement {
                    recv_audio(new_peer.as_mut())
                } else {
                    recv_audio(first.as_mut())
                };
                assert_eq!(received, packet, "socket preserves exact PCM metadata");
                pump.scheduler_mut()
                    .submit_packet(received)
                    .expect("schedule production UDP PCM");
                for _ in 0..64 {
                    if !matches!(
                        pump.tick(host_presentation_time_ms + shift),
                        PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
                    ) {
                        break;
                    }
                }
                next_sample += u64::from(samples_per_packet);
                next_sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index,
                entry_id,
                display_name,
                ..
            }) => {
                assert!(
                    matches!(first_sample_index, 480 | 960),
                    "only the two canonical track boundaries"
                );
                assert_eq!(
                    next_sample, first_sample_index,
                    "boundary follows all preceding PCM"
                );
                let boundary = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id,
                    display_name,
                });
                assert_eq!(
                    host.broadcast_control(&boundary)
                        .expect("boundary TCP")
                        .report
                        .successful_peers,
                    1
                );
                if let Some(ref mut peer) = replacement {
                    assert!(recv_control(peer.as_mut()) == boundary);
                } else {
                    assert!(recv_control(first.as_mut()) == boundary);
                }
                boundaries.push(first_sample_index);
                boundary_controls.push(boundary);
                let pause_anchor = if first_sample_index == 480 {
                    1_010
                } else {
                    1_220
                };
                let pause = ControlMessage::Pause(Pause {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    host_pause_time_ms: MonotonicMillis::new(pause_anchor),
                });
                assert_eq!(
                    host.broadcast_control(&pause)
                        .expect("Pause TCP")
                        .report
                        .successful_peers,
                    1
                );
                if let Some(ref mut peer) = replacement {
                    assert!(recv_control(peer.as_mut()) == pause);
                } else {
                    assert!(recv_control(first.as_mut()) == pause);
                }

                // A callback can cross a track boundary even while the
                // host is paused: the same output ring survives both route
                // takeovers and both presentation-clock re-anchors.
                callback(if first_sample_index == 480 { 241 } else { 337 });
                assert_eq!(
                    silent_disco_audio_callback_count(native.cast_const()),
                    u64::try_from(boundaries.len()).expect("bounded callbacks"),
                    "Pause and route replacement must not reopen the output"
                );

                // Reconnect using the SAME identity while the previous TCP
                // reader is still alive. Every boundary must be replayed
                // before Pause, with the latest authoritative Start anchor.
                let mut returning =
                    authorized_late_joiner(host.as_mut(), "pcm-regression-listener");
                assert!(recv_control(returning.as_mut()) == active_start);
                for cached in &boundary_controls {
                    assert!(recv_control(returning.as_mut()) == *cached);
                }
                assert!(recv_control(returning.as_mut()) == pause);
                if let Some(mut old) = replacement.take() {
                    old.shutdown().expect("retire previous replacement");
                } else {
                    first.shutdown().expect("retire original socket");
                }

                let ControlMessage::StreamStart(mut resumed) = active_start.clone() else {
                    panic!("start fixture");
                };
                let resume_anchor = if first_sample_index == 480 {
                    1_200
                } else {
                    1_400
                };
                resumed.host_start_time_ms = MonotonicMillis::new(resume_anchor);
                let resume = ControlMessage::StreamStart(resumed);
                assert_eq!(
                    host.broadcast_control(&resume)
                        .expect("Resume TCP")
                        .report
                        .successful_peers,
                    1
                );
                assert!(recv_control(returning.as_mut()) == resume);
                pump.scheduler_mut().set_host_start_time_ms(resume_anchor);
                active_start = resume;
                replacement = Some(returning);
            }
            Some(FfiPlaylistFrame::End) => {
                finished = true;
                break;
            }
            None => {}
        }
    }
    assert!(finished, "playlist reaches natural end");
    assert_eq!(next_sequence, 6);
    assert_eq!(next_sample, 1_440);
    assert_eq!(boundaries, [480, 960]);
    callback(382);
    callback(480);
    assert_eq!(captured.borrow().len(), reference.len());
    let mut expected: Vec<f32> = reference
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    for frame in 0..ramp.min(480) {
        for channel in 0..2 {
            let index = frame * 2 + channel;
            let faded = i32::from(reference[index]) * i32::try_from(frame).expect("ramp frame")
                / i32::try_from(ramp).expect("ramp length");
            expected[index] = f32::from(i16::try_from(faded).expect("fade sample")) / 32768.0;
        }
    }
    assert_eq!(
        *captured.borrow(), expected,
        "exact PCM across takeover and boundary"
    );
    assert_eq!(pump.diagnostics().packets_emitted, 6);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    assert_eq!(silent_disco_audio_callback_count(native.cast_const()), 4);
    assert_eq!(silent_disco_audio_underrun_count(native.cast_const()), 0);
    assert_eq!(
        silent_disco_audio_silence_filled_frames(native.cast_const()),
        0
    );
    assert_eq!(tap.counters(), (1_440, 0, 4));
    detach_callback_capture(token);
    release_render_ring(token).expect("release one native output");
    playlist.cancel().expect("cancel producer");

    // Stop retires this generation but not the host session. No newly
    // authorized listener may inherit the two old track boundaries or Pause.
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(2_000),
    });
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("broadcast Stop")
            .report
            .successful_peers,
        1
    );
    assert!(recv_control(replacement.as_mut().expect("returning peer").as_mut()) == stop);
    let controls_at_stop = host.counters().control_frames_sent;
    let mut after_stop = authorized_late_joiner(host.as_mut(), "after-stop-replay");
    assert_eq!(
        host.counters().control_frames_sent,
        controls_at_stop,
        "stopped generation must not replay cached tracks to a late joiner"
    );
    assert_eq!(
        host.broadcast_control(&active_start)
            .expect("reject retired StreamStart")
            .report
            .intended_peers,
        0
    );
    for control in &boundary_controls {
        assert_eq!(
            host.broadcast_control(control)
                .expect("reject retired boundary")
                .report
                .intended_peers,
            0
        );
    }
    assert_eq!(host.counters().control_frames_sent, controls_at_stop);

    // Intentionally REUSE the stream ID with a newer start anchor. Replay
    // reverses two previously staged sources and creates a new output token;
    // source-order errors cannot pass an exact callback PCM comparison.
    let replay_sources = [2_usize, 0].map(|index| FfiPlaylistSource {
        entry_id: format!("replay-{index}"),
        display_name: format!("Replay {index}"),
        staged_path: root
            .join(format!("track-{index}.wav"))
            .to_string_lossy()
            .into_owned(),
    });
    let replay = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        3_000,
        replay_sources.to_vec(),
    )
    .expect("open fresh production playlist");
    let replay_metadata = replay.stream_start();
    let replay_start = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(3_000),
        sample_rate: replay_metadata.sample_rate,
        channels: replay_metadata.channels,
        samples_per_packet: replay_metadata.samples_per_packet,
        current_entry_id: replay_metadata.current_entry_id,
        current_display_name: replay_metadata.current_display_name,
    });
    assert_eq!(
        host.broadcast_control(&replay_start)
            .expect("new Play broadcast")
            .report
            .successful_peers,
        2
    );
    assert!(recv_control(replacement.as_mut().expect("returning peer").as_mut()) == replay_start);
    assert!(recv_control(after_stop.as_mut()) == replay_start);
    let (replay_producer, replay_token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 4_800,
    })
    .expect("fresh output for Play after Stop");
    assert_ne!(
        replay_token, token,
        "Stop/Play must rotate native output token"
    );
    let replay_tap = Arc::new(CallbackCaptureTap::new(4_800).expect("fresh capture"));
    attach_callback_capture(replay_token, Arc::clone(&replay_tap))
        .expect("attach fresh callback capture");
    let mut replay_pump = PlaybackPump::new(
        PlaybackScheduler::new(
            SchedulerConfig {
                startup_buffer_target_ms: 0,
                ..SchedulerConfig::new(
                    session.clone(),
                    stream.clone(),
                    48_000,
                    3_000,
                    replay_metadata.samples_per_packet,
                    2,
                )
            },
            0.0,
        )
        .expect("fresh listener scheduler"),
        replay_producer,
        PlaybackPumpConfig {
            volume: 1.0,
            write_lead_ms: 0,
            max_prefill_ms: 0,
            target_depth_frames: 4_800,
        },
    )
    .expect("fresh listener pump");
    replay_pump.apply_sync_offset(0.0);
    let mut replay_sample = 0_u64;
    let mut replay_sequence = 0_u64;
    let mut replay_boundaries = Vec::new();
    let mut replay_ended = false;
    for _ in 0..24 {
        match replay.next_frame(5_000).expect("replay source frame") {
            Some(FfiPlaylistFrame::Audio {
                sequence,
                sample_rate,
                channels,
                samples_per_packet,
                first_sample_index,
                host_presentation_time_ms,
                payload,
                ..
            }) => {
                assert_eq!(sequence, replay_sequence, "replay packet sequence reset");
                assert_eq!(
                    first_sample_index, replay_sample,
                    "replay frame clock reset"
                );
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
                        .expect("replay audio broadcast")
                        .report
                        .successful_peers,
                    2
                );
                let returning_packet =
                    recv_audio(replacement.as_mut().expect("returning peer").as_mut());
                let late_packet = recv_audio(after_stop.as_mut());
                assert_eq!(returning_packet, packet);
                assert_eq!(late_packet, packet);
                replay_pump
                    .scheduler_mut()
                    .submit_packet(returning_packet)
                    .expect("schedule replay PCM");
                for _ in 0..64 {
                    if !matches!(
                        replay_pump.tick(host_presentation_time_ms),
                        PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
                    ) {
                        break;
                    }
                }
                replay_sample += u64::from(samples_per_packet);
                replay_sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index,
                entry_id,
                display_name,
                ..
            }) => {
                assert_eq!(first_sample_index, replay_sample);
                let boundary = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id,
                    display_name,
                });
                assert_eq!(
                    host.broadcast_control(&boundary)
                        .expect("replay boundary broadcast")
                        .report
                        .successful_peers,
                    2
                );
                assert!(
                    recv_control(replacement.as_mut().expect("returning peer").as_mut())
                        == boundary
                );
                assert!(recv_control(after_stop.as_mut()) == boundary);
                replay_boundaries.push(first_sample_index);
            }
            Some(FfiPlaylistFrame::End) => {
                replay_ended = true;
                break;
            }
            None => {}
        }
    }
    assert!(replay_ended, "fresh playlist must end cleanly");
    assert_eq!(replay_sequence, 4, "two replay tracks of 480 frames each");
    assert_eq!(replay_sample, 960);
    assert_eq!(replay_boundaries, [480]);

    // The old C-ABI callback must report Stopping and fill silence even
    // while a new output ring has queued real replay PCM. In particular,
    // it must not drain the new ring or contaminate the new diagnostic tap.
    let mut retired_output = vec![f32::NAN; 17 * 2];
    let mut retired_supplied = u32::MAX;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            native,
            retired_output.as_mut_ptr(),
            17,
            2,
            &raw mut retired_supplied,
        ),
        2,
        "a callback from before Stop cannot render after replay"
    );
    assert_eq!(retired_supplied, 0);
    assert!(retired_output.iter().all(|sample| sample.to_bits() == 0));
    assert_eq!(replay_tap.counters(), (0, 0, 0));

    let replay_native =
        usize::try_from(replay_token).expect("fresh token") as *mut core::ffi::c_void;
    let mut replay_pcm = Vec::new();
    for frames in [239_u32, 242, 479] {
        let mut output = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
        let mut supplied = 0;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                replay_native,
                output.as_mut_ptr(),
                frames,
                2,
                &raw mut supplied,
            ),
            0
        );
        assert_eq!(supplied, frames);
        assert_eq!(
            replay_tap.drain(usize::try_from(frames).expect("frames")),
            output
        );
        replay_pcm.extend(output);
    }
    let replay_reference: Vec<i16> = reference[960 * 2..]
        .iter()
        .chain(reference[..480 * 2].iter())
        .copied()
        .collect();
    let mut replay_expected: Vec<f32> = replay_reference
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    for frame in 0..ramp.min(480) {
        for channel in 0..2 {
            let index = frame * 2 + channel;
            let faded = i32::from(replay_reference[index])
                * i32::try_from(frame).expect("ramp frame")
                / i32::try_from(ramp).expect("ramp length");
            replay_expected[index] =
                f32::from(i16::try_from(faded).expect("fade sample")) / 32768.0;
        }
    }
    assert_eq!(
        replay_pcm, replay_expected,
        "exact reordered PCM after Stop/Play"
    );
    assert_eq!(replay_pump.diagnostics().packets_emitted, 4);
    assert_eq!(replay_pump.diagnostics().concealed_packets, 0);
    assert_eq!(
        silent_disco_audio_callback_count(replay_native.cast_const()),
        3
    );
    assert_eq!(
        silent_disco_audio_underrun_count(replay_native.cast_const()),
        0
    );
    assert_eq!(
        silent_disco_audio_silence_filled_frames(replay_native.cast_const()),
        0
    );
    assert_eq!(replay_tap.counters(), (960, 0, 3));
    detach_callback_capture(replay_token);
    release_render_ring(replay_token).expect("release replay output");
    replay.cancel().expect("cancel replay packetizer");
    after_stop.shutdown().expect("close replay late joiner");
    replacement
        .expect("reconnected socket")
        .shutdown()
        .expect("close returning socket");
    host.shutdown().expect("close host");
    fs::remove_dir_all(root).expect("remove fixtures");
}
