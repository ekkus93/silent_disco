use super::file_picker::{
    AudioFileBoundary, AudioFileDialog, MAX_AUDIO_SOURCE_BYTES, OpenedAudioFile,
    SelectedSourceRegistry, select_and_inspect,
};
use crate::dto::DesktopErrorDto;
use silent_disco_core::runtime::MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES;
use std::fs::{self, File};
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-block16-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
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

struct FixedDialog(Result<Option<PathBuf>, DesktopErrorDto>);

impl AudioFileDialog for FixedDialog {
    fn pick_file(&self) -> Result<Option<PathBuf>, DesktopErrorDto> {
        self.0.clone()
    }
}

struct SystemBoundary;

impl AudioFileBoundary for SystemBoundary {
    fn open(&self, path: &Path) -> io::Result<OpenedAudioFile> {
        let metadata = fs::metadata(path)?;
        if !metadata.file_type().is_file() {
            return Ok(OpenedAudioFile::new(
                Box::new(io::empty()),
                false,
                metadata.len(),
            ));
        }
        let file = File::open(path)?;
        let opened_metadata = file.metadata()?;
        Ok(OpenedAudioFile::new(
            Box::new(file),
            opened_metadata.file_type().is_file(),
            opened_metadata.len(),
        ))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }
}

struct FailingBoundary(io::ErrorKind);

impl AudioFileBoundary for FailingBoundary {
    fn open(&self, _path: &Path) -> io::Result<OpenedAudioFile> {
        Err(io::Error::from(self.0))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
}

struct CanonicalizeFailingBoundary(io::ErrorKind);

impl AudioFileBoundary for CanonicalizeFailingBoundary {
    fn open(&self, _path: &Path) -> io::Result<OpenedAudioFile> {
        panic!("open must not run after canonicalization failure");
    }

    fn canonicalize(&self, _path: &Path) -> io::Result<PathBuf> {
        Err(io::Error::from(self.0))
    }
}

struct SyntheticBoundary {
    regular: bool,
    byte_length: u64,
    bytes: Vec<u8>,
}

impl AudioFileBoundary for SyntheticBoundary {
    fn open(&self, _path: &Path) -> io::Result<OpenedAudioFile> {
        Ok(OpenedAudioFile::new(
            Box::new(Cursor::new(self.bytes.clone())),
            self.regular,
            self.byte_length,
        ))
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
}

fn dialog_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.audio_source.dialog_failed",
        "audio_source",
        "error",
        true,
        "injected dialog failure",
    )
}

fn write_file(path: &Path, bytes: &[u8]) {
    let mut file = File::create(path).expect("create fixture");
    file.write_all(bytes).expect("write fixture");
    file.sync_all().expect("sync fixture");
}

#[test]
fn cancellation_is_distinct_from_failure() {
    let result = select_and_inspect(&FixedDialog(Ok(None)), &SystemBoundary)
        .expect("dialog cancellation is not a failure");
    assert!(result.is_none());
}

#[test]
fn dialog_failure_never_returns_success() {
    let error = select_and_inspect(&FixedDialog(Err(dialog_error())), &SystemBoundary)
        .expect_err("dialog failure must remain visible");
    assert_eq!(error.code, "desktop.audio_source.dialog_failed");
}

#[test]
fn nonexistent_file_is_rejected() {
    let root = TestDirectory::new();
    let error = select_and_inspect(
        &FixedDialog(Ok(Some(root.path("missing.wav")))),
        &SystemBoundary,
    )
    .expect_err("missing source must fail");
    assert_eq!(error.code, "desktop.audio_source.not_found");
}

#[test]
fn directory_selection_is_rejected() {
    let root = TestDirectory::new();
    let error = select_and_inspect(&FixedDialog(Ok(Some(root.0.clone()))), &SystemBoundary)
        .expect_err("directory must fail");
    assert_eq!(error.code, "desktop.audio_source.not_regular_file");
}

#[test]
fn empty_file_is_rejected() {
    let error = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("empty.wav")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 0,
            bytes: Vec::new(),
        },
    )
    .expect_err("empty source must fail");
    assert_eq!(error.code, "desktop.audio_source.empty");
}

#[test]
fn canonicalization_failure_is_explicit() {
    let error = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("unavailable.wav")))),
        &CanonicalizeFailingBoundary(io::ErrorKind::PermissionDenied),
    )
    .expect_err("canonicalization failure must remain visible");
    assert_eq!(error.code, "desktop.audio_source.permission_denied");
}

#[test]
fn oversized_file_is_rejected_before_reading_payload() {
    let error = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("oversized.flac")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: MAX_AUDIO_SOURCE_BYTES + 1,
            bytes: b"fLaC".to_vec(),
        },
    )
    .expect_err("oversized source must fail");
    assert_eq!(error.code, "desktop.audio_source.too_large");
}

#[test]
fn unicode_filename_is_preserved_in_bounded_descriptor() {
    let root = TestDirectory::new();
    let path = root.path("東京の夜.flac");
    write_file(&path, b"fLaCfixture");
    let source = select_and_inspect(&FixedDialog(Ok(Some(path))), &SystemBoundary)
        .expect("inspect Unicode source")
        .expect("source selected");
    assert_eq!(source.descriptor().display_name, "東京の夜.flac");
    assert_eq!(source.descriptor().byte_length, Some(11));
    assert!(source.descriptor().source_id.starts_with("desktop-source-"));
}

#[test]
fn long_unicode_name_is_truncated_on_a_character_boundary() {
    let name = format!("{}.flac", "界".repeat(100));
    let source = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from(name)))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 8,
            bytes: b"fLaCdata".to_vec(),
        },
    )
    .expect("inspect bounded Unicode source")
    .expect("source selected");
    assert!(source.descriptor().display_name.len() <= MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES);
    assert!(source.descriptor().display_name.ends_with('界'));
}

#[test]
fn control_characters_are_sanitized_from_display_name() {
    let source = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("bad\u{0007}name.flac")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 8,
            bytes: b"fLaCdata".to_vec(),
        },
    )
    .expect("inspect sanitized source")
    .expect("source selected");
    assert_eq!(source.descriptor().display_name, "bad name.flac");
}

#[test]
fn canonical_source_identity_is_deterministic_and_input_sensitive() {
    let first = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("same.wav")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 16,
            bytes: b"RIFF\0\0\0\0WAVEdata".to_vec(),
        },
    )
    .expect("inspect first")
    .expect("first selected");
    let same = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("same.wav")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 16,
            bytes: b"RIFF\0\0\0\0WAVEdata".to_vec(),
        },
    )
    .expect("inspect same")
    .expect("same selected");
    let different = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("same.wav")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 17,
            bytes: b"RIFF\0\0\0\0WAVEdata".to_vec(),
        },
    )
    .expect("inspect different")
    .expect("different selected");
    assert_eq!(first.descriptor().source_id, same.descriptor().source_id);
    assert_ne!(
        first.descriptor().source_id,
        different.descriptor().source_id
    );
}

#[test]
fn deceptive_extension_is_rejected_by_content_signature() {
    let root = TestDirectory::new();
    let path = root.path("deceptive.mp3");
    write_file(&path, b"this is not audio");
    let error = select_and_inspect(&FixedDialog(Ok(Some(path))), &SystemBoundary)
        .expect_err("extension alone must not be trusted");
    assert_eq!(error.code, "desktop.audio_source.unsupported");
}

#[test]
fn malformed_mpeg_sync_is_rejected() {
    let error = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("false-sync.mp3")))),
        &SyntheticBoundary {
            regular: true,
            byte_length: 4,
            bytes: vec![0xff, 0xfb, 0xf0, 0x00],
        },
    )
    .expect_err("reserved bitrate index must not identify MP3");
    assert_eq!(error.code, "desktop.audio_source.unsupported");
}

#[test]
fn permission_denied_is_explicit() {
    let error = select_and_inspect(
        &FixedDialog(Ok(Some(PathBuf::from("private.wav")))),
        &FailingBoundary(io::ErrorKind::PermissionDenied),
    )
    .expect_err("permission failure must remain visible");
    assert_eq!(error.code, "desktop.audio_source.permission_denied");
    assert!(error.retryable);
}

#[test]
fn registry_is_single_slot_and_rolls_back_only_current_selection() {
    let root = TestDirectory::new();
    let first_path = root.path("first.wav");
    let second_path = root.path("second.mp3");
    write_file(&first_path, b"RIFF\x00\x00\x00\x00WAVEfixture");
    write_file(&second_path, b"ID3fixture");
    let first = select_and_inspect(&FixedDialog(Ok(Some(first_path))), &SystemBoundary)
        .expect("inspect first")
        .expect("first selected");
    let second = select_and_inspect(&FixedDialog(Ok(Some(second_path))), &SystemBoundary)
        .expect("inspect second")
        .expect("second selected");
    let first_id = first.descriptor().source_id.clone();
    let second_id = second.descriptor().source_id.clone();
    let registry = SelectedSourceRegistry::new();
    assert!(
        registry
            .replace(first.clone())
            .expect("register first")
            .is_none()
    );
    let previous = registry.replace(second).expect("register second");
    registry
        .restore_if_current(&second_id, previous)
        .expect("restore first");
    assert!(
        registry
            .resolve(&first_id)
            .expect("resolve first")
            .is_some()
    );
    assert!(
        registry
            .resolve(&second_id)
            .expect("resolve second")
            .is_none()
    );
    let cleared = registry
        .clear()
        .expect("clear registry")
        .expect("first source retained before clear");
    assert_eq!(cleared.descriptor().source_id, first_id);
    assert!(
        registry
            .resolve(&first_id)
            .expect("resolve after clear")
            .is_none()
    );
}
