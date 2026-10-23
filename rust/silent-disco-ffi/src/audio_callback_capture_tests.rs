//! Offline test harness for the **native callback return buffer** rather than
//! the existing pre-render-ring debug tap. File I/O happens after the callback
//! returns, never within the real-time audio function.
use crate::audio_abi::{registry_test_guard, silent_disco_audio_read_interleaved_f32};
use crate::audio_output::FfiAudioOutputHandle;
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
    let mut samples = vec![
        0_f32;
        usize::try_from(frames).expect("fixture fits") * CHANNELS
    ];
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
        recording.append(&samples).expect("record returned callback PCM");
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
    assert_eq!(output.push_frames(first_track).expect("first track"), TRACK_FRAMES);
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
    assert_eq!(output.push_frames(rest).expect("remaining tracks"), 2 * TRACK_FRAMES);
    let (status, read, final_pcm) = callback_output(token, 2 * TRACK_FRAMES);
    assert_eq!(status, 0);
    assert_eq!(read, 2 * TRACK_FRAMES);

    let mut capture = first;
    capture.extend(gap);
    capture.extend(final_pcm);
    assert_eq!(capture.len(), samples.len() + CHANNELS);
    assert_eq!(capture[..TRACK_FRAMES as usize * CHANNELS], samples[..TRACK_FRAMES as usize * CHANNELS]);
    assert_ne!(capture, samples, "a one-frame dropout must be detectable");
    output.release();
}
