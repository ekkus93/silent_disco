#!/usr/bin/env python3
from pathlib import Path
import subprocess

BASE_INSTALLER_COMMIT = "34e5d932c8001ef079b05a5b6337799990fb0a96"
installer = subprocess.check_output(
    [
        "git",
        "show",
        f"{BASE_INSTALLER_COMMIT}:scripts/apply-desktop-block17.py",
    ],
    text=True,
)
installer = installer.replace('sha2 = "=0.10.9"', 'sha2 = "=0.11.0"')
installer = installer.replace(
    '"pub(crate) mod profile_lock;\\n",',
    '"pub mod profile_lock;\\n",',
)
installer = installer.replace(
    '"pub(crate) mod profile_lock;\\npub(crate) mod source_staging;\\npub(crate) mod source_staging_control;\\n",',
    '"pub mod profile_lock;\\npub(crate) mod source_staging;\\npub(crate) mod source_staging_control;\\n",',
)
exec(compile(installer, "scripts/apply-desktop-block17.py", "exec"))

staging = Path("desktop/src-tauri/src/platform/source_staging.rs")
text = staging.read_text()
start = text.index("pub(crate) fn stage_audio_source(")
end = text.index("pub(crate) fn cleanup_incomplete_sources", start)
stage_implementation = r'''pub(crate) fn stage_audio_source(
    source: &InspectedAudioSource,
    sources_directory: &Path,
    operation: &SourceStagingOperation,
    progress: &dyn SourceStagingProgressSink,
) -> Result<StagingResult, DesktopErrorDto> {
    operation.ensure_not_cancelled()?;
    let sources_directory = canonicalize_staging_directory(sources_directory)?;
    let expected_length = inspected_source_length(source)?;
    let expected_container = source.container();
    let mut input = open_staging_source(source, expected_length)?;
    let mut temporary = create_staging_temporary(&sources_directory)?;

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
    let copied_container = match validate_copied_container(&copied.signature, expected_container) {
        Ok(container) => container,
        Err(primary) => return Err(close_temporary(temporary, primary)),
    };
    if let Err(primary) = flush_and_verify_temporary(
        &mut temporary,
        &copied,
        copied_container,
    ) {
        return Err(close_temporary(temporary, primary));
    }

    let digest_hex = encode_hex(&copied.digest);
    let descriptor = match build_staged_descriptor(source, &copied, &digest_hex) {
        Ok(descriptor) => descriptor,
        Err(primary) => return Err(close_temporary(temporary, primary)),
    };
    publish_staged_source(
        temporary,
        &sources_directory,
        &copied,
        copied_container,
        &digest_hex,
        descriptor,
    )
}

fn canonicalize_staging_directory(path: &Path) -> Result<std::path::PathBuf, DesktopErrorDto> {
    fs::canonicalize(path).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_directory_unavailable",
            false,
            "canonicalize the profile source directory",
            &error,
        )
    })
}

fn inspected_source_length(source: &InspectedAudioSource) -> Result<u64, DesktopErrorDto> {
    source.descriptor().byte_length.ok_or_else(|| {
        staging_error(
            "desktop.audio_source.staging_length_missing",
            false,
            "the inspected audio source did not include a byte length",
        )
    })
}

fn open_staging_source(
    source: &InspectedAudioSource,
    expected_length: u64,
) -> Result<File, DesktopErrorDto> {
    let input = File::open(source.canonical_path()).map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_source_open_failed",
            true,
            "open the inspected audio source for staging",
            &error,
        )
    })?;
    let metadata = input.metadata().map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_source_metadata_failed",
            true,
            "inspect the opened audio source before staging",
            &error,
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the inspected audio source is no longer a regular file",
        ));
    }
    if metadata.len() != expected_length {
        return Err(staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the audio source length changed after inspection",
        ));
    }
    Ok(input)
}

fn create_staging_temporary(sources_directory: &Path) -> Result<NamedTempFile, DesktopErrorDto> {
    Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(TEMP_SUFFIX)
        .rand_bytes(TEMP_RANDOM_BYTES)
        .tempfile_in(sources_directory)
        .map_err(|error| {
            staging_io_error(
                "desktop.audio_source.staging_temp_create_failed",
                true,
                "create an owned temporary source file",
                &error,
            )
        })
}

fn validate_copied_container(
    signature: &[u8],
    expected_container: AudioContainer,
) -> Result<AudioContainer, DesktopErrorDto> {
    let actual = detect_container(signature).ok_or_else(|| {
        staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the audio source signature became unsupported while staging",
        )
    })?;
    if actual != expected_container {
        return Err(staging_error(
            "desktop.audio_source.staging_source_changed",
            true,
            "the audio source container changed after inspection",
        ));
    }
    Ok(actual)
}

fn flush_and_verify_temporary(
    temporary: &mut NamedTempFile,
    copied: &CopySummary,
    copied_container: AudioContainer,
) -> Result<(), DesktopErrorDto> {
    temporary.as_file_mut().flush().map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_flush_failed",
            true,
            "flush the staged temporary file",
            &error,
        )
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        staging_io_error(
            "desktop.audio_source.staging_sync_failed",
            true,
            "synchronize the staged temporary file",
            &error,
        )
    })?;
    verify_open_file(
        temporary.as_file_mut(),
        copied.byte_length,
        &copied.digest,
        copied_container,
    )
}

fn build_staged_descriptor(
    source: &InspectedAudioSource,
    copied: &CopySummary,
    digest_hex: &str,
) -> Result<AudioSourceDescriptor, DesktopErrorDto> {
    AudioSourceDescriptor::new(
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
    })
}

fn publish_staged_source(
    temporary: NamedTempFile,
    sources_directory: &Path,
    copied: &CopySummary,
    copied_container: AudioContainer,
    digest_hex: &str,
    descriptor: AudioSourceDescriptor,
) -> Result<StagingResult, DesktopErrorDto> {
    let final_path = sources_directory.join(format!(
        "{}.{}",
        digest_hex,
        copied_container.extension()
    ));
    if final_path.exists() {
        let result = reuse_existing(
            temporary,
            &final_path,
            copied.byte_length,
            &copied.digest,
            copied_container,
            descriptor,
        )?;
        sync_directory(sources_directory)?;
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
            sync_directory(sources_directory)?;
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
            let result = reuse_existing(
                error.file,
                &final_path,
                copied.byte_length,
                &copied.digest,
                copied_container,
                descriptor,
            )?;
            sync_directory(sources_directory)?;
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

'''
text = text[:start] + stage_implementation + text[end:]
old = "#[derive(Debug, Clone, PartialEq, Eq)]\npub(crate) struct StagingResult"
new = "#[derive(Debug, Clone)]\npub(crate) struct StagingResult"
if text.count(old) != 1:
    raise SystemExit("source_staging.rs: StagingResult derive anchor mismatch")
text = text.replace(old, new, 1)
old = "    pub reused_existing: bool,\n"
new = "    #[cfg_attr(not(test), allow(dead_code))]\n    pub reused_existing: bool,\n"
if text.count(old) != 1:
    raise SystemExit("source_staging.rs: reuse marker anchor mismatch")
text = text.replace(old, new, 1)
old = "let mut buffer = [0_u8; COPY_BUFFER_BYTES];"
new = "let mut buffer = vec![0_u8; COPY_BUFFER_BYTES].into_boxed_slice();"
if text.count(old) != 2:
    raise SystemExit("source_staging.rs: bounded buffer anchors mismatch")
text = text.replace(old, new)
staging.write_text(text)

control = Path("desktop/src-tauri/src/platform/source_staging_control.rs")
text = control.read_text()
old = "            .map(|state| state.active)\n            .unwrap_or(true)"
new = "            .map_or(true, |state| state.active)"
if text.count(old) != 1:
    raise SystemExit("source_staging_control.rs: cancel state anchor mismatch")
control.write_text(text.replace(old, new, 1))

tests = Path("desktop/src-tauri/src/platform/source_staging_tests.rs")
text = tests.read_text()
old = "        inspected_source(&source_path, &bytes),"
new = "        &inspected_source(&source_path, &bytes),"
if text.count(old) != 4:
    raise SystemExit("source_staging_tests.rs: staging borrow anchors mismatch")
text = text.replace(old, new)
old = "            self.control.cancel();"
new = "            let _ = self.control.cancel();"
if text.count(old) != 1:
    raise SystemExit("source_staging_tests.rs: cancellation result anchor mismatch")
tests.write_text(text.replace(old, new, 1))

host_commands = Path("desktop/src-tauri/src/host_commands.rs")
text = host_commands.read_text()
old = "        stage_audio_source(source, &sources_directory, &operation, &progress)"
new = "        stage_audio_source(&source, &sources_directory, &operation, &progress)"
if text.count(old) != 1:
    raise SystemExit("host_commands.rs: staging borrow anchor mismatch")
host_commands.write_text(text.replace(old, new, 1))

app_state_tests = Path("desktop/src-tauri/src/app_state_tests.rs")
text = app_state_tests.read_text()
marker = "use super::DesktopAppState;"
start = text.find(marker)
if start < 0:
    raise SystemExit("app_state_tests.rs: expected first use statement was not generated")
if text[:start].strip("\ufeff\r\n\t "):
    raise SystemExit("app_state_tests.rs: unexpected non-whitespace prefix")
normalized = text[start:]
if not normalized.startswith(marker):
    raise SystemExit("app_state_tests.rs: unexpected first statement after split")
app_state_tests.write_text(normalized)
