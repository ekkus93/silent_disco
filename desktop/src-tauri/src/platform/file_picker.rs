use crate::dto::DesktopErrorDto;
use sha2::{Digest, Sha256};
use silent_disco_core::runtime::{AudioSourceDescriptor, MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

pub(crate) const MAX_AUDIO_SOURCE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const SOURCE_SIGNATURE_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioContainer {
    Wav,
    Flac,
    Mp3,
}

impl AudioContainer {
    const fn identity_tag(self) -> &'static [u8] {
        match self {
            Self::Wav => b"wav",
            Self::Flac => b"flac",
            Self::Mp3 => b"mp3",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InspectedAudioSource {
    descriptor: AudioSourceDescriptor,
    canonical_path: PathBuf,
    container: AudioContainer,
}

impl InspectedAudioSource {
    #[must_use]
    pub(crate) fn descriptor(&self) -> &AudioSourceDescriptor {
        &self.descriptor
    }

    #[allow(
        dead_code,
        reason = "Block 17 consumes the registered canonical path and inspected container"
    )]
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    #[allow(
        dead_code,
        reason = "Block 17 consumes the inspected container before staging and decode"
    )]
    pub(crate) const fn container(&self) -> AudioContainer {
        self.container
    }
}

#[derive(Default)]
pub(crate) struct SelectedSourceRegistry {
    selected: Mutex<Option<InspectedAudioSource>>,
}

impl SelectedSourceRegistry {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn replace(
        &self,
        source: InspectedAudioSource,
    ) -> Result<Option<InspectedAudioSource>, DesktopErrorDto> {
        let mut selected = self.selected.lock().map_err(|_| registry_error())?;
        Ok(selected.replace(source))
    }

    pub(crate) fn restore_if_current(
        &self,
        current_source_id: &str,
        previous: Option<InspectedAudioSource>,
    ) -> Result<(), DesktopErrorDto> {
        let mut selected = self.selected.lock().map_err(|_| registry_error())?;
        if selected
            .as_ref()
            .is_some_and(|source| source.descriptor.source_id == current_source_id)
        {
            *selected = previous;
        }
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "Block 17 resolves the app-owned path by opaque source ID"
    )]
    pub(crate) fn resolve(
        &self,
        source_id: &str,
    ) -> Result<Option<InspectedAudioSource>, DesktopErrorDto> {
        let selected = self.selected.lock().map_err(|_| registry_error())?;
        Ok(selected
            .as_ref()
            .filter(|source| source.descriptor.source_id == source_id)
            .cloned())
    }
}

pub(crate) trait AudioFileDialog {
    fn pick_file(&self) -> Result<Option<PathBuf>, DesktopErrorDto>;
}

pub(crate) struct TauriAudioFileDialog<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriAudioFileDialog<R> {
    #[must_use]
    pub(crate) fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> AudioFileDialog for TauriAudioFileDialog<R> {
    fn pick_file(&self) -> Result<Option<PathBuf>, DesktopErrorDto> {
        let selected = self
            .app
            .dialog()
            .file()
            .set_title("Select an audio source")
            .add_filter("Supported audio", &["wav", "flac", "mp3"])
            .blocking_pick_file();
        selected
            .map(|path| {
                path.into_path().map_err(|error| {
                    source_error(
                        "desktop.audio_source.path_unavailable",
                        false,
                        &format!("selected file path could not be represented safely: {error}"),
                    )
                })
            })
            .transpose()
    }
}

pub(crate) trait AudioFileBoundary {
    fn open(&self, path: &Path) -> io::Result<OpenedAudioFile>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
}

pub(crate) struct OpenedAudioFile {
    reader: Box<dyn Read + Send>,
    is_regular_file: bool,
    byte_length: u64,
}

impl OpenedAudioFile {
    pub(crate) fn new(
        reader: Box<dyn Read + Send>,
        is_regular_file: bool,
        byte_length: u64,
    ) -> Self {
        Self {
            reader,
            is_regular_file,
            byte_length,
        }
    }
}

pub(crate) struct SystemAudioFileBoundary;

impl AudioFileBoundary for SystemAudioFileBoundary {
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

pub(crate) fn pick_and_inspect<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<InspectedAudioSource>, DesktopErrorDto> {
    select_and_inspect(&TauriAudioFileDialog::new(app), &SystemAudioFileBoundary)
}

pub(crate) fn select_and_inspect(
    dialog: &dyn AudioFileDialog,
    files: &dyn AudioFileBoundary,
) -> Result<Option<InspectedAudioSource>, DesktopErrorDto> {
    let Some(path) = dialog.pick_file()? else {
        return Ok(None);
    };
    inspect_source(&path, files).map(Some)
}

fn inspect_source(
    path: &Path,
    files: &dyn AudioFileBoundary,
) -> Result<InspectedAudioSource, DesktopErrorDto> {
    let safe_name = bounded_display_name(path)?;
    let canonical_path = files
        .canonicalize(path)
        .map_err(|error| map_io_error(error, "canonicalize the selected audio source"))?;
    let mut opened = files
        .open(&canonical_path)
        .map_err(|error| map_io_error(error, "open the selected audio source"))?;
    if !opened.is_regular_file {
        return Err(source_error(
            "desktop.audio_source.not_regular_file",
            false,
            "the selected source is not a regular file",
        ));
    }
    if opened.byte_length == 0 {
        return Err(source_error(
            "desktop.audio_source.empty",
            false,
            "the selected audio source is empty",
        ));
    }
    if opened.byte_length > MAX_AUDIO_SOURCE_BYTES {
        return Err(source_error(
            "desktop.audio_source.too_large",
            false,
            "the selected audio source exceeds the 8 GiB limit",
        ));
    }

    let mut signature = [0_u8; SOURCE_SIGNATURE_BYTES];
    let signature_length = opened
        .reader
        .read(&mut signature)
        .map_err(|error| map_io_error(error, "inspect the selected audio source"))?;
    let container = detect_container(&signature[..signature_length]).ok_or_else(|| {
        source_error(
            "desktop.audio_source.unsupported",
            false,
            "the selected file is not a supported WAV, FLAC, or MP3 source",
        )
    })?;
    let source_id = source_identity(&canonical_path, opened.byte_length, container);
    let descriptor =
        AudioSourceDescriptor::new(source_id, safe_name, Some(opened.byte_length), None).map_err(
            |error| {
                source_error(
                    "desktop.audio_source.invalid_descriptor",
                    false,
                    &format!("inspected audio source metadata is invalid: {error}"),
                )
            },
        )?;
    Ok(InspectedAudioSource {
        descriptor,
        canonical_path,
        container,
    })
}

fn detect_container(signature: &[u8]) -> Option<AudioContainer> {
    if signature.len() >= 12
        && (&signature[..4] == b"RIFF" || &signature[..4] == b"RF64")
        && &signature[8..12] == b"WAVE"
    {
        return Some(AudioContainer::Wav);
    }
    if signature.starts_with(b"fLaC") {
        return Some(AudioContainer::Flac);
    }
    if signature.starts_with(b"ID3") || has_mpeg_audio_frame_sync(signature) {
        return Some(AudioContainer::Mp3);
    }
    None
}

fn has_mpeg_audio_frame_sync(signature: &[u8]) -> bool {
    if signature.len() < 4 || signature[0] != 0xff || signature[1] & 0xe0 != 0xe0 {
        return false;
    }
    let version = (signature[1] >> 3) & 0x03;
    let layer = (signature[1] >> 1) & 0x03;
    let bitrate_index = signature[2] >> 4;
    let sample_rate_index = (signature[2] >> 2) & 0x03;
    version != 0x01
        && layer != 0
        && bitrate_index != 0
        && bitrate_index != 0x0f
        && sample_rate_index != 0x03
}

fn bounded_display_name(path: &Path) -> Result<String, DesktopErrorDto> {
    let raw = path.file_name().ok_or_else(|| {
        source_error(
            "desktop.audio_source.missing_name",
            false,
            "the selected audio source has no usable file name",
        )
    })?;
    let cleaned = raw
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Err(source_error(
            "desktop.audio_source.missing_name",
            false,
            "the selected audio source has no usable file name",
        ));
    }
    Ok(truncate_utf8_bytes(
        cleaned,
        MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES,
    ))
}

fn truncate_utf8_bytes(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn source_identity(path: &Path, byte_length: u64, container: AudioContainer) -> String {
    let mut hasher = Sha256::new();
    update_path_hash(&mut hasher, path);
    hasher.update([0]);
    hasher.update(byte_length.to_le_bytes());
    hasher.update([0]);
    hasher.update(container.identity_tag());
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(47);
    encoded.push_str("desktop-source-");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest.iter().take(16) {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(unix)]
fn update_path_hash(hasher: &mut Sha256, path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    hasher.update(path.as_os_str().as_bytes());
}

#[cfg(windows)]
fn update_path_hash(hasher: &mut Sha256, path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    for unit in path.as_os_str().encode_wide() {
        hasher.update(unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_path_hash(hasher: &mut Sha256, path: &Path) {
    hasher.update(path.to_string_lossy().as_bytes());
}

fn map_io_error(error: io::Error, operation: &str) -> DesktopErrorDto {
    let (code, retryable) = match error.kind() {
        io::ErrorKind::NotFound => ("desktop.audio_source.not_found", true),
        io::ErrorKind::PermissionDenied => ("desktop.audio_source.permission_denied", true),
        _ => ("desktop.audio_source.inspection_failed", true),
    };
    source_error(code, retryable, &format!("could not {operation}: {error}"))
}

fn registry_error() -> DesktopErrorDto {
    source_error(
        "desktop.audio_source.registry_poisoned",
        false,
        "the selected-source registry mutex was poisoned",
    )
}

fn source_error(code: &str, retryable: bool, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(code, "audio_source", "error", retryable, message)
}
