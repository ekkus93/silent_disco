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
    AudioCodec, AudioDatagram, ControlMessage, Pause, ProtocolFrame, StreamStart, TrackBoundary,
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
    assert_eq!(reference.len(), 960 * 2);
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
    let mut captured = Vec::new();
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
        captured.extend(output);
    };

    let mut replacement: Option<Box<dyn ListenerTransportNode>> = None;
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
                let shift = if replacement.is_some() { 200 } else { 0 };
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
                assert_eq!(first_sample_index, 480);
                assert_eq!(next_sample, 480, "boundary follows all first-track PCM");
                let boundary = ControlMessage::TrackBoundary(TrackBoundary {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    first_sample_index: SampleIndex::new(first_sample_index),
                    entry_id,
                    display_name,
                });
                host.broadcast_control(&boundary).expect("boundary TCP");
                assert!(recv_control(first.as_mut()) == boundary);
                boundaries.push(first_sample_index);
                let pause = ControlMessage::Pause(Pause {
                    session_id: session.clone(),
                    stream_id: stream.clone(),
                    host_pause_time_ms: MonotonicMillis::new(1_010),
                });
                host.broadcast_control(&pause).expect("Pause TCP");
                assert!(recv_control(first.as_mut()) == pause);
                callback(241);
                assert_eq!(
                    silent_disco_audio_callback_count(native.cast_const()),
                    1,
                    "Pause and route replacement must not reopen the output"
                );

                // Overlapping TCP connections deliberately claim the SAME
                // identity. Authorization must atomically replace old routes,
                // replay Start/Boundary/Pause, and survive old-reader cleanup.
                let mut returning =
                    authorized_late_joiner(host.as_mut(), "pcm-regression-listener");
                for expected in [&start, &boundary, &pause] {
                    assert!(recv_control(returning.as_mut()) == *expected);
                }
                first.shutdown().expect("retire original socket");
                let ControlMessage::StreamStart(mut resumed) = start.clone() else {
                    panic!("start fixture");
                };
                resumed.host_start_time_ms = MonotonicMillis::new(1_200);
                let resume = ControlMessage::StreamStart(resumed);
                assert_eq!(
                    host.broadcast_control(&resume)
                        .expect("Resume TCP")
                        .report
                        .successful_peers,
                    1
                );
                assert!(recv_control(returning.as_mut()) == resume);
                pump.scheduler_mut().set_host_start_time_ms(1_200);
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
    assert_eq!(next_sequence, 4);
    assert_eq!(next_sample, 960);
    assert_eq!(boundaries, [480]);
    callback(337);
    callback(382);
    assert_eq!(captured.len(), reference.len());
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
    assert_eq!(captured, expected, "exact PCM across takeover and boundary");
    assert_eq!(pump.diagnostics().packets_emitted, 4);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    assert_eq!(silent_disco_audio_callback_count(native.cast_const()), 3);
    assert_eq!(silent_disco_audio_underrun_count(native.cast_const()), 0);
    assert_eq!(
        silent_disco_audio_silence_filled_frames(native.cast_const()),
        0
    );
    assert_eq!(tap.counters(), (960, 0, 3));
    detach_callback_capture(token);
    release_render_ring(token).expect("release one native output");
    playlist.cancel().expect("cancel producer");
    replacement
        .expect("reconnected socket")
        .shutdown()
        .expect("close returning socket");
    host.shutdown().expect("close host");
    fs::remove_dir_all(root).expect("remove fixtures");
}
