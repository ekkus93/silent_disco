//! FFI boundary reservation must fail closed before exposing successor PCM.
use super::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
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

    let mut current_packets = 0;
    let failure = loop {
        match stream.next_frame(2_000) {
            Ok(Some(FfiPlaylistFrame::Audio {
                first_sample_index, ..
            })) => {
                assert!(
                    first_sample_index < 480,
                    "successor PCM escaped a rejected reservation"
                );
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
                assert_eq!(first_sample_index, next_sample, "inserted or overlapping PCM");
                assert_eq!(
                    i16::from_le_bytes([payload[0], payload[1]]),
                    if saw_boundary { 6_000 } else { 3_000 }
                );
                if let Some(previous) = last_sequence {
                    assert_eq!(sequence, previous + 1, "packet sequence must remain continuous");
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
