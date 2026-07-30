use super::{
    COPY_BUFFER_BYTES, TEMP_PREFIX, TEMP_SUFFIX, cleanup_incomplete_sources, copy_stream,
    encode_hex, stage_audio_source, verify_path,
};
use crate::dto::DesktopErrorDto;
use crate::platform::file_picker::{AudioContainer, InspectedAudioSource};
use crate::platform::source_staging_control::{
    SourceStagingControl, SourceStagingProgressDto, SourceStagingProgressSink,
};
use sha2::{Digest, Sha256};
use silent_disco_core::runtime::AudioSourceDescriptor;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-source-staging-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            assert!(
                error.kind() == io::ErrorKind::NotFound || std::thread::panicking(),
                "failed to remove test directory: {error}"
            );
        }
    }
}

#[derive(Default)]
struct RecordingProgress {
    events: Mutex<Vec<SourceStagingProgressDto>>,
}

impl RecordingProgress {
    fn events(&self) -> Vec<SourceStagingProgressDto> {
        self.events.lock().expect("progress mutex").clone()
    }
}

impl SourceStagingProgressSink for RecordingProgress {
    fn emit(&self, progress: SourceStagingProgressDto) -> Result<(), DesktopErrorDto> {
        self.events.lock().expect("progress mutex").push(progress);
        Ok(())
    }
}

fn wav_bytes(payload_byte: u8, payload_length: usize) -> Vec<u8> {
    let mut bytes = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
    bytes.extend(std::iter::repeat_n(payload_byte, payload_length));
    bytes
}

fn inspected_source(path: &Path, bytes: &[u8]) -> InspectedAudioSource {
    let descriptor = AudioSourceDescriptor::new(
        "desktop-prestage-test",
        path.file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned(),
        Some(bytes.len() as u64),
        None,
    )
    .expect("descriptor");
    InspectedAudioSource::from_staged(
        descriptor,
        fs::canonicalize(path).expect("canonical source"),
        AudioContainer::Wav,
    )
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[test]
fn staging_publishes_verified_content_and_preserves_original() {
    let root = TestDirectory::new();
    let sources = root.join("sources");
    fs::create_dir(&sources).expect("sources");
    let source_path = root.join("original.wav");
    let bytes = wav_bytes(0x41, 4096);
    fs::write(&source_path, &bytes).expect("write source");
    let original = fs::read(&source_path).expect("original bytes");
    let control = SourceStagingControl::new();
    let operation = control.begin().expect("begin");
    let progress = RecordingProgress::default();

    let result = stage_audio_source(
        &inspected_source(&source_path, &bytes),
        &sources,
        &operation,
        &progress,
    )
    .expect("stage source");

    assert!(!result.reused_existing);
    assert!(result.source.canonical_path().starts_with(&sources));
    assert_eq!(
        fs::read(result.source.canonical_path()).expect("staged"),
        bytes
    );
    assert_eq!(
        fs::read(&source_path).expect("original after stage"),
        original
    );
    assert!(
        result
            .source
            .descriptor()
            .source_id
            .starts_with("desktop-staged-sha256-")
    );
    let events = progress.events();
    assert_eq!(events.first().expect("initial").copied_bytes, 0);
    assert_eq!(
        events.last().expect("final").copied_bytes,
        bytes.len() as u64
    );
    assert!(
        events.len() <= 2,
        "rapid copies must not emit unbounded progress"
    );
    assert!(fs::read_dir(&sources).expect("read sources").all(|entry| {
        !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .starts_with(TEMP_PREFIX)
    }));
}

#[test]
fn verified_existing_content_is_reused_without_overwrite() {
    let root = TestDirectory::new();
    let sources = root.join("sources");
    fs::create_dir(&sources).expect("sources");
    let source_path = root.join("original.wav");
    let bytes = wav_bytes(0x52, 1024);
    fs::write(&source_path, &bytes).expect("source");
    let control = SourceStagingControl::new();
    let progress = RecordingProgress::default();

    let first_operation = control.begin().expect("first operation");
    let first = stage_audio_source(
        &inspected_source(&source_path, &bytes),
        &sources,
        &first_operation,
        &progress,
    )
    .expect("first stage");
    drop(first_operation);
    let first_path = first.source.canonical_path().to_path_buf();
    let first_bytes = fs::read(&first_path).expect("first staged bytes");

    let second_operation = control.begin().expect("second operation");
    let second = stage_audio_source(
        &inspected_source(&source_path, &bytes),
        &sources,
        &second_operation,
        &progress,
    )
    .expect("reuse");

    assert!(second.reused_existing);
    assert_eq!(second.source.canonical_path(), first_path);
    assert_eq!(fs::read(first_path).expect("reused bytes"), first_bytes);
}

#[test]
fn content_address_collision_fails_without_replacing_existing_file() {
    let root = TestDirectory::new();
    let sources = root.join("sources");
    fs::create_dir(&sources).expect("sources");
    let source_path = root.join("original.wav");
    let bytes = wav_bytes(0x61, 2048);
    fs::write(&source_path, &bytes).expect("source");
    let final_path = sources.join(format!("{}.wav", encode_hex(&digest(&bytes))));
    let conflicting = wav_bytes(0x62, 2048);
    assert_eq!(conflicting.len(), bytes.len());
    fs::write(&final_path, &conflicting).expect("conflicting final");
    let control = SourceStagingControl::new();
    let operation = control.begin().expect("operation");

    let error = stage_audio_source(
        &inspected_source(&source_path, &bytes),
        &sources,
        &operation,
        &RecordingProgress::default(),
    )
    .expect_err("collision");

    assert_eq!(error.code, "desktop.audio_source.staging_collision");
    assert_eq!(
        fs::read(&final_path).expect("collision content"),
        conflicting
    );
    assert_eq!(fs::read(&source_path).expect("original"), bytes);
}

struct CancellingReader {
    cursor: Cursor<Vec<u8>>,
    control: SourceStagingControl,
    reads: usize,
}

impl Read for CancellingReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.reads == 1 {
            let _ = self.control.cancel();
        }
        self.reads += 1;
        self.cursor.read(buffer)
    }
}

#[test]
fn cancellation_is_checked_during_bounded_copy() {
    let bytes = wav_bytes(0x33, COPY_BUFFER_BYTES * 2);
    let control = SourceStagingControl::new();
    let operation = control.begin().expect("operation");
    let mut reader = CancellingReader {
        cursor: Cursor::new(bytes.clone()),
        control: control.clone(),
        reads: 0,
    };
    let mut destination = Vec::new();

    let error = copy_stream(
        &mut reader,
        &mut destination,
        bytes.len() as u64,
        &operation,
        &RecordingProgress::default(),
    )
    .expect_err("cancelled");

    assert_eq!(error.code, "desktop.audio_source.staging_cancelled");
    assert!(destination.len() < bytes.len());
}

struct VanishingReader {
    first: Cursor<Vec<u8>>,
    failed: bool,
}

impl Read for VanishingReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.failed {
            return Err(io::Error::new(io::ErrorKind::NotFound, "source vanished"));
        }
        self.failed = true;
        let limit = buffer.len().min(32);
        self.first.read(&mut buffer[..limit])
    }
}

#[test]
fn source_read_failure_is_visible_and_never_publishes() {
    let bytes = wav_bytes(0x20, 128);
    let control = SourceStagingControl::new();
    let operation = control.begin().expect("operation");
    let mut reader = VanishingReader {
        first: Cursor::new(bytes.clone()),
        failed: false,
    };

    let error = copy_stream(
        &mut reader,
        &mut Vec::new(),
        bytes.len() as u64,
        &operation,
        &RecordingProgress::default(),
    )
    .expect_err("source failure");

    assert_eq!(
        error.code,
        "desktop.audio_source.staging_source_read_failed"
    );
}

struct FailingWriter {
    accepted: usize,
}

impl Write for FailingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.accepted == 0 {
            let accepted = buffer.len().min(8);
            self.accepted = accepted;
            return Ok(accepted);
        }
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "destination failed",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn destination_write_failure_is_visible() {
    let bytes = wav_bytes(0x44, 128);
    let control = SourceStagingControl::new();
    let operation = control.begin().expect("operation");

    let error = copy_stream(
        &mut Cursor::new(bytes.clone()),
        &mut FailingWriter { accepted: 0 },
        bytes.len() as u64,
        &operation,
        &RecordingProgress::default(),
    )
    .expect_err("write failure");

    assert_eq!(
        error.code,
        "desktop.audio_source.staging_destination_write_failed"
    );
}

#[test]
fn verification_rejects_hash_mismatch() {
    let root = TestDirectory::new();
    let path = root.join("staged.wav");
    let bytes = wav_bytes(0x77, 64);
    fs::write(&path, &bytes).expect("staged");

    let error = verify_path(
        &path,
        bytes.len() as u64,
        &digest(&wav_bytes(0x78, 64)),
        AudioContainer::Wav,
    )
    .expect_err("hash mismatch");

    assert_eq!(
        error.code,
        "desktop.audio_source.staging_verification_failed"
    );
}

#[test]
fn startup_cleanup_removes_only_strict_owned_regular_temps() {
    let root = TestDirectory::new();
    let sources = root.join("sources");
    fs::create_dir(&sources).expect("sources");
    let owned = sources.join(format!("{TEMP_PREFIX}Ab12Cd34Ef56{TEMP_SUFFIX}"));
    let unrelated = sources.join(".silent-disco-source-not-owned.part");
    fs::write(&owned, b"incomplete").expect("owned temp");
    fs::write(&unrelated, b"unrelated").expect("unrelated");

    cleanup_incomplete_sources(&sources).expect("cleanup");

    assert!(!owned.exists());
    assert!(unrelated.exists());
}

#[cfg(unix)]
#[test]
fn startup_cleanup_refuses_owned_looking_symlink() {
    use std::os::unix::fs::symlink;

    let root = TestDirectory::new();
    let sources = root.join("sources");
    fs::create_dir(&sources).expect("sources");
    let target = root.join("target.wav");
    fs::write(&target, b"preserve me").expect("target");
    let owned = sources.join(format!("{TEMP_PREFIX}Ab12Cd34Ef56{TEMP_SUFFIX}"));
    symlink(&target, &owned).expect("symlink");

    let error = cleanup_incomplete_sources(&sources).expect_err("refuse symlink");

    assert_eq!(error.code, "desktop.audio_source.staging_cleanup_refused");
    assert_eq!(fs::read(target).expect("target"), b"preserve me");
}

#[cfg(not(unix))]
#[test]
fn startup_cleanup_refuses_owned_looking_non_file() {
    let root = TestDirectory::new();
    let sources = root.join("sources");
    fs::create_dir(&sources).expect("sources");
    let owned = sources.join(format!("{TEMP_PREFIX}Ab12Cd34Ef56{TEMP_SUFFIX}"));
    fs::create_dir(&owned).expect("owned-looking directory");

    let error = cleanup_incomplete_sources(&sources).expect_err("refuse non-file");

    assert_eq!(error.code, "desktop.audio_source.staging_cleanup_refused");
    assert!(owned.is_dir());
}
