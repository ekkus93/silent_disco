//! End-to-end software PCM continuity: real host source decoding and
//! packetization through the native callback return buffer. Transport and the
//! Android device mixer are separate outstanding acceptance gates.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_read_interleaved_f32,
};
use crate::audio_output::FfiAudioOutputHandle;
use crate::audio_abi::{
    register_render_ring, release_render_ring,
};
use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, PumpTick, RenderRingConfig,
    SchedulerConfig,
};
use silent_disco_core::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};
use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const TRACK_FRAMES: usize = 480;
const CHANNELS: usize = 2;

fn source_wav(sample: i16) -> Vec<u8> {
    let bytes = TRACK_FRAMES * CHANNELS * 2;
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
    data.extend_from_slice(&2_u16.to_le_bytes());
    data.extend_from_slice(&48_000_u32.to_le_bytes());
    data.extend_from_slice(&192_000_u32.to_le_bytes());
    data.extend_from_slice(&4_u16.to_le_bytes());
    data.extend_from_slice(&16_u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&u32::try_from(bytes).expect("fixture").to_le_bytes());
    for _ in 0..TRACK_FRAMES * CHANNELS {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    data
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

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one real playlist, scheduler, render ring and ABI continuity regression"
)]
fn playlist_pcm_survives_listener_scheduler_and_native_audio_callback() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-scheduler-callback-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let values = [1_024_i16, 2_048, 4_096];
    let sources = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let path = root.join(format!("track-{index}.wav"));
            fs::write(&path, source_wav(*value)).expect("source fixture");
            FfiPlaylistSource {
                entry_id: format!("track-{index}"),
                display_name: format!("Track {index}"),
                staged_path: path.to_string_lossy().into_owned(),
            }
        })
        .collect();
    let playlist = FfiHostPlaylistStreamer::open(
        "end-to-end-session".to_owned(),
        "end-to-end-stream".to_owned(),
        1_000,
        sources,
    )
    .expect("production playlist");
    let start = playlist.stream_start();
    let mut scheduler_config = SchedulerConfig::new(
        SessionId::new("end-to-end-session").expect("session"),
        StreamId::new("end-to-end-stream").expect("stream"),
        start.sample_rate,
        start.host_start_time_ms,
        start.samples_per_packet,
        start.channels,
    );
    scheduler_config.startup_buffer_target_ms = 0;
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
                assert_eq!(first_sample_index, next_sample, "producer sample continuity");
                pump.scheduler_mut()
                    .submit_packet(AudioDatagram {
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
                    })
                    .expect("listener accepts production packet");
                next_sample += u64::from(samples_per_packet);
                next_sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                first_sample_index, ..
            }) => boundaries.push(first_sample_index),
            Some(FfiPlaylistFrame::End) => {
                finished = true;
                break;
            }
            None => {}
        }
    }
    assert!(finished);
    assert_eq!(boundaries, [480, 960]);
    assert_eq!(next_sample, 1_440);

    // Virtual presentation time, no wall-clock sleeps or hardware emulator:
    // the real scheduler/pump must queue all decoded producer packets.
    for now in 1_000..=1_040 {
        for _ in 0..64 {
            if !matches!(pump.tick(now), PumpTick::Queued { .. } | PumpTick::FlushedPending { .. }) {
                break;
            }
        }
    }
    assert_eq!(pump.diagnostics().prefill_frames, 0);
    assert_eq!(pump.diagnostics().packets_emitted, next_sequence);
    let mut from_ring_total = 0_u64;
    let mut captured = Vec::new();
    for _ in 0..3 {
        let mut returned = vec![0.0_f32; TRACK_FRAMES * CHANNELS];
        let mut frames_supplied = 0;
        let status = silent_disco_audio_read_interleaved_f32(
            usize::try_from(token).expect("token") as *mut core::ffi::c_void,
            returned.as_mut_ptr(),
            u32::try_from(TRACK_FRAMES).expect("track frames"),
            2,
            &raw mut frames_supplied,
        );
        assert_eq!(status, 0, "no gap from listener scheduler to native callback");
        from_ring_total += u64::from(frames_supplied);
        captured.extend(returned);
    }
    assert_eq!(from_ring_total, 1_440);
    let expected: Vec<f32> = values
        .iter()
        .flat_map(|value| std::iter::repeat_n(f32::from(*value) / 32768.0, TRACK_FRAMES * CHANNELS))
        .collect();
    assert_eq!(captured, expected, "track data changed in listener scheduler");
    assert_eq!(tap.drain(1_440), expected, "callback tap differs from returned PCM");
    assert_eq!(tap.counters(), (1_440, 0, 3));
    detach_callback_capture(token);
    release_render_ring(token).expect("native ring release");
    playlist.cancel().expect("playlist cleanup");
    fs::remove_dir_all(root).expect("fixture cleanup");
}
