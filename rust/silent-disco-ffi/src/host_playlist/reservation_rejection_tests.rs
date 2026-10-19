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
