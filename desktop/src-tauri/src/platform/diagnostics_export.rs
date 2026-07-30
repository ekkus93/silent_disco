use super::failure::DesktopPlatformFailure;
use serde::Serialize;
use sha2::{Digest, Sha256};
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CapabilitySnapshot, CoreSnapshot};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsExport {
    schema_version: u16,
    export_id: String,
    snapshot: SnapshotExport,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotExport {
    revision: String,
    selected_role: Option<String>,
    host_lifecycle: String,
    listener_lifecycle: String,
    transport_state: String,
    playback_state: String,
    playback_position_ms: String,
    discovery_active: bool,
    discovered_session_count: usize,
    pending_join_request_count: usize,
    listener_count: usize,
    capabilities: CapabilityExport,
    last_error: Option<ErrorExport>,
    shutting_down: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the stable diagnostics wire record mirrors the core capability snapshot"
)]
struct CapabilityExport {
    nearby_discovery_available: bool,
    nearby_advertising_available: bool,
    local_network_available: bool,
    audio_source_selection_available: bool,
    audio_output_available: bool,
    secure_store_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorExport {
    code: String,
    subsystem: String,
    severity: String,
    retryable: bool,
    message: String,
}

impl From<CapabilitySnapshot> for CapabilityExport {
    fn from(value: CapabilitySnapshot) -> Self {
        Self {
            nearby_discovery_available: value.nearby_discovery_available,
            nearby_advertising_available: value.nearby_advertising_available,
            local_network_available: value.local_network_available,
            audio_source_selection_available: value.audio_source_selection_available,
            audio_output_available: value.audio_output_available,
            secure_store_available: value.secure_store_available,
        }
    }
}

impl From<&CoreSnapshot> for SnapshotExport {
    fn from(value: &CoreSnapshot) -> Self {
        Self {
            revision: value.revision.get().to_string(),
            selected_role: value.selected_role.map(|role| role.wire_name().to_owned()),
            host_lifecycle: value.host_lifecycle.wire_name().to_owned(),
            listener_lifecycle: value.listener_lifecycle.wire_name().to_owned(),
            transport_state: value.transport_state.wire_name().to_owned(),
            playback_state: value.playback_state.wire_name().to_owned(),
            playback_position_ms: value.playback_position_ms.to_string(),
            discovery_active: value.discovery_active,
            discovered_session_count: value.discovered_sessions.len(),
            pending_join_request_count: value.pending_join_requests.len(),
            listener_count: value.listeners.len(),
            capabilities: CapabilityExport::from(value.capabilities),
            last_error: value.last_error.as_ref().map(|error| ErrorExport {
                code: error.code.stable_name().to_owned(),
                subsystem: error.subsystem.stable_name().to_owned(),
                severity: error.severity.stable_name().to_owned(),
                retryable: error.retryable,
                message: error.message.clone(),
            }),
            shutting_down: value.shutting_down,
        }
    }
}

/// Writes one real, bounded diagnostics export into the application-owned profile directory.
///
/// The untrusted export identifier is hashed before it becomes a filename. The file is written
/// through a create-new temporary path, flushed, synchronized, and atomically renamed. Native
/// paths do not enter the core completion or frontend IPC contract.
///
/// # Errors
///
/// Returns a structured platform failure for an unsafe directory, serialization failure,
/// existing destination, write/sync failure, or atomic-install failure.
pub(super) fn write_export(
    diagnostics_directory: &Path,
    export_id: &str,
    snapshot: &CoreSnapshot,
) -> Result<(), DesktopPlatformFailure> {
    validate_directory(diagnostics_directory)?;
    let stem = export_filename_stem(export_id)?;
    let destination = diagnostics_directory.join(format!("diagnostics-{stem}.json"));
    let temporary = diagnostics_directory.join(format!(".diagnostics-{stem}.tmp"));
    if destination.exists() || temporary.exists() {
        return Err(failure(
            "desktop diagnostics export destination already exists",
        ));
    }

    let payload = DiagnosticsExport {
        schema_version: 1,
        export_id: export_id.to_owned(),
        snapshot: SnapshotExport::from(snapshot),
    };
    let encoded = serde_json::to_vec_pretty(&payload)
        .map_err(|_| failure("desktop diagnostics export serialization failed"))?;

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| failure("desktop diagnostics export temporary file could not be created"))?;
    let mut writer = BufWriter::new(file);
    let write_result = writer
        .write_all(&encoded)
        .and_then(|()| {
            writer.write_all(
                b"
",
            )
        })
        .and_then(|()| writer.flush())
        .and_then(|()| writer.get_ref().sync_all());
    if write_result.is_err() {
        let cleanup_failed = fs::remove_file(&temporary).is_err();
        return Err(if cleanup_failed {
            failure("desktop diagnostics export write and temporary cleanup failed")
        } else {
            failure("desktop diagnostics export write failed")
        });
    }
    drop(writer);

    if fs::rename(&temporary, &destination).is_err() {
        let cleanup_failed = fs::remove_file(&temporary).is_err();
        return Err(if cleanup_failed {
            failure("desktop diagnostics export install and temporary cleanup failed")
        } else {
            failure("desktop diagnostics export could not be installed atomically")
        });
    }
    File::open(diagnostics_directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| failure("desktop diagnostics export directory sync failed"))?;
    Ok(())
}

fn validate_directory(path: &Path) -> Result<(), DesktopPlatformFailure> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| failure("desktop diagnostics directory could not be inspected"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(failure(
            "desktop diagnostics directory is not a safe directory",
        ));
    }
    Ok(())
}

fn export_filename_stem(export_id: &str) -> Result<String, DesktopPlatformFailure> {
    let digest = Sha256::digest(export_id.as_bytes());
    let mut stem = String::with_capacity(digest.len() * 2);
    for byte in digest {
        if write!(&mut stem, "{byte:02x}").is_err() {
            return Err(failure("desktop diagnostics filename encoding failed"));
        }
    }
    Ok(stem)
}

const fn failure(message: &'static str) -> DesktopPlatformFailure {
    DesktopPlatformFailure::new(
        CoreErrorCode::PlatformOperationFailed,
        message,
        ErrorSeverity::Error,
        true,
    )
}
