use crate::dto::DesktopErrorDto;
use crate::platform::file_picker::{
    AudioContainer, InspectedAudioSource, MAX_AUDIO_SOURCE_BYTES, SOURCE_SIGNATURE_BYTES,
    detect_container,
};
use crate::platform::source_staging_control::{
    SourceStagingOperation, SourceStagingProgressDto, SourceStagingProgressSink,
};
use sha2::{Digest, Sha256};
use silent_disco_core::runtime::AudioSourceDescriptor;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::{Builder, NamedTempFile};

const COPY_BUFFER_BYTES: usize = 64 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const TEMP_PREFIX: &str = ".silent-disco-source-";
const TEMP_SUFFIX: &str = ".part";
const TEMP_RANDOM_BYTES: usize = 12;
const CONTENT_SOURCE_PREFIX: &str = "desktop-staged-sha256-";
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagingResult {
    pub source: InspectedAudioSource,
    pub reused_existing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CopySummary {
    pub byte_length: u64,
    pub digest: [u8; 32],
    pub signature: Vec<u8>,
}

pub(crate) fn stage_audio_source(
    source: InspectedAudioSource,
    sources_directory: &Path,
    operation: &SourceStagingOperation,
    progress: &dyn SourceStagingProgressSink,
) -> Result<StagingResult, DesktopErrorDto> {
    operation.ensure_not_cancelled()?;
    let sources_directory = fs::canonicalize(sources_directory).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_directory_unavailable",
            false,
            "canonicalize the profile source directory",
            &error,
        )
    })?;
    let source_path = source.canonical_path().to_path_buf();
    let expected_length = source.descriptor().byte_length.ok_or_else(|| {
        staging_error(
            "desktop.audio_source.staging_length_missing",
            false,
            "the inspected audio source did not include a byte length",
        )
    })?;
    let expected_container = source.container();
    let mut input = File::open(&source_path).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_source_open_failed",
            true,
            "open the inspected audio source for staging",
            &error,
        )
    })?;
    let source_metadata = input.metadata().map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_source_metadata_failed",
            true,
            "inspect the opened audio source before staging",
            &error,
        )
    })?;
    if !source_metadata.file_type().is_file() {
        return Err(staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the inspected audio source is no longer a regular file",
        ));
    }
    if source_metadata.len() != expected_length {
        return Err(staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the audio source length changed after inspection",
        ));
    }

    let mut temporary = Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(TEMP_SUFFIX)
        .rand_bytes(TEMP_RANDOM_BYTES)
        .tempfile_in(&sources_directory)
        .map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_temp_create_failed",
                true,
                "create an owned temporary source file",
                &error,
            )
        })?;

    let copied = match copy_stream(
        &mut input,
        temporary.as_file_mut(),
        expected_length,
        operation,
        progress,
    ) {
        Ok(copied) => copied,
        Err(primary) => return Err(close_temporary(temporary, primary)),
    };
    let copied_container = detect_container(&copied.signature).ok_or_else(|| {
        staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the audio source signature became unsupported while staging",
        )
    });
    let copied_container = match copied_container {
        Ok(container) if container == expected_container => container,
        Ok(_) => {
            return Err(close_temporary(
                temporary,
                staging_error(
                    "desktop.audio_source.staging_source_changed",
                    true,
                    "the audio source container changed after inspection",
                ),
            ));
        }
        Err(primary) => return Err(close_temporary(temporary, primary)),
    };

    if let Err(error) = temporary.as_file_mut().flush() {
        return Err(close_temporary(
            temporary,
            staging_io_error(
                "desktop.audio_source.staging_flush_failed",
                true,
                "flush the staged temporary file",
                &error,
            ),
        ));
    }
    if let Err(error) = temporary.as_file().sync_all() {
        return Err(close_temporary(
            temporary,
            staging_io_error(
                "desktop.audio_source.staging_sync_failed",
                true,
                "synchronize the staged temporary file",
                &error,
            ),
        ));
    }
    if let Err(primary) = verify_open_file(
        temporary.as_file_mut(),
        copied.byte_length,
        &copied.digest,
        copied_container,
    ) {
        return Err(close_temporary(temporary, primary));
    }

    let digest_hex = encode_hex(&copied.digest);
    let final_path = sources_directory.join(format!(
        "{}.{}",
        digest_hex,
        copied_container.extension()
    ));
    let descriptor = AudioSourceDescriptor::new(
        format!("{CONTENT_SOURCE_PREFIX}{digest_hex}"),
        source.descriptor().display_name.clone(),
        Some(copied.byte_length),
        source.descriptor().duration_ms,
    )
    .map_err(|error| {
        staging_error(
            "desktop.audio_source.staging_descriptor_invalid",
            false,
            &format!("staged source descriptor is invalid: {error}"),
        )
    })?;

    if final_path.exists() {
        let result = reuse_existing(
            temporary,
            &final_path,
            copied.byte_length,
            &copied.digest,
            copied_container,
            descriptor,
        )?;
        sync_directory(&sources_directory)?;
        return Ok(result);
    }

    match temporary.persist_noclobber(&final_path) {
        Ok(published) => {
            published.sync_all().map_err(|error| {
                staging_io_error(
                    "desktop.audio_source.staging_publish_sync_failed",
                    true,
                    "synchronize the published staged source",
                    &error,
                )
            })?;
            drop(published);
            sync_directory(&sources_directory)?;
            let canonical_path = fs::canonicalize(&final_path).map_err(|error| {
                staging_io_error(
                    "desktop.audio_source.staging_publish_verify_failed",
                    true,
                    "canonicalize the published staged source",
                    &error,
                )
            })?;
            Ok(StagingResult {
                source: InspectedAudioSource::from_staged(
                    descriptor,
                    canonical_path,
                    copied_container,
                ),
                reused_existing: false,
            })
        }
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            let temporary = error.file;
            let result = reuse_existing(
                temporary,
                &final_path,
                copied.byte_length,
                &copied.digest,
                copied_container,
                descriptor,
            )?;
            sync_directory(&sources_directory)?;
            Ok(result)
        }
        Err(error) => {
            let primary = staging_io_error(
                "desktop.audio_source.staging_publish_failed",
                true,
                "publish the staged source without replacing existing data",
                &error.error,
            );
            Err(close_temporary(error.file, primary))
        }
    }
}

pub(crate) fn cleanup_incomplete_sources(sources_directory: &Path) -> Result<(), DesktopErrorDto> {
    let entries = match fs::read_dir(sources_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(staging_io_error(
                "desktop.audio_source.staging_cleanup_scan_failed",
                true,
                "scan the profile source directory for incomplete staging files",
                &error,
            ));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_cleanup_scan_failed",
                true,
                "read an entry while scanning incomplete staging files",
                &error,
            )
        })?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !is_owned_temporary_name(file_name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_cleanup_inspect_failed",
                true,
                "inspect an owned incomplete staging path",
                &error,
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(staging_error(
                "desktop.audio_source.staging_cleanup_refused",
                false,
                "an owned-looking incomplete staging path was not a regular file; cleanup was refused",
            ));
        }
        fs::remove_file(&path).map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_cleanup_failed",
                true,
                "remove an owned incomplete staging file",
                &error,
            )
        })?;
    }
    sync_directory(sources_directory)
}

pub(super) fn copy_stream<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    expected_length: u64,
    operation: &SourceStagingOperation,
    progress: &dyn SourceStagingProgressSink,
) -> Result<CopySummary, DesktopErrorDto> {
    if expected_length == 0 || expected_length > MAX_AUDIO_SOURCE_BYTES {
        return Err(staging_error(
            "desktop.audio_source.staging_length_invalid",
            false,
            "the inspected audio source length is outside the staging bounds",
        ));
    }
    operation.ensure_not_cancelled()?;
    progress.emit(SourceStagingProgressDto {
        copied_bytes: 0,
        total_bytes: expected_length,
    })?;

    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    let mut signature = Vec::with_capacity(SOURCE_SIGNATURE_BYTES);
    let mut digest = Sha256::new();
    let mut copied = 0_u64;
    let mut last_report = Instant::now();
    let mut last_reported_bytes = 0_u64;

    loop {
        operation.ensure_not_cancelled()?;
        let read = reader.read(&mut buffer).map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_source_read_failed",
                true,
                "read the audio source while staging",
                &error,
            )
        })?;
        if read == 0 {
            break;
        }
        let next = copied.checked_add(read as u64).ok_or_else(|| {
            staging_error(
                "desktop.audio_source.staging_length_overflow",
                false,
                "the staged source length overflowed",
            )
        })?;
        if next > expected_length || next > MAX_AUDIO_SOURCE_BYTES {
            return Err(staging_error(
                "desktop.audio_source.staging_source_changed",
                true,
                "the audio source grew after inspection",
            ));
        }
        writer.write_all(&buffer[..read]).map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_destination_write_failed",
                true,
                "write the staged temporary source",
                &error,
            )
        })?;
        if signature.len() < SOURCE_SIGNATURE_BYTES {
            let remaining = SOURCE_SIGNATURE_BYTES - signature.len();
            signature.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        digest.update(&buffer[..read]);
        copied = next;

        if last_report.elapsed() >= PROGRESS_INTERVAL {
            progress.emit(SourceStagingProgressDto {
                copied_bytes: copied,
                total_bytes: expected_length,
            })?;
            last_report = Instant::now();
            last_reported_bytes = copied;
        }
    }

    operation.ensure_not_cancelled()?;
    if copied != expected_length {
        return Err(staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the audio source became shorter after inspection",
        ));
    }
    if copied != last_reported_bytes {
        progress.emit(SourceStagingProgressDto {
            copied_bytes: copied,
            total_bytes: expected_length,
        })?;
    }
    Ok(CopySummary {
        byte_length: copied,
        digest: digest.finalize().into(),
        signature,
    })
}

pub(super) fn verify_path(
    path: &Path,
    expected_length: u64,
    expected_digest: &[u8; 32],
    expected_container: AudioContainer,
) -> Result<(), DesktopErrorDto> {
    let mut file = File::open(path).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_verify_open_failed",
            true,
            "open staged source content for verification",
            &error,
        )
    })?;
    verify_open_file(
        &mut file,
        expected_length,
        expected_digest,
        expected_container,
    )
}

fn verify_open_file(
    file: &mut File,
    expected_length: u64,
    expected_digest: &[u8; 32],
    expected_container: AudioContainer,
) -> Result<(), DesktopErrorDto> {
    let metadata = file.metadata().map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_verify_metadata_failed",
            true,
            "inspect staged source content for verification",
            &error,
        )
    })?;
    if !metadata.file_type().is_file() || metadata.len() != expected_length {
        return Err(staging_error(
            "desktop.audio_source.staging_verification_failed",
            true,
            "staged source metadata did not match the copied content",
        ));
    }
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_verify_seek_failed",
            true,
            "rewind staged source content for verification",
            &error,
        )
    })?;
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    let mut signature = Vec::with_capacity(SOURCE_SIGNATURE_BYTES);
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_verify_read_failed",
                true,
                "read staged source content for verification",
                &error,
            )
        })?;
        if read == 0 {
            break;
        }
        length = length.checked_add(read as u64).ok_or_else(|| {
            staging_error(
                "desktop.audio_source.staging_length_overflow",
                false,
                "verified staged source length overflowed",
            )
        })?;
        if signature.len() < SOURCE_SIGNATURE_BYTES {
            let remaining = SOURCE_SIGNATURE_BYTES - signature.len();
            signature.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        digest.update(&buffer[..read]);
    }
    let actual_digest: [u8; 32] = digest.finalize().into();
    if length != expected_length
        || &actual_digest != expected_digest
        || detect_container(&signature) != Some(expected_container)
    {
        return Err(staging_error(
            "desktop.audio_source.staging_verification_failed",
            true,
            "staged source length, hash, or signature verification failed",
        ));
    }
    Ok(())
}

fn reuse_existing(
    temporary: NamedTempFile,
    final_path: &Path,
    expected_length: u64,
    expected_digest: &[u8; 32],
    expected_container: AudioContainer,
    descriptor: AudioSourceDescriptor,
) -> Result<StagingResult, DesktopErrorDto> {
    let verification = verify_path(
        final_path,
        expected_length,
        expected_digest,
        expected_container,
    )
    .map_err(|error| {
        staging_error(
            "desktop.audio_source.staging_collision",
            false,
            &format!(
                "content-addressed staged source collision could not be reused safely: {}",
                error.message
            ),
        )
    });
    if let Err(primary) = verification {
        return Err(close_temporary(temporary, primary));
    }
    temporary.close().map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_temp_cleanup_failed",
            true,
            "remove the redundant owned temporary source",
            &error,
        )
    })?;
    let canonical_path = fs::canonicalize(final_path).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_reuse_verify_failed",
            true,
            "canonicalize the verified existing staged source",
            &error,
        )
    })?;
    Ok(StagingResult {
        source: InspectedAudioSource::from_staged(
            descriptor,
            canonical_path,
            expected_container,
        ),
        reused_existing: true,
    })
}

fn close_temporary(temporary: NamedTempFile, primary: DesktopErrorDto) -> DesktopErrorDto {
    match temporary.close() {
        Ok(()) => primary,
        Err(error) => DesktopErrorDto::new(
            &primary.code,
            &primary.subsystem,
            &primary.severity,
            primary.retryable,
            &format!(
                "{}; owned temporary source cleanup also failed: {error}",
                primary.message
            ),
        ),
    }
}

fn is_owned_temporary_name(file_name: &str) -> bool {
    let Some(random) = file_name
        .strip_prefix(TEMP_PREFIX)
        .and_then(|name| name.strip_suffix(TEMP_SUFFIX))
    else {
        return false;
    };
    random.len() == TEMP_RANDOM_BYTES && random.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn encode_hex(digest: &[u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), DesktopErrorDto> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_directory_sync_failed",
                true,
                "synchronize the profile source directory",
                &error,
            )
        })
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), DesktopErrorDto> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_directory_sync_failed",
                true,
                "synchronize the profile source directory",
                &error,
            )
        })
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> Result<(), DesktopErrorDto> {
    Ok(())
}

fn staging_io_error(
    code: &str,
    retryable: bool,
    operation: &str,
    error: &io::Error,
) -> DesktopErrorDto {
    staging_error(code, retryable, &format!("{operation}: {error}"))
}

fn staging_error(code: &str, retryable: bool, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(code, "audio_source", "error", retryable, message)
}

#[cfg(test)]
#[path = "source_staging_tests.rs"]
mod tests;
