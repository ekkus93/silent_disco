use super::decoder::active_worker_count;
use super::{
    AudioFormat, DecodeError, DecodeErrorKind, DecodeWorkerState, DecodedPcmChunk,
    StreamingDecodeConfig, StreamingDecodeHandle,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
static AUDIO_TEST_LOCK: Mutex<()> = Mutex::new(());

struct DecodedFixture {
    chunks: Vec<DecodedPcmChunk>,
    emitted_frames: u64,
}

#[test]
fn validates_every_resource_bound() {
    let _guard = audio_test_guard();
    for invalid in [
        StreamingDecodeConfig {
            chunk_frames: 0,
            ..StreamingDecodeConfig::default()
        },
        StreamingDecodeConfig {
            queue_chunks: 0,
            ..StreamingDecodeConfig::default()
        },
        StreamingDecodeConfig {
            max_source_bytes: 0,
            ..StreamingDecodeConfig::default()
        },
    ] {
        assert_eq!(
            invalid.validate().expect_err("invalid bound").kind,
            DecodeErrorKind::InvalidConfiguration
        );
    }
}

#[test]
fn decodes_wav_incrementally_to_canonical_pcm() {
    let _guard = audio_test_guard();
    assert_successful_fixture("wav");
}

#[test]
fn decodes_flac_incrementally_to_canonical_pcm() {
    let _guard = audio_test_guard();
    assert_successful_fixture("flac");
}

#[test]
fn decodes_mp3_incrementally_to_canonical_pcm() {
    let _guard = audio_test_guard();
    assert_successful_fixture("mp3");
}

#[test]
fn rejects_an_unenabled_container_as_unsupported() {
    let _guard = audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let path = write_bytes(&temp, "unsupported.ogg", b"OggSdesktop-block19-unsupported");
    let error = open_error(path);
    assert_eq!(error.kind, DecodeErrorKind::UnsupportedFormat);
}

#[test]
fn reports_truncated_audio_as_corrupt_not_unsupported() {
    let _guard = audio_test_guard();
    let mut bytes = fixture_bytes("flac");
    bytes.truncate(bytes.len() / 2);
    let temp = TempDir::new().expect("temp");
    let path = write_bytes(&temp, "truncated.flac", &bytes);
    let error = terminal_error(path);
    assert_eq!(error.kind, DecodeErrorKind::CorruptInput);
}

#[test]
fn rejects_invalid_id3_metadata_before_worker_creation() {
    let _guard = audio_test_guard();
    let bytes = b"ID3\x04\x00\x00\x00\x00\x01\x00tiny";
    let temp = TempDir::new().expect("temp");
    let path = write_bytes(&temp, "invalid.mp3", bytes);
    let error = open_error(path);
    assert_eq!(error.kind, DecodeErrorKind::InvalidMetadata);
    assert_eq!(active_worker_count(), 0);
}

#[test]
fn rejects_an_empty_source_without_starting_a_worker() {
    let _guard = audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let path = write_bytes(&temp, "empty.wav", &[]);
    let error = open_error(path);
    assert_eq!(error.kind, DecodeErrorKind::EmptySource);
    assert_eq!(active_worker_count(), 0);
}

#[test]
fn preserves_a_very_short_final_chunk() {
    let _guard = audio_test_guard();
    let config = StreamingDecodeConfig {
        chunk_frames: 1_000,
        queue_chunks: 8,
        ..StreamingDecodeConfig::default()
    };
    let decoded = decode_fixture("wav", config).expect("decode");
    let final_chunk = decoded.chunks.last().expect("final chunk");
    assert!(final_chunk.end_of_stream);
    assert!(final_chunk.frame_count() > 0);
    assert!(final_chunk.frame_count() < config.chunk_frames);
}

#[test]
fn exposes_bounded_backpressure_and_cancels_while_queue_is_full() {
    let _guard = audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let path = write_bytes(&temp, "long.wav", &pcm_wav(44_100, 1, 5));
    let config = StreamingDecodeConfig {
        chunk_frames: 64,
        queue_chunks: 1,
        ..StreamingDecodeConfig::default()
    };
    let handle = StreamingDecodeHandle::open(path, config).expect("open");
    wait_until(TEST_TIMEOUT, || handle.statistics().backpressure_events > 0);
    let statistics = handle.statistics();
    assert_eq!(statistics.queue_capacity_chunks, 1);
    assert_eq!(statistics.maximum_queued_duration_ms, 1);
    assert!(statistics.queued_chunks <= statistics.queue_capacity_chunks);
    assert!(statistics.backpressure_events > 0);

    let error = handle.cancel_and_join().expect_err("cancelled");
    assert_eq!(error.kind, DecodeErrorKind::Cancelled);
    assert_eq!(active_worker_count(), 0);
}

/// Block 35.1 "decoder/source queues" diagnostics: a reader obtained before
/// the handle is consumed elsewhere must observe the exact same live state
/// `statistics()` on the original handle would -- it's a view onto the same
/// atomics, not a snapshot frozen at the moment it was created.
#[test]
fn a_statistics_reader_observes_the_same_live_state_as_the_handle_it_was_taken_from() {
    let _guard = audio_test_guard();
    let temp = TempDir::new().expect("temp");
    let path = write_bytes(&temp, "long.wav", &pcm_wav(44_100, 1, 5));
    let config = StreamingDecodeConfig {
        chunk_frames: 64,
        queue_chunks: 1,
        ..StreamingDecodeConfig::default()
    };
    let handle = StreamingDecodeHandle::open(path, config).expect("open");
    let reader = handle.statistics_reader();
    wait_until(TEST_TIMEOUT, || reader.snapshot().backpressure_events > 0);

    let from_reader = reader.snapshot();
    let from_handle = handle.statistics();
    assert_eq!(
        from_reader.queue_capacity_chunks,
        from_handle.queue_capacity_chunks
    );
    assert_eq!(
        from_reader.backpressure_events,
        from_handle.backpressure_events
    );
    assert!(from_reader.backpressure_events > 0);

    drop(handle.cancel_and_join());
}

#[test]
fn a_new_source_starts_a_new_stream_at_sample_zero() {
    let _guard = audio_test_guard();
    let temp = TempDir::new().expect("temp");
    for extension in ["wav", "flac"] {
        let path = write_fixture(&temp, extension);
        let handle = StreamingDecodeHandle::open(
            path,
            StreamingDecodeConfig {
                chunk_frames: 64,
                queue_chunks: 1,
                ..StreamingDecodeConfig::default()
            },
        )
        .expect("open source");
        let first = handle.recv_timeout(TEST_TIMEOUT).expect("first chunk");
        assert_eq!(first.first_sample_index.get(), 0);
        let error = handle.cancel_and_join().expect_err("early cancellation");
        assert_eq!(error.kind, DecodeErrorKind::Cancelled);
        assert_eq!(active_worker_count(), 0);
    }
}

#[test]
fn repeated_open_close_leaves_no_worker_or_queue_growth() {
    let _guard = audio_test_guard();
    for _ in 0..24 {
        let decoded = decode_fixture("mp3", StreamingDecodeConfig::default()).expect("decode");
        assert!(!decoded.chunks.is_empty());
        assert_eq!(active_worker_count(), 0);
    }
}

/// Serializes every test that opens a real decode worker, including
/// packetizer-worker tests in the sibling `packetizer_tests` module, since
/// both observe the shared `active_worker_count()` global.
pub(super) fn audio_test_guard() -> MutexGuard<'static, ()> {
    AUDIO_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn assert_successful_fixture(extension: &str) {
    let config = StreamingDecodeConfig {
        chunk_frames: 257,
        queue_chunks: 3,
        ..StreamingDecodeConfig::default()
    };
    let decoded = decode_fixture(extension, config).expect("fixture decode");
    assert!(!decoded.chunks.is_empty());
    assert_eq!(
        decoded
            .chunks
            .iter()
            .filter(|chunk| chunk.end_of_stream)
            .count(),
        1
    );
    assert!(
        decoded
            .chunks
            .last()
            .is_some_and(|chunk| chunk.end_of_stream)
    );

    let mut expected_index = 0_u64;
    for chunk in &decoded.chunks {
        assert_eq!(chunk.format, AudioFormat::CANONICAL);
        assert_eq!(chunk.first_sample_index.get(), expected_index);
        assert!(!chunk.frames.is_empty());
        assert_eq!(chunk.frames.len() % 2, 0);
        assert!(chunk.frame_count() <= config.chunk_frames);
        expected_index += u64::try_from(chunk.frame_count()).expect("bounded frame count");
    }
    assert_eq!(decoded.emitted_frames, expected_index);
    assert!(decoded.emitted_frames > 500);
}

fn decode_fixture(
    extension: &str,
    config: StreamingDecodeConfig,
) -> Result<DecodedFixture, DecodeError> {
    let temp = TempDir::new().expect("temp");
    let path = write_fixture(&temp, extension);
    let handle = StreamingDecodeHandle::open(path, config)?;
    assert_eq!(handle.stream_info().format, AudioFormat::CANONICAL);
    assert!(handle.stream_info().source_byte_length > 0);
    assert_eq!(handle.stream_info().source_duration_ms, None);

    let mut chunks = Vec::new();
    loop {
        match handle.recv_timeout(TEST_TIMEOUT) {
            Ok(chunk) => {
                let end_of_stream = chunk.end_of_stream;
                chunks.push(chunk);
                if end_of_stream {
                    break;
                }
            }
            Err(error) => panic!("decoder disconnected before EOS: {error}"),
        }
    }
    let summary = handle.join()?;
    assert_eq!(summary.state, DecodeWorkerState::Completed);
    assert_eq!(active_worker_count(), 0);
    Ok(DecodedFixture {
        chunks,
        emitted_frames: summary.emitted_frames,
    })
}

fn open_error(path: PathBuf) -> DecodeError {
    match StreamingDecodeHandle::open(path, StreamingDecodeConfig::default()) {
        Err(error) => error,
        Ok(handle) => {
            drop(handle);
            panic!("source unexpectedly opened")
        }
    }
}

fn terminal_error(path: PathBuf) -> DecodeError {
    match StreamingDecodeHandle::open(path, StreamingDecodeConfig::default()) {
        Err(error) => error,
        Ok(handle) => {
            loop {
                match handle.recv_timeout(TEST_TIMEOUT) {
                    Ok(chunk) if chunk.end_of_stream => {
                        panic!("corrupt source unexpectedly reached EOS")
                    }
                    Ok(_) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        panic!("corrupt source did not terminate")
                    }
                }
            }
            handle.join().expect_err("corrupt worker result")
        }
    }
}

fn fixture_bytes(extension: &str) -> Vec<u8> {
    let encoded = match extension {
        "wav" => include_str!("fixtures/short.wav.b64"),
        "flac" => include_str!("fixtures/short.flac.b64"),
        "mp3" => include_str!("fixtures/short.mp3.b64"),
        _ => panic!("unknown fixture extension: {extension}"),
    };
    STANDARD.decode(encoded.trim()).expect("fixture base64")
}

fn write_fixture(temp: &TempDir, extension: &str) -> PathBuf {
    write_bytes(
        temp,
        &format!("short.{extension}"),
        &fixture_bytes(extension),
    )
}

fn write_bytes(temp: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = temp.path().join(name);
    fs::write(&path, bytes).expect("write fixture");
    path
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

fn pcm_wav(sample_rate: u32, channels: u16, seconds: u32) -> Vec<u8> {
    let frame_count = sample_rate.checked_mul(seconds).expect("fixture frames");
    let sample_count = frame_count
        .checked_mul(u32::from(channels))
        .expect("fixture samples");
    let data_bytes = sample_count.checked_mul(2).expect("fixture bytes");
    let riff_size = data_bytes.checked_add(36).expect("RIFF size");
    let byte_rate = sample_rate
        .checked_mul(u32::from(channels))
        .and_then(|value| value.checked_mul(2))
        .expect("byte rate");
    let block_align = channels.checked_mul(2).expect("block align");

    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for sample_index in 0..sample_count {
        let sample = if sample_index % 64 < 32 {
            8_000_i16
        } else {
            -8_000_i16
        };
        bytes.write_all(&sample.to_le_bytes()).expect("sample");
    }
    bytes
}
