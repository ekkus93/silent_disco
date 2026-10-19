//! Producer-side reservation must preserve a committed successor while retargeting later songs.
use super::{FfiHostPlaylistError, FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use silent_disco_core::audio::{PacketizerWorkerError, PacketizerWorkerErrorKind};
use silent_disco_core::protocol::TrackBoundary;
use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn pcm_wav(frames: u32, sample: i16) -> Vec<u8> {
    let bytes = frames * 4;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(bytes + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&48_000_u32.to_le_bytes());
    wav.extend_from_slice(&192_000_u32.to_le_bytes());
    wav.extend_from_slice(&4_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for _ in 0..frames * 2 {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one complete reserved producer/edit race with packet-level assertions"
)]
fn ffi_edit_behind_reserved_successor_keeps_its_boundary_and_retargets_later_audio() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sd-ffi-reserved-retarget-{unique}"));
    fs::create_dir_all(&root).expect("fixture directory");
    let paths = [
        root.join("current.wav"),
        root.join("reserved.wav"),
        root.join("obsolete.wav"),
        root.join("replacement.wav"),
    ];
    for (path, sample) in paths.iter().zip([3_000, 6_000, 9_000, 12_000]) {
        fs::write(path, pcm_wav(480, sample)).expect("write PCM source");
    }
    let source = |name: &str, path: &std::path::Path| FfiPlaylistSource {
        entry_id: name.to_owned(),
        display_name: name.to_owned(),
        staged_path: path.to_string_lossy().into_owned(),
    };

    // Hold the producer at its first synchronous reservation, before it can
    // enqueue successor PCM; the consumer has not dequeued that boundary.
    let (entered_tx, entered_rx) = sync_channel(1);
    let (resume_tx, resume_rx) = sync_channel(1);
    let resume_rx = Mutex::new(resume_rx);
    let reservations = Arc::new(Mutex::new(Vec::<(String, u64)>::new()));
    let observed = Arc::clone(&reservations);
    let reserve = Arc::new(
        move |boundary: &TrackBoundary, cancelled: Arc<AtomicBool>| {
            observed
                .lock()
                .expect("reservation log")
                .push((boundary.entry_id.clone(), boundary.first_sample_index.get()));
            if boundary.entry_id == "reserved" {
                entered_tx.send(()).expect("signal first reservation");
                resume_rx
                    .lock()
                    .expect("resume lock")
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| PacketizerWorkerError {
                        kind: PacketizerWorkerErrorKind::Cancelled,
                        message: format!("test reservation gate not released: {error}"),
                    })?;
            }
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(PacketizerWorkerError {
                    kind: PacketizerWorkerErrorKind::Cancelled,
                    message: "test producer cancelled".to_owned(),
                });
            }
            Ok(())
        },
    );
    let stream = FfiHostPlaylistStreamer::open_with_reserver(
        "session-retarget".to_owned(),
        "stream-retarget".to_owned(),
        5_000,
        vec![
            source("current", &paths[0]),
            source("reserved", &paths[1]),
            source("obsolete", &paths[2]),
        ],
        Some(reserve),
    )
    .expect("open producer");
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("producer reached first reservation");

    // Preserve the already-chosen successor while replacing only the
    // uncommitted third track. The first boundary must never be duplicated.
    stream
        .replace_upcoming(vec![
            source("reserved", &paths[1]),
            source("replacement", &paths[3]),
        ])
        .expect("retarget later entry");
    // Once the actor has applied a reserved boundary, its snapshot omits
    // that now-current entry. Both an empty upcoming list and a later
    // replacement are valid: neither may remove the committed successor.
    stream
        .replace_upcoming(vec![])
        .expect("actor snapshot after boundary application");
    stream
        .replace_upcoming(vec![source("replacement", &paths[3])])
        .expect("restore the later replacement without the reserved shadow");
    // Invalid live edits must fail transactionally. Neither a duplicate nor
    // a non-leading reserved shadow may replace the accepted retarget.
    for invalid in [
        vec![source("reserved", &paths[1]), source("reserved", &paths[1])],
        vec![
            source("replacement", &paths[3]),
            source("reserved", &paths[1]),
        ],
    ] {
        assert!(
            matches!(
                stream.replace_upcoming(invalid),
                Err(FfiHostPlaylistError::Packetizer(_))
            ),
            "an invalid reserved-entry edit must be rejected"
        );
    }
    resume_tx.send(()).expect("release reservation");

    let mut next_sample = 0_u64;
    let mut last_sequence = None;
    let mut boundaries = Vec::new();
    loop {
        match stream.next_frame(5_000).expect("stream frame") {
            Some(FfiPlaylistFrame::Audio {
                sequence,
                first_sample_index,
                samples_per_packet,
                payload,
                ..
            }) => {
                assert_eq!(first_sample_index, next_sample, "PCM gap or overlap");
                let expected = if next_sample < 480 {
                    3_000
                } else if next_sample < 960 {
                    6_000
                } else {
                    12_000
                };
                assert_eq!(
                    i16::from_le_bytes([payload[0], payload[1]]),
                    expected,
                    "obsolete successor PCM reached the consumer"
                );
                if let Some(previous) = last_sequence {
                    assert_eq!(sequence, previous + 1, "sequence discontinuity");
                }
                last_sequence = Some(sequence);
                next_sample += u64::from(samples_per_packet);
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            }) => {
                assert_eq!(
                    first_sample_index, next_sample,
                    "boundary must match the audible sample clock"
                );
                boundaries.push((entry_id, first_sample_index));
            }
            Some(FfiPlaylistFrame::End) => break,
            None => panic!("producer stalled while retargeting later track"),
        }
    }
    assert_eq!(next_sample, 1_440);
    assert_eq!(last_sequence, Some(5));
    assert_eq!(
        boundaries,
        vec![
            ("reserved".to_owned(), 480),
            ("replacement".to_owned(), 960),
        ]
    );
    assert_eq!(*reservations.lock().expect("reservation log"), boundaries);
    fs::remove_dir_all(root).expect("remove fixtures");
}
