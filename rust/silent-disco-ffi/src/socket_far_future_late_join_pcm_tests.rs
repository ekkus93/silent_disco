//! Production decoded playlist -> authenticated TCP/UDP late join -> scheduler
//! -> native C-ABI callback after the host has already sent 500 packets.
//! A listener must not invent a three-packet startup gap while corroborating
//! a far-future live sequence. No physical Oboe/DAC output is claimed.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, register_render_ring, registry_test_guard,
    release_render_ring, silent_disco_audio_callback_count,
    silent_disco_audio_read_interleaved_f32, silent_disco_audio_silence_filled_frames,
    silent_disco_audio_underrun_count,
};
use crate::callback_capture::CallbackCaptureTap;
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame};
use crate::playlist_pcm_capture_tests::source_wav_format;
use crate::socket_failure_recovery_pcm_tests::source;
use crate::socket_pause_resume_pcm_tests::{authorized_late_joiner, recv_audio, recv_control};
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig,
};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, ProtocolFrame, StreamStart, TrackBoundary,
};
use silent_disco_core::transport::{
    HostTransportConfig, ManualTransportClock, SocketTransportFactory, TransportFactory,
};
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one decoded long playlist, mid-track socket admission and native PCM callback"
)]
fn late_join_after_reorder_window_renders_all_live_pcm_without_bootstrap_concealment() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-live-edge-pcm-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("long-first.wav");
    let second = root.join("late-current.wav");
    // 500 * 240 samples at 48 kHz is 2,500ms. A new listener joining at
    // that boundary has no packet-zero history and is well past the 1,280ms
    // reorder window. Its first audible source must be the second track.
    fs::write(&first, source_wav_format(1_024, 48_000, 2, 500 * 240))
        .expect("stage long first track");
    fs::write(&second, source_wav_format(2_048, 48_000, 2, 5 * 240))
        .expect("stage live second track");
    let session = SessionId::new("end-to-end-session").expect("session");
    let stream = StreamId::new("live-edge-late-join").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(3_500));
    let mut host = SocketTransportFactory
        .bind_host(HostTransportConfig::loopback(session.clone()), clock)
        .expect("production TCP/UDP host");
    let playlist = FfiHostPlaylistStreamer::open(
        session.to_string(),
        stream.to_string(),
        1_000,
        vec![source("long-first", &first), source("late-current", &second)],
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
        host.broadcast_control(&start).expect("cache start").report.intended_peers,
        0
    );
    let mut sequence = 0_u64;
    let mut boundary_seen = false;
    for _ in 0..510 {
        match playlist.next_frame(5_000).expect("production packetizer") {
            Some(FfiPlaylistFrame::Audio { sequence: packet, first_sample_index, .. }) => {
                assert!(!boundary_seen, "the live second track is handled below");
                assert_eq!(packet, sequence);
                assert_eq!(first_sample_index, sequence * 240);
                sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary { first_sample_index, entry_id, display_name, .. }) => {
                assert_eq!(sequence, 500);
                assert_eq!(first_sample_index, 120_000);
                assert_eq!(entry_id, "late-current");
                let boundary = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id,
                    display_name,
                });
                host.broadcast_control(&boundary).expect("cache audible boundary");
                boundary_seen = true;
                break;
            }
            other => panic!("expected long first track and boundary, got {other:?}"),
        }
    }
    assert!(boundary_seen);
    let mut listener = authorized_late_joiner(host.as_mut(), "far-future-callback-listener");
    let ControlMessage::StreamStart(replayed) = recv_control(listener.as_mut()) else {
        panic!("late join must receive the current authoritative track");
    };
    assert_eq!(replayed.host_start_time_ms.get(), 1_000);
    assert_eq!(replayed.current_entry_id.as_deref(), Some("late-current"));
    assert_eq!(replayed.current_display_name.as_deref(), Some("late-current"));

    let mut config = SchedulerConfig::new(session.clone(), stream.clone(), 48_000, 1_000, 240, 2);
    config.startup_buffer_target_ms = 0;
    let ramp = usize::try_from(
        u64::from(config.sample_rate) * u64::from(config.concealment_ramp_ms) / 1_000,
    ).expect("ramp");
    let (producer, token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 4_800,
    }).expect("one native output");
    let mut pump = PlaybackPump::new(
        PlaybackScheduler::new(config, 0.0).expect("listener scheduler"),
        producer,
        PlaybackPumpConfig {
            volume: 1.0,
            write_lead_ms: 0,
            max_prefill_ms: 0,
            target_depth_frames: 4_800,
        },
    ).expect("listener pump");
    pump.apply_sync_offset(0.0);
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("callback tap"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach tap");
    for expected_sequence in 500..505 {
        let Some(FfiPlaylistFrame::Audio {
            sequence, sample_rate, channels, samples_per_packet,
            first_sample_index, host_presentation_time_ms, payload, ..
        }) = playlist.next_frame(5_000).expect("live packet") else {
            panic!("expected second-track PCM");
        };
        assert_eq!(sequence, expected_sequence);
        assert_eq!(first_sample_index, expected_sequence * 240);
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
                .expect("live UDP").report.successful_peers,
            1
        );
        let received = recv_audio(listener.as_mut());
        assert_eq!(received, packet, "real socket must preserve exact live PCM");
        let submitted = pump.scheduler_mut().submit_packet(received);
        if expected_sequence < 502 {
            assert!(submitted.is_err(), "two candidates require corroboration");
        } else {
            submitted.expect("live-edge packets must recover in sequence");
        }
        // All three corroborating packets arrive while the first slot is
        // still reachable; subsequent ticks follow the live presentation
        // clock. No synthetic gap or stale first-track PCM is permitted.
        if expected_sequence >= 502 {
            let now = 3_500 + (expected_sequence - 502) * 5;
            for _ in 0..64 {
                if !matches!(pump.tick(now), PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }) {
                    break;
                }
            }
        }
    }
    assert!(matches!(
        playlist.next_frame(5_000).expect("natural queue end"),
        Some(FfiPlaylistFrame::End)
    ));
    for now in [3_515, 3_520] {
        for _ in 0..64 {
            if !matches!(pump.tick(now), PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }) {
                break;
            }
        }
    }
    assert_eq!(pump.diagnostics().packets_emitted, 5);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    assert_eq!(pump.scheduler_mut().jitter_statistics().resynchronisations, 1);
    assert_eq!(pump.scheduler_mut().jitter_statistics().skipped, 0);

    let native = usize::try_from(token).expect("token") as *mut core::ffi::c_void;
    let mut actual = Vec::new();
    for frames in [241_u32, 337, 382, 240] {
        let mut output = vec![f32::NAN; usize::try_from(frames).expect("frames") * 2];
        let mut supplied = 0_u32;
        assert_eq!(
            silent_disco_audio_read_interleaved_f32(
                native, output.as_mut_ptr(), frames, 2, &raw mut supplied,
            ),
            0,
            "no native callback underrun on the live edge"
        );
        assert_eq!(supplied, frames);
        assert_eq!(tap.drain(usize::try_from(frames).expect("frames")), output);
        actual.extend(output);
    }
    let expected: Vec<f32> = (0..1_200_usize)
        .flat_map(|frame| {
            let sample = if frame < ramp {
                2_048_i32 * i32::try_from(frame).expect("frame") / i32::try_from(ramp).expect("ramp")
            } else {
                2_048_i32
            };
            std::iter::repeat_n(f32::from(i16::try_from(sample).expect("sample")) / 32768.0, 2)
        })
        .collect();
    assert_eq!(actual, expected, "exact second-track PCM, only one startup ramp");
    assert_eq!(silent_disco_audio_callback_count(native.cast_const()), 4);
    assert_eq!(silent_disco_audio_underrun_count(native.cast_const()), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(native.cast_const()), 0);
    assert_eq!(tap.counters(), (1_200, 0, 4));
    detach_callback_capture(token);
    release_render_ring(token).expect("retire one output");
    playlist.cancel().expect("release producer");
    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
