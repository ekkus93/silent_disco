//! Test the production playlist producer using actual encoded source fixtures.
//! Expected samples come from independent runs of the canonical Rust decoder;
//! protocol boundary and packet sequence continuity must survive codec changes.
use crate::audio_abi::{
    attach_callback_capture, detach_callback_capture, registry_test_guard,
    silent_disco_audio_callback_count, silent_disco_audio_read_interleaved_f32,
    silent_disco_audio_silence_filled_frames, silent_disco_audio_underrun_count,
};
use crate::audio_output::FfiAudioOutputHandle;
use crate::callback_capture::{CallbackCaptureTap, MIN_CAPTURE_FRAMES};
use crate::host_playlist::{FfiHostPlaylistStreamer, FfiPlaylistFrame, FfiPlaylistSource};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use silent_disco_core::audio::{StreamingDecodeConfig, StreamingDecodeHandle};
use std::fs;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one combined source decode, playlist packet sequence and boundary regression"
)]
fn encoded_flac_mp3_and_wav_playlist_matches_independent_canonical_pcm() {
    let _guard = registry_test_guard();
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
        (
            "wav",
            include_str!("../../silent-disco-core/src/audio/fixtures/short.wav.b64"),
        ),
        (
            "flac",
            include_str!("../../silent-disco-core/src/audio/fixtures/short.flac.b64"),
        ),
        (
            "mp3",
            include_str!("../../silent-disco-core/src/audio/fixtures/short.mp3.b64"),
        ),
    ];
    let mut canonical = Vec::<i16>::new();
    let mut expected_boundaries = Vec::new();
    let mut sources = Vec::new();

    for (index, (extension, contents)) in encoded_sources.iter().enumerate() {
        let path = root.join(format!("song-{index}.{extension}"));
        let bytes = STANDARD
            .decode(contents.trim())
            .expect("known decoder fixture");
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

    // Exercise the same native C ABI used by Android Oboe with the actual
    // encoded playlist output, not just the packetizer's decoded bytes.
    // The tap is drained after every callback, as a background writer would.
    let output = FfiAudioOutputHandle::open(4_800, 1).expect("native output ring");
    let token = output.engine_token().expect("output token");
    let tap = Arc::new(CallbackCaptureTap::new(MIN_CAPTURE_FRAMES).expect("tap"));
    attach_callback_capture(u64::try_from(token).expect("token"), Arc::clone(&tap))
        .expect("attach native capture");
    let mut callback_pcm = Vec::<f32>::new();
    let mut sequence = 0_u64;
    let mut next_sample = 0_u64;
    let mut actual = Vec::<i16>::new();
    let mut boundaries = Vec::new();
    let mut ended = false;
    // These bundled fixtures are intentionally small; an ordinary playlist
    // must not stall or silently drop an encoded successor.
    for _ in 0..1_024 {
        match playlist
            .next_frame(5_000)
            .expect("production playlist frame")
        {
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
                assert_eq!(
                    payload.len(),
                    usize::try_from(samples_per_packet).expect("frames") * 4
                );
                let packet_samples: Vec<i16> = payload
                    .chunks_exact(2)
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                actual.extend_from_slice(&packet_samples);
                let packet_pcm: Vec<f32> = packet_samples
                    .iter()
                    .map(|sample| f32::from(*sample) / 32768.0)
                    .collect();
                assert_eq!(
                    output
                        .push_frames(packet_pcm.clone())
                        .expect("native ring write"),
                    samples_per_packet
                );
                let mut returned = vec![f32::NAN; packet_pcm.len()];
                let mut supplied = 0_u32;
                let status = silent_disco_audio_read_interleaved_f32(
                    usize::try_from(token).expect("token") as *mut core::ffi::c_void,
                    returned.as_mut_ptr(),
                    samples_per_packet,
                    2,
                    &raw mut supplied,
                );
                assert_eq!(
                    status, 0,
                    "encoded boundary must not underrun native output"
                );
                assert_eq!(supplied, samples_per_packet);
                assert_eq!(returned, packet_pcm, "callback changed decoded samples");
                assert_eq!(
                    tap.drain(usize::try_from(samples_per_packet).expect("frames")),
                    returned,
                    "native tap must capture the callback-returned PCM"
                );
                callback_pcm.extend(returned);
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
    assert_eq!(
        boundaries, expected_boundaries,
        "exact decoded sample boundaries"
    );
    assert!(
        actual.len() >= canonical.len(),
        "must not drop decoded PCM tail"
    );
    assert_eq!(
        &actual[..canonical.len()],
        canonical,
        "codec switch altered PCM"
    );
    // The only permitted excess is the producer's final-packet padding, not
    // silence inserted at either track boundary.
    assert!(actual.len() - canonical.len() < 480);
    assert!(
        actual[canonical.len()..].iter().all(|sample| *sample == 0),
        "only final packet padding may add silence"
    );
    assert_eq!(
        callback_pcm.len(),
        actual.len(),
        "native callback frame count"
    );
    let canonical_pcm: Vec<f32> = canonical
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect();
    assert_eq!(
        &callback_pcm[..canonical_pcm.len()],
        canonical_pcm,
        "native callback must match the independent WAV/FLAC/MP3 decode"
    );
    assert!(
        callback_pcm[canonical.len()..]
            .iter()
            .all(|sample| sample.to_bits() == 0),
        "native callback may contain only final-packet padding"
    );
    let engine = usize::try_from(token).expect("token") as *const core::ffi::c_void;
    assert_eq!(silent_disco_audio_callback_count(engine), sequence);
    assert_eq!(silent_disco_audio_underrun_count(engine), 0);
    assert_eq!(silent_disco_audio_silence_filled_frames(engine), 0);
    assert_eq!(tap.counters(), (next_sample, 0, sequence));
    detach_callback_capture(u64::try_from(token).expect("token"));
    output.release();
    playlist.cancel().expect("close encoded playlist");
    fs::remove_dir_all(root).expect("remove generated sources");
}
