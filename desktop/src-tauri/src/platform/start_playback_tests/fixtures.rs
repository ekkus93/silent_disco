//! Staged WAV source fixtures shared by the automated `start_playback` test
//! suite (`streaming_tests`, `lifecycle_tests`) -- not used by the manual
//! device tests, which generate their own melody fixtures in
//! `manual::melody`.

use crate::platform::file_picker::{AudioContainer, InspectedAudioSource, SelectedSourceRegistry};
use silent_disco_core::runtime::AudioSourceDescriptor;
use std::fs;
use tempfile::TempDir;

pub(super) fn stage_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    stage_wav_source(temp, "desktop-block-playback-source", pcm_wav())
}

/// Comfortably longer than [`SEND_AHEAD_HORIZON_MS`]-worth of playback (via
/// `playback_streamer::SEND_AHEAD_HORIZON_MS`, not directly importable from
/// this test module) so a mid-stream check (a sync request/response
/// round trip, an explicit `stop_playback()`) genuinely happens before
/// natural end-of-file, unlike the short `stage_source` fixture, which now
/// bursts out entirely and reaches natural EOF almost immediately.
pub(super) fn stage_long_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    stage_wav_source(temp, "desktop-block-playback-long-source", long_pcm_wav())
}

pub(super) fn stage_wav_source(
    temp: &TempDir,
    source_id: &str,
    wav_bytes: Vec<u8>,
) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    let source_path = temp.path().join("source.wav");
    fs::write(&source_path, wav_bytes).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(source_id, "source.wav", Some(byte_length), None)
        .expect("descriptor");
    let registry = SelectedSourceRegistry::new();
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register staged source");
    (descriptor, registry)
}

fn pcm_wav() -> Vec<u8> {
    square_wave_pcm_wav(4_410)
}

/// 3 real seconds -- comfortably longer than the playback pump's send-ahead
/// horizon, so playback is still genuinely running (not yet at natural
/// end-of-file) by the time a mid-stream check runs.
fn long_pcm_wav() -> Vec<u8> {
    square_wave_pcm_wav(44_100 * 3)
}

/// Truncated before the 44-byte WAV header is even complete, so the shared
/// decoder cannot parse it at all -- `StreamingDecodeHandle::open` fails
/// synchronously. See
/// `starting_playback_with_a_corrupt_source_fails_visibly_at_the_orchestration_level`.
pub(super) fn corrupt_wav_bytes() -> Vec<u8> {
    let mut bytes = pcm_wav();
    bytes.truncate(20);
    bytes
}

/// A WAV whose header parses fine and declares 3 real seconds of data (via
/// `long_pcm_wav`), but whose body is cut to only ~0.1s of actual bytes --
/// the header's declared `data` chunk size is left untouched, so the shared
/// decoder only discovers the shortfall once it reads past the truncation
/// point. Confirmed empirically (see `docs/AUDIO_PLAYBACK_STATE_2026-08-10.md`
/// D1 for context): `StreamingDecodeHandle::open` succeeds and the failure
/// surfaces later as `DecodeErrorKind::CorruptInput` while draining chunks --
/// exactly the "host source read failure" this fixture exists to trigger.
/// See `a_host_source_read_failure_mid_stream_does_not_claim_continued_normal_streaming`.
pub(super) fn truncated_body_full_header_wav() -> Vec<u8> {
    const WAV_HEADER_BYTES: usize = 44;
    const SURVIVING_DATA_BYTES: usize = 4_410 * 2; // ~0.1s at 44.1kHz mono 16-bit
    let mut bytes = long_pcm_wav();
    bytes.truncate(WAV_HEADER_BYTES + SURVIVING_DATA_BYTES);
    bytes
}

fn square_wave_pcm_wav(frame_count: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frame_count {
        let sample = if index % 64 < 32 {
            8_000_i16
        } else {
            -8_000_i16
        };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}
