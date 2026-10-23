//! FFI boundary reservation must fail closed before exposing successor PCM.
use super::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
    silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
};
use crate::audio_output::FfiAudioOutputHandle;
use crate::callback_capture::CallbackCaptureTap;
use silent_disco_core::audio::{PacketizerWorkerError, PacketizerWorkerErrorKind};
use silent_disco_core::protocol::TrackBoundary;
use std::fs;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{SystemTime, UNIX_EPOCH};

fn wav_pcm16(frames: u32, value: i16) -> Vec<u8> {
    let data_bytes = frames * 4;
    let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&192_000_u32.to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for _ in 0..frames * 2 {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn rejected_ffi_boundary_cannot_leak_successor_pcm_or_metadata() {
    let _guard = registry_test_guard();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-rejected-boundary-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("current.wav");
    let rejected = root.join("rejected.wav");
    fs::write(&first, wav_pcm16(480, 3_000)).expect("current fixture");
    fs::write(&rejected, wav_pcm16(480, 6_000)).expect("successor fixture");

    let reserve = Arc::new(|boundary: &TrackBoundary, _: Arc<AtomicBool>| {
        assert_eq!(boundary.entry_id, "removed");
        assert_eq!(boundary.first_sample_index.get(), 480);
        Err(PacketizerWorkerError {
            kind: PacketizerWorkerErrorKind::Packetize,
            message: "stale successor reservation rejected".to_owned(),
        })
    });
    let source = |entry_id: &str, path: &std::path::Path| FfiPlaylistSource {
        entry_id: entry_id.to_owned(),
        display_name: entry_id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    };
    let stream = FfiHostPlaylistStreamer::open_with_reserver(
        "session-rejection".to_owned(),
        "stream-rejection".to_owned(),
        5_000,
        vec![source("current", &first), source("removed", &rejected)],
        Some(reserve),
    )
    .expect("reserved playlist opens");

    // Keep the actual native C-ABI output open while a producer reservation
    // is rejected. Neither a rejected boundary nor its successor's samples
    // may escape through the audio callback.
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("native output ring");
    let token = u64::try_from(output.engine_token().expect("native token"))
        .expect("nonnegative token");
    let tap = Arc::new(CallbackCaptureTap::new(4_800).expect("bounded callback tap"));
    attach_callback_capture(token, Arc::clone(&tap)).expect("attach callback tap");
    let mut next_sample_index = 0_u64;
    let mut current_packets = 0;
    let failure = loop {
        match stream.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio {
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            })) => {
                assert_eq!(
                    first_sample_index, next_sample_index,
                    "rejected successor must not hide an audio gap"
                );
                assert!(first_sample_index < 480, "rejected successor PCM escaped");
                let samples: Vec<f32> = payload
                    .chunks_exact(2)
                    .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                    .collect();
                assert_eq!(
                    output.push_frames(samples).expect("queue valid current PCM"),
                    samples_per_packet
                );
                next_sample_index += u64::from(samples_per_packet);
                current_packets += 1;
            }
            Ok(Some(FfiPlaylistFrame::TrackBoundary { .. })) => {
                panic!("rejected reservation published successor metadata")
            }
            Ok(Some(FfiPlaylistFrame::End)) => {
                panic!("rejected reservation must report a failure, not clean EOF")
            }
            Ok(None) => panic!("rejected reservation stalled"),
            Err(error) => break error,
        }
    };
    assert!(current_packets > 0, "current source must remain audible");
    assert!(
        failure
            .to_string()
            .contains("stale successor reservation rejected")
    );
    assert_eq!(next_sample_index, 480, "current track was truncated");
    assert_eq!(output.queued_frames().expect("valid queued PCM"), 480);
    let engine = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    let mut captured = vec![f32::NAN; 480 * 2];
    let mut supplied = u32::MAX;
    let status = silent_disco_audio_read_interleaved_f32(
        usize::try_from(token).expect("token") as *mut core::ffi::c_void,
        captured.as_mut_ptr(),
        480,
        2,
        &raw mut supplied,
    );
    assert_eq!(status, 0, "entire current track must reach native callback");
    assert_eq!(supplied, 480);
    let expected_sample = f32::from(3_000_i16) / 32768.0;
    let mismatch = captured
        .iter()
        .position(|sample| sample.to_bits() != expected_sample.to_bits());
    let tapped = tap.drain(480);
    let capture_mismatch = tapped
        .iter()
        .zip(&captured)
        .position(|(tapped, returned)| tapped.to_bits() != returned.to_bits());
    assert_eq!(output.queued_frames().expect("drained output"), 0);

    // A callback *after* the valid track ends is allowed to synthesize
    // silence, but must report it as missing output, not false gaplessness.
    let mut missing_tail = [f32::NAN; 17 * 2];
    let mut tail_from_ring = u32::MAX;
    let tail_status = silent_disco_audio_read_interleaved_f32(
        usize::try_from(token).expect("token") as *mut core::ffi::c_void,
        missing_tail.as_mut_ptr(),
        17,
        2,
        &raw mut tail_from_ring,
    );
    assert_eq!(tail_status, 1, "output starvation must be observable");
    assert_eq!(tail_from_ring, 0);
    assert!(missing_tail.iter().all(|sample| sample.to_bits() == 0));
    assert_eq!(tap.drain(17), missing_tail);
    let (tap_frames, tap_lost, tap_callbacks) = tap.counters();
    let native_callbacks = silent_disco_audio_callback_count(engine);
    let native_underruns = silent_disco_audio_underrun_count(engine);
    let native_silence = silent_disco_audio_silence_filled_frames(engine);
    let passed = mismatch.is_none()
        && capture_mismatch.is_none()
        && tapped.len() == captured.len()
        && tap_frames == 497
        && tap_lost == 0
        && tap_callbacks == 2
        && native_callbacks == 2
        && native_underruns == 1
        && native_silence == 17;

    // Capture only reproducible failure-location metadata in public CI:
    // never upload the generated samples or source audio.
    if let Some(directory) = std::env::var_os("SILENT_DISCO_PCM_EVIDENCE_DIR") {
        let path = std::path::PathBuf::from(directory);
        fs::create_dir_all(&path).expect("PCM evidence directory");
        let commit = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_owned());
        let status = if passed { "PASS" } else { "FAIL" };
        let first_mismatch_frame =
            mismatch.map_or("null".to_owned(), |sample| (sample / 2).to_string());
        let first_mismatch_channel =
            mismatch.map_or("null".to_owned(), |sample| (sample % 2).to_string());
        let capture_mismatch_frame =
            capture_mismatch.map_or("null".to_owned(), |sample| (sample / 2).to_string());
        let report = format!(
            "{{\n  \"schema\": 1,\n  \"status\": \"{status}\",\n  \"commit\": \"{commit}\",\n  \"capture\": \"native-callback-after-rejected-production-reservation\",\n  \"rejected_boundary_frame\": 480,\n  \"valid_source_frames\": 480,\n  \"captured_valid_frames\": {},\n  \"first_mismatch_frame\": {first_mismatch_frame},\n  \"first_mismatch_channel\": {first_mismatch_channel},\n  \"tap_first_mismatch_frame\": {capture_mismatch_frame},\n  \"native_callback_count\": {native_callbacks},\n  \"native_underrun_callbacks\": {native_underruns},\n  \"native_silence_filled_frames\": {native_silence},\n  \"tap_captured_frames\": {tap_frames},\n  \"tap_lost_frames\": {tap_lost},\n  \"tap_callback_count\": {tap_callbacks},\n  \"expected_tail_silence_frames\": 17,\n  \"rejected_successor_audio_leaked\": false,\n  \"physical_dac_verified\": false\n}}\n",
            captured.len() / 2
        );
        fs::write(path.join("rejected-boundary-callback.json"), report)
            .expect("write rejection callback evidence");
    }
    assert!(passed, "native failure-path callback corruption at sample {mismatch:?}");
    detach_callback_capture(token);
    output.release();
    stream.cancel().expect("close failed stream");
    fs::remove_dir_all(root).expect("fixture cleanup");
}

#[test]
fn accepted_ffi_reservation_keeps_contiguous_audio() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-accepted-boundary-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let first = root.join("current.wav");
    let second = root.join("successor.wav");
    fs::write(&first, wav_pcm16(480, 3_000)).expect("current fixture");
    fs::write(&second, wav_pcm16(480, 6_000)).expect("successor fixture");

    let reservations = Arc::new(AtomicUsize::new(0));
    let callback_count = Arc::clone(&reservations);
    let reserve = Arc::new(move |boundary: &TrackBoundary, _: Arc<AtomicBool>| {
        assert_eq!(boundary.entry_id, "successor");
        assert_eq!(boundary.first_sample_index.get(), 480);
        callback_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    let source = |entry_id: &str, path: &std::path::Path| FfiPlaylistSource {
        entry_id: entry_id.to_owned(),
        display_name: entry_id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    };
    let stream = FfiHostPlaylistStreamer::open_with_reserver(
        "session-accepted".to_owned(),
        "stream-accepted".to_owned(),
        5_000,
        vec![source("current", &first), source("successor", &second)],
        Some(reserve),
    )
    .expect("reserved playlist opens");

    let mut next_sample = 0_u64;
    let mut last_sequence = None;
    let mut audio_count = 0;
    let mut saw_boundary = false;
    loop {
        match stream.next_frame(2_000).expect("next accepted frame") {
            Some(FfiPlaylistFrame::Audio {
                sequence,
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            }) => {
                assert_eq!(
                    first_sample_index, next_sample,
                    "inserted or overlapping PCM"
                );
                assert_eq!(
                    i16::from_le_bytes([payload[0], payload[1]]),
                    if saw_boundary { 6_000 } else { 3_000 }
                );
                if let Some(previous) = last_sequence {
                    assert_eq!(
                        sequence,
                        previous + 1,
                        "packet sequence must remain continuous"
                    );
                }
                last_sequence = Some(sequence);
                next_sample += u64::from(samples_per_packet);
                audio_count += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            }) => {
                assert!(!saw_boundary, "unexpected duplicate track boundary");
                assert_eq!(entry_id, "successor");
                assert_eq!(first_sample_index, next_sample);
                assert_eq!(first_sample_index, 480);
                saw_boundary = true;
            }
            Some(FfiPlaylistFrame::End) => break,
            None => panic!("accepted playlist stalled"),
        }
    }
    assert!(saw_boundary, "reserved transition must reach the listener");
    assert_eq!(next_sample, 960);
    assert_eq!(audio_count, 4);
    assert_eq!(reservations.load(Ordering::SeqCst), 1);
    fs::remove_dir_all(root).expect("fixture cleanup");
}

fn three_track_pcm_fixtures() -> (std::path::PathBuf, [std::path::PathBuf; 3]) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-second-rejection-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let paths = [
        root.join("first.wav"),
        root.join("second.wav"),
        root.join("rejected.wav"),
    ];
    for (path, sample) in paths.iter().zip([3_000, 6_000, 9_000]) {
        fs::write(path, wav_pcm16(480, sample)).expect("PCM fixture");
    }
    (root, paths)
}

#[test]
fn second_rejected_reservation_never_exposes_third_track_after_a_valid_boundary() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (root, [first, second, rejected]) = three_track_pcm_fixtures();

    let reservation_count = Arc::new(AtomicUsize::new(0));
    let callbacks = Arc::clone(&reservation_count);
    let reserve = Arc::new(move |boundary: &TrackBoundary, _: Arc<AtomicBool>| {
        assert_eq!(boundary.stream_id.to_string(), "stream-second-rejection");
        match callbacks.fetch_add(1, Ordering::SeqCst) {
            0 => {
                assert_eq!(boundary.entry_id, "second");
                assert_eq!(boundary.first_sample_index.get(), 480);
                Ok(())
            }
            1 => {
                assert_eq!(boundary.entry_id, "rejected");
                assert_eq!(boundary.first_sample_index.get(), 960);
                Err(PacketizerWorkerError {
                    kind: PacketizerWorkerErrorKind::Packetize,
                    message: "third-track reservation rejected".to_owned(),
                })
            }
            _ => panic!("producer retried a rejected reservation"),
        }
    });
    let source = |entry_id: &str, path: &std::path::Path| FfiPlaylistSource {
        entry_id: entry_id.to_owned(),
        display_name: entry_id.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    };
    let stream = FfiHostPlaylistStreamer::open_with_reserver(
        "session-second-rejection".to_owned(),
        "stream-second-rejection".to_owned(),
        5_000,
        vec![
            source("first", &first),
            source("second", &second),
            source("rejected", &rejected),
        ],
        Some(reserve),
    )
    .expect("reserved playlist opens");

    let mut next_sample = 0_u64;
    let mut last_sequence = None;
    let mut saw_second = false;
    let failure = loop {
        match stream.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio {
                sequence,
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            })) => {
                assert_eq!(first_sample_index, next_sample, "PCM discontinuity");
                assert!(first_sample_index < 960, "rejected successor PCM escaped");
                assert_eq!(
                    i16::from_le_bytes([payload[0], payload[1]]),
                    if saw_second { 6_000 } else { 3_000 }
                );
                if let Some(previous) = last_sequence {
                    assert_eq!(sequence, previous + 1, "sequence discontinuity");
                }
                last_sequence = Some(sequence);
                next_sample += u64::from(samples_per_packet);
            }
            Ok(Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            })) => {
                assert!(!saw_second, "rejected third boundary escaped");
                assert_eq!(entry_id, "second");
                assert_eq!(first_sample_index, 480);
                assert_eq!(first_sample_index, next_sample);
                saw_second = true;
            }
            Ok(Some(FfiPlaylistFrame::End)) => {
                panic!("rejected reservation must not report clean queue end")
            }
            Ok(None) => panic!("second rejected reservation stalled"),
            Err(error) => break error,
        }
    };
    assert!(saw_second, "the first reserved boundary must be delivered");
    assert_eq!(next_sample, 960, "both accepted tracks must finish");
    assert_eq!(last_sequence, Some(3));
    assert_eq!(reservation_count.load(Ordering::SeqCst), 2);
    assert!(
        failure
            .to_string()
            .contains("third-track reservation rejected")
    );
    fs::remove_dir_all(root).expect("fixture cleanup");
}
