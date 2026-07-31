use super::audio_decode::prepare_staged_audio_source;
use super::file_picker::{AudioContainer, InspectedAudioSource, SelectedSourceRegistry};
use silent_disco_core::audio::{AudioFormat, DecodeWorkerState, StreamingDecodeConfig};
use silent_disco_core::runtime::AudioSourceDescriptor;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn prepares_the_exact_registered_staged_source() {
    let temp = TempDir::new().expect("temp");
    let source_path = temp.path().join("source.wav");
    fs::write(&source_path, pcm_wav()).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(
        "desktop-staged-test-source",
        "source.wav",
        Some(byte_length),
        None,
    )
    .expect("descriptor");
    let registry = SelectedSourceRegistry::new();
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register");

    let prepared = prepare_staged_audio_source(
        &registry,
        &descriptor,
        StreamingDecodeConfig {
            chunk_frames: 127,
            queue_chunks: 2,
            ..StreamingDecodeConfig::default()
        },
    )
    .expect("prepare");
    assert_eq!(prepared.descriptor(), &descriptor);
    assert_eq!(
        prepared.decoder().stream_info().format,
        AudioFormat::CANONICAL
    );

    let decoder = prepared.into_decoder();
    let mut expected_index = 0_u64;
    loop {
        let chunk = decoder.recv_timeout(TEST_TIMEOUT).expect("chunk");
        assert_eq!(chunk.first_sample_index.get(), expected_index);
        expected_index += u64::try_from(chunk.frame_count()).expect("frames");
        if chunk.end_of_stream {
            break;
        }
    }
    let summary = decoder.join().expect("join");
    assert_eq!(summary.state, DecodeWorkerState::Completed);
    assert_eq!(summary.emitted_frames, expected_index);
}

#[test]
fn refuses_a_source_id_that_is_not_registered() {
    let descriptor =
        AudioSourceDescriptor::new("desktop-staged-missing", "missing.wav", Some(10), None)
            .expect("descriptor");
    let error = prepare_error(&SelectedSourceRegistry::new(), &descriptor);
    assert_eq!(error.code, "desktop.audio_decode.source_not_staged");
    assert!(error.retryable);
}

#[test]
fn refuses_a_stale_descriptor_for_the_current_source_id() {
    let temp = TempDir::new().expect("temp");
    let source_path = temp.path().join("source.wav");
    fs::write(&source_path, pcm_wav()).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let current = AudioSourceDescriptor::new(
        "desktop-staged-current",
        "source.wav",
        Some(byte_length),
        None,
    )
    .expect("current descriptor");
    let stale = AudioSourceDescriptor::new(
        "desktop-staged-current",
        "renamed.wav",
        Some(byte_length),
        None,
    )
    .expect("stale descriptor");
    let registry = SelectedSourceRegistry::new();
    registry
        .replace(InspectedAudioSource::from_staged(
            current,
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register");

    let error = prepare_error(&registry, &stale);
    assert_eq!(error.code, "desktop.audio_decode.source_changed");
}

fn prepare_error(
    registry: &SelectedSourceRegistry,
    descriptor: &AudioSourceDescriptor,
) -> crate::dto::DesktopErrorDto {
    match prepare_staged_audio_source(registry, descriptor, StreamingDecodeConfig::default()) {
        Err(error) => error,
        Ok(prepared) => {
            drop(prepared);
            panic!("source unexpectedly prepared")
        }
    }
}

fn pcm_wav() -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let frame_count = 4_410_u32;
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
