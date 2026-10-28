//! Interleaved native callbacks while decoded PCM traverses real TCP/UDP
//! sockets and the listener scheduler. Pause, reconnect and queue boundaries
//! must not require a new native output. Legacy v3 sockets are plaintext;
//! physical audio and secure transport acceptance remain separate.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, register_render_ring, registry_test_guard,
    release_render_ring, silent_disco_audio_callback_count,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_silence_filled_frames,
    silent_disco_audio_underrun_count,
};
use crate::callback_capture::CallbackCaptureTap;
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame};
use crate::playlist_pcm_capture_tests::{authorized_pcm_socket_pair, source_wav_format};
use crate::socket_failure_recovery_pcm_tests::{expected_pcm, source};
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_audio, recv_control};
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
use silent_disco_core::transport::ListenerTransportNode;
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn read_callback(token: u64, frames: u32, tap: &CallbackCaptureTap) -> Vec<f32> {
    let mut output = vec![f32::NAN; frames as usize * 2];
    let mut supplied = 0_u32;
    let native = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
    assert_eq!(
        silent_disco_audio_read_interleaved_f32(
            native,
            output.as_mut_ptr(),
            frames,
            2,
            &raw mut supplied,
        ),
        0,
        "callback must not underrun while sockets continue streaming"
    );
    assert_eq!(supplied, frames);
    assert_eq!(
        tap.drain(frames as usize),
        output,
        "nonblocking capture must agree with actual callback return"
    );
    output
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one real-socket streaming/callback, pause, reconnect and Stop lifecycle"
)]
fn live_callback_reads_between_socket_packets_keep_output_open_across_pause_and_reconnect() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-interleaved-callback-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("first.wav");
    let second = root.join("second.wav");
    let third = root.join("third.wav");
    fs::write(&first, source_wav_format(3_000, 48_000, 2, 480))
        .expect("first canonical source");
    fs::write(&second, source_wav_format(7_000, 44_100, 1, 441))
        .expect("mixed-rate mono successor");
    fs::write(&third, source_wav_format(9_000, 48_000, 2, 480))
        .expect("final canonical source");

    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("interleaved-native-callback").expect("stream");
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let playlist = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![
            source("first", &first),
            source("mixed-rate", &second),
            source("last", &third),
        ],
    )
    .expect("production decoded playlist");
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
            .expect("broadcast start")
            .report
            .successful_peers,
        1
    );
    assert_eq!(recv_control(listener.as_mut()), start);

    let mut config = SchedulerConfig::new(session.clone(), stream.clone(), 48_000, 1_000, 240, 2);
    config.startup_buffer_target_ms = 0;
    let ramp = usize::try_from(
        u64::from(config.sample_rate) * u64::from(config.concealment_ramp_ms) / 1_000,
    )
    .expect("startup ramp");
    let (producer, token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 4_800,
    })
    .expect("one native output");
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
    .expect("listener playback pump");
    pump.apply_sync_offset(0.0);
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("callback tap"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach tap");

    let mut newcomer: Option<Box<dyn ListenerTransportNode>> = None;
    let mut captured = Vec::new();
    let mut packets = 0_u64;
    let mut boundaries = Vec::new();
    let mut resumed = false;
    let mut ended = false;
    for _ in 0..24 {
        match playlist.next_frame(5_000).expect("decoded production frame") {
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
                assert_eq!(sequence, packets);
                assert_eq!(first_sample_index, packets * 240);
                let now = host_presentation_time_ms + if resumed { 200 } else { 0 };
                let packet = AudioDatagram {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    sequence: PacketSequence::new(sequence),
                    codec: AudioCodec::PcmS16Le,
                    sample_rate,
                    channels,
                    samples_per_packet,
                    first_sample_index: SampleIndex::new(first_sample_index),
                    host_presentation_time_ms: MonotonicMillis::new(now),
                    payload,
                };
                assert_eq!(
                    host.broadcast_audio(&ProtocolFrame::Audio(packet.clone()))
                        .expect("send live PCM over UDP")
                        .report
                        .successful_peers,
                    if newcomer.is_some() { 2 } else { 1 }
                );
                let received = recv_audio(listener.as_mut());
                assert_eq!(received, packet, "exact socket PCM and presentation index");
                if let Some(peer) = newcomer.as_deref_mut() {
                    assert_eq!(recv_audio(peer), packet, "late joiner receives live PCM");
                }
                pump.scheduler_mut()
                    .submit_packet(received)
                    .expect("schedule socket PCM");
                for _ in 0..64 {
                    if !matches!(
                        pump.tick(now),
                        PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
                    ) {
                        break;
                    }
                }
                packets += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index,
                entry_id,
                display_name,
                ..
            }) => {
                assert_eq!(first_sample_index, packets * 240);
                let boundary = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id,
                    display_name,
                });
                assert_eq!(
                    host.broadcast_control(&boundary)
                        .expect("boundary over TCP")
                        .report
                        .successful_peers,
                    if newcomer.is_some() { 2 } else { 1 }
                );
                assert_eq!(recv_control(listener.as_mut()), boundary);
                if let Some(peer) = newcomer.as_deref_mut() {
                    assert_eq!(recv_control(peer), boundary);
                }
                boundaries.push(first_sample_index);
                if first_sample_index == 480 {
                    // The native callback is actively draining the ring
                    // before Pause. Keep its 239-frame tail across a
                    // same-identity TCP/UDP replacement and Resume.
                    captured.extend(read_callback(token, 241, &tap));
                    let pause = ControlMessage::Pause(Pause {
                        session_id: session.clone(),
                        stream_id: stream.clone(),
                        host_pause_time_ms: MonotonicMillis::new(1_010),
                    });
                    host.broadcast_control(&pause).expect("Pause TCP");
                    assert_eq!(recv_control(listener.as_mut()), pause);
                    let mut replacement =
                        authorized_late_joiner(host.as_mut(), "pcm-regression-listener");
                    for expected in [&start, &boundary, &pause] {
                        assert_eq!(
                            recv_control(replacement.as_mut()),
                            *expected,
                            "reconnect during Pause must replay authoritative controls"
                        );
                    }
                    listener.shutdown().expect("retire previous socket");
                    listener = replacement;
                    let mut extra =
                        authorized_late_joiner(host.as_mut(), "interleaved-pause-late");
                    for expected in [&start, &boundary, &pause] {
                        assert_eq!(recv_control(extra.as_mut()), *expected);
                    }
                    let ControlMessage::StreamStart(mut reanchored) = start.clone() else {
                        panic!("initial start");
                    };
                    reanchored.host_start_time_ms = MonotonicMillis::new(1_200);
                    let resume = ControlMessage::StreamStart(reanchored);
                    assert_eq!(
                        host.broadcast_control(&resume)
                            .expect("Resume TCP")
                            .report
                            .successful_peers,
                        2
                    );
                    assert_eq!(recv_control(listener.as_mut()), resume);
                    assert_eq!(recv_control(extra.as_mut()), resume);
                    pump.scheduler_mut().set_host_start_time_ms(1_200);
                    newcomer = Some(extra);
                    resumed = true;
                    let native = usize::try_from(token).expect("token") as *const core::ffi::c_void;
                    assert_eq!(silent_disco_audio_callback_count(native), 1);
                } else {
                    // Callback 2 crosses the first source boundary with
                    // 239 old frames and 98 new frames; no second ramp.
                    captured.extend(read_callback(token, 337, &tap));
                }
            }
            Some(FfiPlaylistFrame::End) => {
                ended = true;
                break;
            }
            None => {}
        }
    }
    assert!(ended, "three-track playlist must terminate");
    assert_eq!(packets, 6, "fresh continuous 5-ms packet sequence");
    assert_eq!(boundaries, [480, 960]);
    captured.extend(read_callback(token, 382, &tap));
    captured.extend(read_callback(token, 480, &tap));
    assert_eq!(
        captured,
        expected_pcm(&[3_000, 7_000, 9_000], ramp),
        "interleaved callbacks must preserve exact PCM across both boundaries"
    );
    let native = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    assert_eq!(silent_disco_audio_callback_count(native), 4);
    assert_eq!(silent_disco_audio_underrun_count(native), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(native), 0);
    assert_eq!(pump.diagnostics().packets_emitted, 6);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    assert_eq!(tap.counters(), (1_440, 0, 4));

    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_230),
    });
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("natural end Stop")
            .report
            .successful_peers,
        2
    );
    assert_eq!(recv_control(listener.as_mut()), stop);
    assert_eq!(recv_control(newcomer.as_deref_mut().expect("late listener")), stop);
    let stopped_controls = host.counters().control_frames_sent;
    let mut stopped_joiner = authorized_late_joiner(host.as_mut(), "interleaved-stopped-late");
    assert_eq!(
        host.counters().control_frames_sent,
        stopped_controls,
        "stopped late joiner must not receive cached StreamStart/Pause/boundaries"
    );
    stopped_joiner.shutdown().expect("stop late joiner");
    detach_callback_capture(token);
    release_render_ring(token).expect("release original native output");
    playlist.cancel().expect("release producer");
    newcomer.take().expect("late listener").shutdown().expect("close late");
    listener.shutdown().expect("close original listener");
    host.shutdown().expect("close host");
    fs::remove_dir_all(root).expect("cleanup fixtures");
}
