//! Test the production playlist producer using actual encoded source fixtures.
//! Expected samples come from independent runs of the canonical Rust decoder;
//! protocol boundary and packet sequence continuity must survive codec changes.
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use silent_disco_core::audio::{StreamingDecodeConfig, StreamingDecodeHandle};
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn encoded_flac_mp3_and_wav_playlist_matches_independent_canonical_pcm() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "silent-disco-compressed-playlist-{}-{unique}",
        std::process::id(),
    ));
    fs::create_dir_all(&root).expect("create fixtures directory");
    let encoded_sources = [
        ("wav", include_str!("../../silent-disco-core/src/audio/fixtures/short.wav.b64")),
        ("flac", include_str!("../../silent-disco-core/src/audio/fixtures/short.flac.b64")),
        ("mp3", include_str!("../../silent-disco-core/src/audio/fixtures/short.mp3.b64")),
    ];
    let mut canonical = Vec::<i16>::new();
    let mut expected_boundaries = Vec::new();
    let mut sources = Vec::new();

    for (index, (extension, contents)) in encoded_sources.iter().enumerate() {
        let path = root.join(format!("song-{index}.{extension}"));
        let bytes = STANDARD.decode(contents.trim()).expect("known decoder fixture");
        fs::write(&path, bytes).expect("stage encoded source");
        let decoder = StreamingDecodeHandle::open(&path, StreamingDecodeConfig::default())
            .expect("decode canonical PCM reference");
        loop {
            let chunk = decoder
                .recv_timeout(Duration::from_secs(5))
                .expect("reference decoder produces PCM");
            let at_end = chunk.end_of_stream;
            canonical.extend(chunk.frames);
            if at_end {
                break;
            }
        }
        decoder.join().expect("reference decoder completed");
        assert_eq!(canonical.len() % 2, 0, "interleaved canonical stereo");
        if index + 1 < encoded_sources.len() {
            expected_boundaries.push(u64::try_from(canonical.len() / 2).expect("bounded fixture"));
        }
        sources.push(FfiPlaylistSource {
            entry_id: format!("encoded-entry-{index}"),
            display_name: format!("Encoded {extension}"),
            staged_path: path.to_string_lossy().into_owned(),
        });
    }
    assert_eq!(expected_boundaries.len(), 2);
    let playlist = FfiHostPlaylistStreamer::open(
        "encoded-playlist-session".to_owned(),
        "encoded-playlist-stream".to_owned(),
        1_000,
        sources,
    )
    .expect("start production playlist");
    let start = playlist.stream_start();
    assert_eq!(start.sample_rate, 48_000);
    assert_eq!(start.channels, 2);

    let mut sequence = 0_u64;
    let mut next_sample = 0_u64;
    let mut actual = Vec::<i16>::new();
    let mut boundaries = Vec::new();
    let mut ended = false;
    // These bundled fixtures are intentionally small; an ordinary playlist
    // must not stall or silently drop an encoded successor.
    for _ in 0..1_024 {
        match playlist.next_frame(5_000).expect("production playlist frame") {
            Some(FfiPlaylistFrame::Audio {
                sequence: observed_sequence,
                first_sample_index,
                sample_rate,
                channels,
                samples_per_packet,
                payload,
                ..
            }) => {
                assert_eq!(observed_sequence, sequence, "packet sequence continuity");
                assert_eq!(first_sample_index, next_sample, "PCM index gap/overlap");
                assert_eq!(sample_rate, 48_000);
                assert_eq!(channels, 2);
                assert_eq!(payload.len(), usize::try_from(samples_per_packet).expect("frames") * 4);
                actual.extend(
                    payload.chunks_exact(2)
                        .map(|pair| i16::from_le_bytes([pair[0], pair[1]])),
                );
                next_sample += u64::from(samples_per_packet);
                sequence += 1;
            }
            Some(FfiPlaylistFrame::TrackBoundary {
                entry_id,
                first_sample_index,
                ..
            }) => {
                assert_eq!(first_sample_index, next_sample, "boundary ordering");
                assert_eq!(entry_id, format!("encoded-entry-{}", boundaries.len() + 1));
                boundaries.push(first_sample_index);
            }
            Some(FfiPlaylistFrame::End) => {
                ended = true;
                break;
            }
            None => {}
        }
    }
    assert!(ended, "production playlist must reach End");
    assert_eq!(boundaries, expected_boundaries, "exact decoded sample boundaries");
    assert!(actual.len() >= canonical.len(), "must not drop decoded PCM tail");
    assert_eq!(&actual[..canonical.len()], canonical, "codec switch altered PCM");
    // The only permitted excess is the producer's final-packet padding, not
    // silence inserted at either track boundary.
    assert!(actual.len() - canonical.len() < 480);
    assert!(
        actual[canonical.len()..].iter().all(|sample| *sample == 0),
        "only final packet padding may add silence"
    );
    playlist.cancel().expect("close encoded playlist");
    fs::remove_dir_all(root).expect("remove generated sources");
}
