//! Offline test harness for the **native callback return buffer** rather than
//! the existing pre-render-ring debug tap. File I/O happens after the callback
//! returns, never within the real-time audio function.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_read_interleaved_f32,
};
use crate::audio_output::FfiAudioOutputHandle;
use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};
use crate::listener_playback::{FfiListenerPlaybackConfig, FfiListenerPlaybackHandle};
use silent_disco_core::audio::DebugPcmRecorder;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;

const RATE: u32 = 48_000;
const TRACK_FRAMES: u32 = 240;
const CHANNELS: usize = 2;

fn three_track_pcm() -> Vec<i16> {
    (0..3)
        .flat_map(|track| {
            (0..TRACK_FRAMES).flat_map(move |frame| {
                [
                    i16::try_from(1_000 + track * 300 + frame).expect("bounded fixture"),
                    i16::try_from(2_000 + track * 300 + frame).expect("bounded fixture"),
                ]
            })
        })
        .collect()
}

fn callback_output(token: i64, frames: u32) -> (i32, u32, Vec<i16>) {
    let mut samples = vec![0_f32; usize::try_from(frames).expect("fixture fits") * CHANNELS];
    let mut from_ring = 0;
    // The token is intentionally pointer-shaped but never dereferenced.
    let engine = usize::try_from(token).expect("positive token") as *mut c_void;
    let status = silent_disco_audio_read_interleaved_f32(
        engine,
        samples.as_mut_ptr(),
        frames,
        2,
        &raw mut from_ring,
    );
    // Fixture values are positive i16s with an exact 1/32768 representation
    // in f32, so the inverse is exact. Never use this narrowing conversion
    // with unknown volume, clipping, or arbitrary signal processing.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "fixture samples are bounded and exactly representable as i16"
    )]
    let pcm = samples
        .into_iter()
        .map(|sample| (sample * 32768.0) as i16)
        .collect();
    (status, from_ring, pcm)
}

fn capture_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "silent-disco-callback-{name}-{}-{:?}.wav",
        std::process::id(),
        std::thread::current().id()
    ))
}

#[test]
fn native_callback_capture_exactly_matches_three_track_source_pcm() {
    let _guard = registry_test_guard();
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("register callback ring");
    let token = output.engine_token().expect("engine token");
    let expected = three_track_pcm();
    let source = expected
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(
        output.push_frames(source).expect("push canonical source"),
        3 * TRACK_FRAMES
    );

    let path = capture_path("gapless");
    let mut recording = DebugPcmRecorder::create(&path, RATE, 2).expect("create capture");
    for _ in 0..3 {
        let (status, frames_read, samples) = callback_output(token, TRACK_FRAMES);
        assert_eq!(status, 0, "unexpected native callback underrun");
        assert_eq!(frames_read, TRACK_FRAMES);
        recording
            .append(&samples)
            .expect("record returned callback PCM");
    }
    recording.finish(RATE).expect("finalize callback WAV");
    assert_eq!(output.queued_frames().expect("ring readable"), 0);
    let captured = fs::read(&path).expect("read WAV");
    assert_eq!(&captured[..4], b"RIFF");
    let reference_bytes: Vec<u8> = expected
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    assert_eq!(
        &captured[44..],
        reference_bytes,
        "native callback returned a gap, duplication, or altered PCM sample"
    );
    fs::remove_file(path).expect("remove fixture capture");
    output.release();
}

#[test]
fn native_callback_capture_exposes_silence_injected_at_track_boundary() {
    let _guard = registry_test_guard();
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("register callback ring");
    let token = output.engine_token().expect("engine token");
    let samples = three_track_pcm();
    let first_track = samples[..(TRACK_FRAMES as usize * CHANNELS)]
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(
        output.push_frames(first_track).expect("first track"),
        TRACK_FRAMES
    );
    let (status, read, first) = callback_output(token, TRACK_FRAMES);
    assert_eq!(status, 0);
    assert_eq!(read, TRACK_FRAMES);

    // Intentionally request the next callback before the producer writes
    // the second song: the actual ABI must zero-fill and signal PARTIAL.
    let (status, read, gap) = callback_output(token, 1);
    assert_eq!(status, 1, "underrun must be visible");
    assert_eq!(read, 0);
    assert_eq!(gap, [0_i16, 0_i16]);
    let rest = samples[(TRACK_FRAMES as usize * CHANNELS)..]
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(
        output.push_frames(rest).expect("remaining tracks"),
        2 * TRACK_FRAMES
    );
    let (status, read, final_pcm) = callback_output(token, 2 * TRACK_FRAMES);
    assert_eq!(status, 0);
    assert_eq!(read, 2 * TRACK_FRAMES);

    let mut capture = first;
    capture.extend(gap);
    capture.extend(final_pcm);
    assert_eq!(capture.len(), samples.len() + CHANNELS);
    assert_eq!(
        capture[..TRACK_FRAMES as usize * CHANNELS],
        samples[..TRACK_FRAMES as usize * CHANNELS]
    );
    assert_ne!(capture, samples, "a one-frame dropout must be detectable");
    output.release();
}

#[test]
fn attached_native_callback_tap_captures_underruns_and_reports_overflow() {
    let _guard = registry_test_guard();
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("register output");
    let token = output.engine_token().expect("engine token");
    let tap =
        std::sync::Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("bounded tap"));
    attach_callback_capture(
        u64::try_from(token).expect("positive token"),
        std::sync::Arc::clone(&tap),
    )
    .expect("attach exactly one capture");

    let expected = three_track_pcm();
    let frames = expected
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(
        output.push_frames(frames).expect("send three tracks"),
        3 * TRACK_FRAMES
    );
    let (status, supplied, heard) = callback_output(token, 3 * TRACK_FRAMES);
    assert_eq!(status, 0);
    assert_eq!(supplied, 3 * TRACK_FRAMES);
    assert_eq!(heard, expected);
    assert_eq!(tap.counters(), (720, 0, 1));

    // The next frame did not exist in the producer ring. Capture the actual
    // zero returned by the C ABI, not only the pre-ring scheduler PCM.
    let (status, supplied, silence) = callback_output(token, 1);
    assert_eq!(status, 1);
    assert_eq!(supplied, 0);
    assert_eq!(silence, [0, 0]);
    let recorded = tap.drain(721);
    assert_eq!(recorded.len(), 721 * CHANNELS);
    let canonical: Vec<f32> = expected
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(&recorded[..canonical.len()], canonical);
    assert_eq!(&recorded[canonical.len()..], [0.0, 0.0]);
    assert_eq!(tap.counters(), (721, 0, 2));

    // Fill the fixed-size tap without draining and then request one more
    // callback. A genuine drop must increment telemetry rather than silently
    // presenting an incomplete WAV as full-fidelity evidence.
    for _ in 0..MIN_CAPTURE_FRAMES {
        let _ = callback_output(token, 1);
    }
    assert_eq!(tap.counters(), (5_521, 0, 4_802));
    let _ = callback_output(token, 1);
    assert_eq!(tap.counters(), (5_521, 1, 4_803));
    detach_callback_capture(u64::try_from(token).expect("token"));
    let before = tap.counters();
    let _ = callback_output(token, 1);
    assert_eq!(
        tap.counters(),
        before,
        "detached capture must remain stable"
    );
    output.release();
}

#[test]
fn partial_native_callback_underrun_is_recorded_with_exact_silence_tail() {
    let _guard = registry_test_guard();
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("register output ring");
    let token = output.engine_token().expect("engine token");
    let tap = std::sync::Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("tap"));
    attach_callback_capture(
        u64::try_from(token).expect("positive token"),
        std::sync::Arc::clone(&tap),
    )
    .expect("attach capture");

    // The render ring has only three frames when the real native callback
    // requests five. The tap must record both the three audible frames and
    // the two zero-filled frames actually returned to the audio device.
    let source = [1_024_i16, 2_048, 3_072, 4_096, 5_120, 6_144];
    let decoded = source
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(output.push_frames(decoded).expect("push source"), 3);
    let (status, from_ring, returned) = callback_output(token, 5);
    assert_eq!(status, 1, "partial callback must report underrun");
    assert_eq!(from_ring, 3);
    assert_eq!(
        returned,
        [1_024, 2_048, 3_072, 4_096, 5_120, 6_144, 0, 0, 0, 0]
    );
    assert_eq!(
        tap.drain(5),
        returned
            .iter()
            .map(|sample| f32::from(*sample) / 32768.0)
            .collect::<Vec<_>>(),
        "capture must contain the exact callback-returned silence tail"
    );
    assert_eq!(tap.counters(), (5, 0, 1), "no silent capture loss");
    let engine = usize::try_from(token).expect("token") as *const c_void;
    assert_eq!(
        crate::audio_abi::silent_disco_audio_callback_count(engine),
        1
    );
    assert_eq!(
        crate::audio_abi::silent_disco_audio_underrun_count(engine),
        1
    );
    assert_eq!(
        crate::audio_abi::silent_disco_audio_silence_filled_frames(engine),
        2
    );

    detach_callback_capture(u64::try_from(token).expect("token"));
    output.release();
}

#[test]
fn uniffi_callback_capture_is_opt_in_drains_tail_and_can_restart() {
    let _guard = registry_test_guard();
    let playback = FfiListenerPlaybackHandle::open(FfiListenerPlaybackConfig {
        session_id: "capture-lifecycle-session".to_owned(),
        stream_id: "capture-lifecycle-stream".to_owned(),
        sample_rate: RATE,
        host_start_time_ms: 1_000,
        samples_per_packet: TRACK_FRAMES,
        channels: 2,
        startup_buffer_target_ms: 0,
        rebuffer_target_ms: 0,
        ring_capacity_frames: 4_800,
        ring_target_fill_frames: 1,
        write_lead_ms: 0,
        max_prefill_ms: 0,
        volume: 1.0,
    })
    .expect("listener runtime");
    assert!(
        playback.read_callback_capture(1).is_err(),
        "capture defaults off"
    );
    assert!(
        playback.start_callback_capture(0).is_err(),
        "zero capacity rejected"
    );
    playback
        .start_callback_capture(u32::try_from(MIN_CAPTURE_FRAMES).expect("capacity"))
        .expect("explicit opt-in");
    assert!(
        playback
            .start_callback_capture(u32::try_from(MIN_CAPTURE_FRAMES).expect("capacity"))
            .is_err(),
        "duplicate tap must be rejected"
    );
    assert!(
        playback.read_callback_capture(0).is_err(),
        "zero read rejected"
    );

    let token = playback.engine_token();
    let (_, _, returned) = callback_output(token, 3);
    assert_eq!(returned, [0_i16; 6], "no source means native silence");
    let batch = playback
        .read_callback_capture(3)
        .expect("drain callback tail");
    assert_eq!(batch.samples, [0.0_f32; 6]);
    assert_eq!(batch.captured_frames, 3);
    assert_eq!(batch.lost_frames, 0);
    assert_eq!(batch.callback_count, 1);

    playback.end_callback_capture();
    let _ = callback_output(token, 1);
    let after_end = playback.read_callback_capture(3).expect("final drain");
    assert!(after_end.samples.is_empty());
    assert_eq!(after_end.captured_frames, 3, "detach freezes counters");
    assert_eq!(after_end.callback_count, 1);
    assert!(
        playback
            .start_callback_capture(u32::try_from(MIN_CAPTURE_FRAMES).expect("capacity"))
            .is_err(),
        "a detached tap remains reserved until explicitly cleared"
    );
    playback.clear_callback_capture();
    assert!(playback.read_callback_capture(1).is_err());
    playback
        .start_callback_capture(u32::try_from(MIN_CAPTURE_FRAMES).expect("capacity"))
        .expect("restart after full drain and clear");
    playback.clear_callback_capture();
    playback.stop().expect("stop listener runtime");
    assert!(
        playback
            .start_callback_capture(u32::try_from(MIN_CAPTURE_FRAMES).expect("capacity"))
            .is_err(),
        "stopped output must reject capture"
    );
}
