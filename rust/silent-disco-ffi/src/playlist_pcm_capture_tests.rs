//! End-to-end software PCM continuity: real host source decoding and
//! packetization through the native callback return buffer. Transport and the
//! Android device mixer are separate outstanding acceptance gates.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_available_frames, silent_disco_audio_callback_count,
    silent_disco_audio_frames_rendered, silent_disco_audio_read_interleaved_f32,
    silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
};
use crate::audio_abi::{register_render_ring, release_render_ring};
use crate::audio_output::FfiAudioOutputHandle;
use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig, StreamingDecodeConfig, StreamingDecodeHandle,
};
use silent_disco_core::domain::{
    DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
    StreamStart, TrackBoundary,
};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportEvent,
    TransportFactory,
};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TRACK_FRAMES: usize = 480;
const CHANNELS: usize = 2;

pub(super) fn source_wav_format(sample: i16, sample_rate: u32, channels: u16, frames: usize) -> Vec<u8> {
    let bytes = frames * usize::from(channels) * 2;
    let mut data = Vec::with_capacity(44 + bytes);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(
        &u32::try_from(36 + bytes)
            .expect("fixture size")
            .to_le_bytes(),
    );
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16_u32.to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
    data.extend_from_slice(&channels.to_le_bytes());
    data.extend_from_slice(&sample_rate.to_le_bytes());
    data.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
    data.extend_from_slice(&(channels * 2).to_le_bytes());
    data.extend_from_slice(&16_u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&u32::try_from(bytes).expect("fixture").to_le_bytes());
    for _ in 0..frames * usize::from(channels) {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    data
}

fn source_wav(sample: i16) -> Vec<u8> {
    source_wav_format(sample, 48_000, 2, TRACK_FRAMES)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one combined decoder, playlist boundary and native callback regression"
)]
fn real_playlist_pcm_survives_multiple_boundaries_through_native_output_callback() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "silent-disco-real-playlist-callback-{}-{unique}",
        std::process::id(),
    ));
    fs::create_dir_all(&root).expect("fixture directory");

    let values = [1_024_i16, 2_048, 4_096];
    let mut sources = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let file = root.join(format!("track-{index}.wav"));
        fs::write(&file, source_wav(*value)).expect("write source WAV");
        sources.push(FfiPlaylistSource {
            entry_id: format!("entry-{index}"),
            display_name: format!("Track {index}"),
            staged_path: file.to_string_lossy().into_owned(),
        });
    }
    let producer = FfiHostPlaylistStreamer::open(
        "pcm-reference-session".to_owned(),
        "pcm-reference-stream".to_owned(),
        1_000,
        sources,
    )
    .expect("production playlist worker opens");
    let stream_start = producer.stream_start();
    assert_eq!(stream_start.sample_rate, 48_000);
    assert_eq!(stream_start.channels, 2);

    let output = FfiAudioOutputHandle::open(4_800, 1).expect("register output ring");
    let token = output.engine_token().expect("token");
    let tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("tap"));
    attach_callback_capture(u64::try_from(token).expect("token"), Arc::clone(&tap))
        .expect("attach tap");

    let mut audio_frames = 0_u64;
    let mut next_sequence = 0_u64;
    let mut boundaries = Vec::new();
    let mut reached_end = false;
    for _ in 0..64 {
        match producer.next_frame(1_000).expect("producer frame") {
            Some(FfiPlaylistFrame::Audio {
                first_sample_index,
                sequence,
                payload,
                channels,
                samples_per_packet,
                ..
            }) => {
                assert_eq!(first_sample_index, audio_frames, "no PCM gap/overlap");
                assert_eq!(sequence, next_sequence, "no sequence gap/overlap");
                assert_eq!(channels, 2);
                assert_eq!(
                    payload.len(),
                    usize::try_from(samples_per_packet).expect("packet") * 4
                );
                let decoded: Vec<f32> = payload
                    .chunks_exact(2)
                    .map(|bytes| f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.0)
                    .collect();
                assert_eq!(
                    output.push_frames(decoded).expect("write decoded payload"),
                    samples_per_packet
                );
                audio_frames += u64::from(samples_per_packet);
                next_sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index, ..
            }) => {
                boundaries.push(first_sample_index);
            }
            Some(FfiPlaylistFrame::End) => {
                reached_end = true;
                break;
            }
            None => {}
        }
    }
    assert!(reached_end, "playlist must finish, not silently time out");
    assert_eq!(boundaries, [480, 960], "two exact canonical boundaries");
    assert_eq!(audio_frames, 1_440);

    let mut returned = Vec::new();
    for _ in 0..3 {
        let mut pcm = vec![0_f32; TRACK_FRAMES * CHANNELS];
        let mut from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            usize::try_from(token).expect("token") as *mut core::ffi::c_void,
            pcm.as_mut_ptr(),
            u32::try_from(TRACK_FRAMES).expect("frames"),
            2,
            &raw mut from_ring,
        );
        assert_eq!(status, 0, "native audio callback must not underrun");
        assert_eq!(from_ring, 480);
        returned.extend(pcm);
    }
    let expected: Vec<f32> = values
        .iter()
        .flat_map(|value| std::iter::repeat_n(f32::from(*value) / 32768.0, TRACK_FRAMES * CHANNELS))
        .collect();
    assert_eq!(
        returned, expected,
        "the actual callback PCM must match source tracks"
    );
    assert_eq!(tap.drain(1_440), expected);
    assert_eq!(tap.counters(), (1_440, 0, 3));

    detach_callback_capture(u64::try_from(token).expect("token"));
    output.release();
    producer.cancel().expect("close worker");
    fs::remove_dir_all(&root).expect("clean fixture directory");
}

pub(super) fn authorized_pcm_socket_pair()
-> (Box<dyn HostTransportNode>, Box<dyn ListenerTransportNode>) {
    let session = SessionId::new("end-to-end-session").expect("session");
    let device = DeviceId::new("pcm-regression-listener").expect("device");
    let clock = Arc::new(ManualTransportClock::new(1_000));
    let factory = SocketTransportFactory;
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session.clone()),
            clock.clone(),
        )
        .expect("production socket host");
    let listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("production socket listener");
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session,
            device: DeviceIdentity {
                device_id: device.clone(),
                display_name: "PCM regression listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("join handshake");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = host
            .recv_event(deadline.saturating_duration_since(Instant::now()))
            .expect("host receives join");
        if matches!(
            event,
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
                ..
            }
        ) {
            break;
        }
    }
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize socket listener");
    loop {
        let event = host
            .recv_event(deadline.saturating_duration_since(Instant::now()))
            .expect("host observes authorization");
        if matches!(event, TransportEvent::PeerAuthorized { .. }) {
            break;
        }
    }
    (host, listener)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one real playlist, socket transport, scheduler, render ring and ABI regression"
)]
fn playlist_pcm_survives_listener_scheduler_and_native_audio_callback() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-scheduler-callback-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let values = [1_024_i16, 2_048, 4_096, 1_024];
    // Each source is exactly 10ms, but the decoder must normalize differing
    // sample rates and mono/stereo input before packetization and UDP delivery.
    let formats = [
        (48_000, 2, 480),
        (44_100, 1, 441),
        (96_000, 2, 960),
        (48_000, 2, 480),
    ];
    let sources: Vec<FfiPlaylistSource> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let path = root.join(format!("track-{}.wav", if index == 3 { 0 } else { index }));
            let (rate, channels, frames) = formats[index];
            fs::write(&path, source_wav_format(*value, rate, channels, frames))
                .expect("source fixture");
            FfiPlaylistSource {
                entry_id: format!("track-{index}"),
                display_name: format!("Track {index}"),
                staged_path: path.to_string_lossy().into_owned(),
            }
        })
        .collect();
    // A distinct queue entry deliberately reuses the first staged source.
    assert_eq!(sources[0].staged_path, sources[3].staged_path);
    assert_ne!(sources[0].entry_id, sources[3].entry_id);
    // Independent canonical decoder run provides the expected source PCM,
    // including the precise resampler output, not a guessed constant value.
    let mut reference_pcm = Vec::new();
    for index in 0..values.len() {
        let decoder = StreamingDecodeHandle::open(
            root.join(format!("track-{}.wav", if index == 3 { 0 } else { index })),
            StreamingDecodeConfig::default(),
        )
        .expect("open reference decoder");
        while let Ok(chunk) = decoder.recv() {
            reference_pcm.extend(chunk.frames);
        }
        decoder.join().expect("reference decoder completed");
    }
    assert_eq!(reference_pcm.len(), 1_920 * CHANNELS);
    let (mut host, mut listener) = authorized_pcm_socket_pair();
    let playlist = FfiHostPlaylistStreamer::open(
        "end-to-end-session".to_owned(),
        "end-to-end-stream".to_owned(),
        1_000,
        sources,
    )
    .expect("production playlist");
    let start = playlist.stream_start();
    // Exercise the production TCP control route as well as its UDP audio
    // route: the listener must observe the stream metadata before boundaries.
    let announced = host
        .broadcast_control(&ControlMessage::StreamStart(StreamStart {
            session_id: SessionId::new("end-to-end-session").expect("session"),
            stream_id: StreamId::new("end-to-end-stream").expect("stream"),
            host_start_time_ms: MonotonicMillis::new(start.host_start_time_ms),
            sample_rate: start.sample_rate,
            channels: start.channels,
            samples_per_packet: start.samples_per_packet,
            current_entry_id: start.current_entry_id.clone(),
            current_display_name: start.current_display_name.clone(),
        }))
        .expect("broadcast stream start");
    assert_eq!(announced.report.successful_peers, 1);
    let received_start = loop {
        let event = listener
            .recv_event(Duration::from_secs(5))
            .expect("listener receives stream start");
        if let TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::StreamStart(start)),
            ..
        } = event
        {
            break start;
        }
    };
    assert_eq!(received_start.stream_id.as_str(), "end-to-end-stream");
    assert_eq!(received_start.sample_rate, start.sample_rate);
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("end-to-end-session").expect("session"),
        StreamId::new("end-to-end-stream").expect("stream"),
        start.sample_rate,
        start.host_start_time_ms,
        start.samples_per_packet,
        start.channels,
    );
    scheduler_config.startup_buffer_target_ms = 0;
    // The production scheduler intentionally fades in the first real packet
    // from silence. This is an expected waveform transformation, not a lost
    // frame or a gap at a subsequent track boundary.
    let startup_ramp_frames = usize::try_from(
        u64::from(start.sample_rate) * u64::from(scheduler_config.concealment_ramp_ms) / 1_000,
    )
    .expect("ramp fits usize");
    let scheduler = PlaybackScheduler::new(scheduler_config, 0.0).expect("scheduler");
    let (render_producer, token) = register_render_ring(RenderRingConfig {
        capacity_frames: 4_800,
        target_fill_frames: 4_800,
    })
    .expect("native render ring");
    let mut pump = PlaybackPump::new(
        scheduler,
        render_producer,
        PlaybackPumpConfig {
            volume: 1.0,
            write_lead_ms: 0,
            max_prefill_ms: 0,
            target_depth_frames: 4_800,
        },
    )
    .expect("listener playback pump");
    pump.apply_sync_offset(0.0);
    let tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("capture tap"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach tap");

    let mut next_sample = 0;
    let mut next_sequence = 0;
    let mut boundaries = Vec::new();
    let mut finished = false;
    for _ in 0..64 {
        match playlist.next_frame(1_000).expect("host frame") {
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
                assert_eq!(sequence, next_sequence, "producer sequence continuity");
                assert_eq!(
                    first_sample_index, next_sample,
                    "producer sample continuity"
                );
                let sent = host
                    .broadcast_audio(&ProtocolFrame::Audio(AudioDatagram {
                        session_id: SessionId::new("end-to-end-session").expect("session"),
                        stream_id: StreamId::new("end-to-end-stream").expect("stream"),
                        sequence: PacketSequence::new(sequence),
                        codec: AudioCodec::PcmS16Le,
                        sample_rate,
                        channels,
                        samples_per_packet,
                        first_sample_index: SampleIndex::new(first_sample_index),
                        host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
                        payload,
                    }))
                    .expect("production UDP socket delivery");
                assert_eq!(sent.report.successful_peers, 1, "authorized listener");
                let received = loop {
                    let event = listener
                        .recv_event(Duration::from_secs(5))
                        .expect("production socket receives audio");
                    if let TransportEvent::FrameReceived {
                        channel: TransportChannel::Audio,
                        frame: ProtocolFrame::Audio(datagram),
                        ..
                    } = event
                    {
                        break datagram;
                    }
                };
                assert_eq!(received.sequence.get(), sequence, "socket packet order");
                pump.scheduler_mut()
                    .submit_packet(received)
                    .expect("listener accepts socket packet");
                next_sample += u64::from(samples_per_packet);
                next_sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index,
                entry_id,
                display_name,
                ..
            }) => {
                let sent = host
                    .broadcast_control(&ControlMessage::TrackBoundary(TrackBoundary {
                        session_id: SessionId::new("end-to-end-session").expect("session"),
                        stream_id: StreamId::new("end-to-end-stream").expect("stream"),
                        first_sample_index: SampleIndex::new(first_sample_index),
                        entry_id: entry_id.clone(),
                        display_name: display_name.clone(),
                    }))
                    .expect("broadcast real boundary on TCP");
                assert_eq!(sent.report.successful_peers, 1);
                let received = loop {
                    let event = listener
                        .recv_event(Duration::from_secs(5))
                        .expect("listener receives track boundary");
                    if let TransportEvent::FrameReceived {
                        frame: ProtocolFrame::Control(ControlMessage::TrackBoundary(boundary)),
                        ..
                    } = event
                    {
                        break boundary;
                    }
                };
                assert_eq!(received.first_sample_index.get(), first_sample_index);
                assert_eq!(received.entry_id, entry_id);
                assert_eq!(received.display_name, display_name);
                boundaries.push(received.first_sample_index.get());
            }
            Some(FfiPlaylistFrame::End) => {
                finished = true;
                break;
            }
            None => {}
        }
    }
    assert!(finished);
    assert_eq!(boundaries, [480, 960, 1_440]);
    assert_eq!(next_sample, 1_920);

    // Virtual presentation time, no wall-clock sleeps or hardware emulator:
    // the real scheduler/pump must queue all decoded producer packets.
    // Only advance through the four real 10 ms packets. Polling past the
    // natural queue end without a Stop control would legitimately make the
    // still-running scheduler conceal a hypothetical missing fifth packet.
    for now in 1_000..=1_039 {
        for _ in 0..64 {
            if !matches!(
                pump.tick(now),
                PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }
            ) {
                break;
            }
        }
    }
    assert_eq!(pump.diagnostics().prefill_frames, 0);
    assert_eq!(pump.diagnostics().packets_emitted, next_sequence);
    assert_eq!(pump.diagnostics().concealed_packets, 0);
    let mut from_ring_total = 0_u64;
    let mut captured = Vec::new();
    // Deliberately *do not* align callback buffers to 480-frame source
    // boundaries. Real Oboe callback sizes are not synchronized to a
    // playlist, and the second/third reads must each cross a track boundary.
    // 241 + 337 = 578 (> 480); 578 + 517 = 1_095 (> 960),
    // and the final callback crosses the reused source's 1,440-frame boundary.
    for callback_frames in [241_u32, 337, 517, 345, 480] {
        let mut returned =
            vec![0.0_f32; usize::try_from(callback_frames).expect("bounded callback") * CHANNELS];
        let mut frames_supplied = 0;
        let status = silent_disco_audio_read_interleaved_f32(
            usize::try_from(token).expect("token") as *mut core::ffi::c_void,
            returned.as_mut_ptr(),
            callback_frames,
            2,
            &raw mut frames_supplied,
        );
        assert_eq!(
            status, 0,
            "no gap from listener scheduler to native callback"
        );
        assert_eq!(frames_supplied, callback_frames, "native callback underrun");
        from_ring_total += u64::from(frames_supplied);
        captured.extend(returned);
    }
    assert_eq!(from_ring_total, 1_920);
    let engine = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    assert_eq!(silent_disco_audio_callback_count(engine), 5);
    assert_eq!(silent_disco_audio_frames_rendered(engine), 1_920);
    assert_eq!(silent_disco_audio_underrun_count(engine), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);
    let mut expected: Vec<f32> = reference_pcm
        .into_iter()
        .map(|sample| f32::from(sample) / 32768.0)
        .collect();
    // Exactly model the intentional startup fade from the real scheduler.
    // Later track boundaries must not acquire additional fades or silence.
    for frame in 0..startup_ramp_frames {
        let numerator = i32::from(values[0]) * i32::try_from(frame).expect("frame fits");
        let sample = numerator / i32::try_from(startup_ramp_frames).expect("ramp fits");
        let sample = f32::from(i16::try_from(sample).expect("sample fits")) / 32768.0;
        for channel in 0..CHANNELS {
            expected[frame * CHANNELS + channel] = sample;
        }
    }
    // Record failure evidence before asserting. Without this ordering a
    // regression emitted only a panic and CI uploaded no mismatch location.
    let first_mismatch = captured
        .iter()
        .zip(&expected)
        .position(|(actual, reference)| actual.to_bits() != reference.to_bits())
        .or_else(|| {
            (captured.len() != expected.len()).then_some(captured.len().min(expected.len()))
        });
    let mismatched_samples = captured
        .iter()
        .zip(&expected)
        .filter(|(actual, reference)| actual.to_bits() != reference.to_bits())
        .count()
        + captured.len().abs_diff(expected.len());
    let tap_pcm = tap.drain(1_920);
    let tap_mismatched_samples = tap_pcm
        .iter()
        .zip(&captured)
        .filter(|(tapped, returned)| tapped.to_bits() != returned.to_bits())
        .count()
        + tap_pcm.len().abs_diff(captured.len());
    let (tap_captured_frames, tap_lost_frames, tap_callbacks) = tap.counters();
    let passed = mismatched_samples == 0 && tap_mismatched_samples == 0 && tap_lost_frames == 0;
    // Hosted CI writes comparison *metadata* only, never source tracks or
    // returned raw audio; an exact output-SHA is supplied by the CI runner.
    if let Some(directory) = std::env::var_os("SILENT_DISCO_PCM_EVIDENCE_DIR") {
        let commit = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_owned());
        let path = std::path::PathBuf::from(directory);
        fs::create_dir_all(&path).expect("PCM evidence directory");
        let status = if passed { "PASS" } else { "FAIL" };
        let first_frame =
            first_mismatch.map_or("null".to_owned(), |sample| (sample / CHANNELS).to_string());
        let first_channel =
            first_mismatch.map_or("null".to_owned(), |sample| (sample % CHANNELS).to_string());
        let callback_count = silent_disco_audio_callback_count(engine);
        let native_underruns = silent_disco_audio_underrun_count(engine);
        let native_silence = silent_disco_audio_silence_filled_frames(engine);
        let captured_frames = captured.len() / CHANNELS;
        let json = format!(
            "{{\n  \"schema\": 1,\n  \"status\": \"{status}\",\n  \"commit\": \"{commit}\",\n  \"capture\": \"native-rust-callback-after-real-udp-socket\",\n  \"source_formats\": [\"48000-stereo\", \"44100-mono\", \"96000-stereo\", \"48000-stereo-reused\"],\n  \"reused_source_entry_ids\": [\"track-0\", \"track-3\"],\n  \"source_boundaries\": [480, 960, 1440],\n  \"sample_rate\": 48000,\n  \"channels\": 2,\n  \"expected_frames\": 1920,\n  \"captured_frames\": {captured_frames},\n  \"callback_count\": {callback_count},\n  \"native_underrun_callbacks\": {native_underruns},\n  \"native_silence_filled_frames\": {native_silence},\n  \"callback_frame_chunks\": [241, 337, 517, 345, 480],\n  \"tap_captured_frames\": {tap_captured_frames},\n  \"tap_lost_frames\": {tap_lost_frames},\n  \"tap_callback_count\": {tap_callbacks},\n  \"tap_mismatched_samples\": {tap_mismatched_samples},\n  \"mismatched_samples\": {mismatched_samples},\n  \"first_mismatch_frame\": {first_frame},\n  \"first_mismatch_channel\": {first_channel},\n  \"physical_dac_verified\": false\n}}\n"
        );
        fs::write(path.join("mixed-rate-socket-callback.json"), json)
            .expect("write machine-readable comparison evidence");
    }
    assert_eq!(captured.len(), expected.len(), "output sample count");
    if let Some(index) = first_mismatch {
        panic!(
            "listener callback PCM mismatch at frame {} channel {}; boundaries {boundaries:?}",
            index / CHANNELS,
            index % CHANNELS
        );
    }
    assert_eq!(tap_pcm, captured, "callback tap differs from returned PCM");
    assert_eq!(tap.counters(), (1_920, 0, 5));

    // The packetizer may leave intentional final-packet zero padding in the
    // render ring after all 1,920 source frames. Drain that padding and then
    // request 17 additional frames to force an observable native underrun.
    // The tap must record both the ring's padding and synthesized silence.
    let padding_frames = silent_disco_audio_available_frames(engine);
    let requested_tail = padding_frames.checked_add(17).expect("bounded padding");
    let mut final_tail =
        vec![f32::NAN; usize::try_from(requested_tail).expect("frames") * CHANNELS];
    let mut tail_from_ring = u32::MAX;
    let tail_status = silent_disco_audio_read_interleaved_f32(
        usize::try_from(token).expect("token") as *mut core::ffi::c_void,
        final_tail.as_mut_ptr(),
        requested_tail,
        2,
        &raw mut tail_from_ring,
    );
    assert_eq!(tail_status, 1, "end-of-stream underrun must be observable");
    assert_eq!(tail_from_ring, padding_frames);
    assert!(final_tail.iter().all(|sample| sample.to_bits() == 0));
    assert_eq!(
        tap.drain(usize::try_from(requested_tail).expect("frames")),
        final_tail
    );
    assert_eq!(tap.counters(), (1_920 + u64::from(requested_tail), 0, 6));
    assert_eq!(silent_disco_audio_callback_count(engine), 6);
    assert_eq!(
        silent_disco_audio_frames_rendered(engine),
        1_920 + u64::from(padding_frames)
    );
    assert_eq!(silent_disco_audio_underrun_count(engine), 1);
    assert_eq!(silent_disco_audio_silence_filled_frames(engine), 17);

    detach_callback_capture(token);
    release_render_ring(token).expect("native ring release");
    playlist.cancel().expect("playlist cleanup");
    listener.shutdown().expect("listener socket shutdown");
    host.shutdown().expect("host socket shutdown");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
