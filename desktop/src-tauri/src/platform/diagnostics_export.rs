use super::failure::DesktopPlatformFailure;
use crate::diagnostics_dto::DesktopDiagnosticsDto;
use crate::dto::DesktopErrorDto;
use serde::Serialize;
use sha2::{Digest, Sha256};
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CapabilitySnapshot, CoreSnapshot};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;
use ts_rs::TS;

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

fn failure(message: &'static str) -> DesktopPlatformFailure {
    DesktopPlatformFailure::new(
        CoreErrorCode::PlatformOperationFailed,
        message,
        ErrorSeverity::Error,
        true,
    )
}

// ---------------------------------------------------------------------------
// Block 35.3: user-directed diagnostics export to an arbitrary, save-dialog-
// chosen destination. Deliberately independent of `write_export` above (which
// remains the narrow, unused `CoreCommand::ExportDiagnostics` pathway) --
// this exports the rich, bounded `DesktopDiagnosticsDto` (Block 35.1) to
// wherever the user picks, following the same "trait for the I/O boundary,
// plain function for orchestration" discipline as `file_picker.rs`.
// ---------------------------------------------------------------------------

/// Hard cap on the exported file's byte size. The DTO is already bounded
/// (Block 35.1's `listeners_truncated`/`MAX_DIAGNOSTICS_LISTENERS`), so this
/// is defense in depth, not the primary bounding mechanism -- exceeding it
/// means something upstream stopped bounding correctly, and that must be a
/// loud failure, not a silently truncated file.
const MAX_EXPORT_BYTES: usize = 1024 * 1024;

/// Distinguishes "the user picked Cancel" from every other failure (Block
/// 35.3 "cancellation distinct from failure"). Only [`DiagnosticsExportOutcome::Saved`]
/// means a file was actually committed to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[ts(rename_all = "camelCase")]
pub enum DiagnosticsExportOutcome {
    Saved,
    Cancelled,
}

pub(crate) trait DiagnosticsSaveDialog {
    fn pick_destination(
        &self,
        suggested_file_name: &str,
    ) -> Result<Option<PathBuf>, DesktopErrorDto>;
}

pub(crate) struct TauriDiagnosticsSaveDialog<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> TauriDiagnosticsSaveDialog<R> {
    #[must_use]
    pub(crate) fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> DiagnosticsSaveDialog for TauriDiagnosticsSaveDialog<R> {
    fn pick_destination(
        &self,
        suggested_file_name: &str,
    ) -> Result<Option<PathBuf>, DesktopErrorDto> {
        // Mirrors `file_picker.rs::TauriAudioFileDialog::pick_file`: the
        // pinned dialog plugin reports native close/cancel as `None`, with
        // no separate picker-backend-error channel.
        let selected = self
            .app
            .dialog()
            .file()
            .set_title("Save diagnostics export")
            .add_filter("Diagnostics JSON", &["json"])
            .set_file_name(suggested_file_name)
            .blocking_save_file();
        selected
            .map(|path| {
                path.into_path().map_err(|error| {
                    export_error(
                        "desktop.diagnostics.path_unavailable",
                        false,
                        &format!("selected destination could not be represented safely: {error}"),
                    )
                })
            })
            .transpose()
    }
}

/// Suggests a stable, sortable default file name for the save dialog. Not
/// itself a privacy concern -- it embeds only a wall-clock timestamp, never
/// used for sync/playback scheduling (monotonic-only, per this project's
/// rules), just a human-facing default file name.
#[must_use]
pub(crate) fn suggested_export_file_name(now_ms: u64) -> String {
    format!("silent-disco-diagnostics-{now_ms}.json")
}

/// Real entry point wired to the `export_host_diagnostics` Tauri command:
/// opens a native save dialog, then writes the export if the user picked a
/// destination.
///
/// # Errors
///
/// Returns a structured error for dialog, serialization, bounds, or write
/// failure. Cancellation is not an error -- see [`DiagnosticsExportOutcome`].
pub(crate) fn export_host_diagnostics<R: Runtime>(
    app: AppHandle<R>,
    diagnostics: &DesktopDiagnosticsDto,
    now_ms: u64,
) -> Result<DiagnosticsExportOutcome, DesktopErrorDto> {
    export_with_dialog(
        &TauriDiagnosticsSaveDialog::new(app),
        diagnostics,
        &suggested_export_file_name(now_ms),
    )
}

/// Pure(ish) orchestration over the [`DiagnosticsSaveDialog`] boundary --
/// testable with a fake dialog, matching `file_picker.rs::select_and_inspect`.
pub(crate) fn export_with_dialog(
    dialog: &dyn DiagnosticsSaveDialog,
    diagnostics: &DesktopDiagnosticsDto,
    suggested_file_name: &str,
) -> Result<DiagnosticsExportOutcome, DesktopErrorDto> {
    let Some(destination) = dialog.pick_destination(suggested_file_name)? else {
        return Ok(DiagnosticsExportOutcome::Cancelled);
    };
    write_diagnostics_export(&destination, diagnostics)?;
    Ok(DiagnosticsExportOutcome::Saved)
}

/// Writes `diagnostics` to `destination` -- a temporary file is created
/// alongside it, written, flushed, and synced, then installed only once the
/// payload is fully durable (Block 35.3 "temporary write then atomic rename
/// where supported" / "no success until file is committed"). A pre-existing
/// destination is replaced: the native save dialog that produced it already
/// confirmed the overwrite with the user (Block 35.3 "existing file
/// policy"). Any failure removes the temporary file it created (Block 35.3
/// "partial write cleanup") and never touches a pre-existing destination.
///
/// # Errors
///
/// Returns a structured error for an oversized payload, an unwritable
/// destination directory, a write/sync failure, or an install failure.
pub(crate) fn write_diagnostics_export(
    destination: &Path,
    diagnostics: &DesktopDiagnosticsDto,
) -> Result<(), DesktopErrorDto> {
    let encoded = serde_json::to_vec_pretty(diagnostics).map_err(|_| {
        export_error(
            "desktop.diagnostics.serialization_failed",
            false,
            "diagnostics export serialization failed",
        )
    })?;
    enforce_bounded_export_size(&encoded)?;

    let file_name = destination.file_name().ok_or_else(|| {
        export_error(
            "desktop.diagnostics.invalid_destination",
            false,
            "the chosen destination has no file name",
        )
    })?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(".tmp");
    let temporary = destination.with_file_name(temporary_name);

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            map_export_io_error(
                &error,
                "create a temporary file next to the chosen destination",
            )
        })?;
    let mut writer = BufWriter::new(file);
    let write_result = writer
        .write_all(&encoded)
        .and_then(|()| writer.flush())
        .and_then(|()| writer.get_ref().sync_all());
    if let Err(error) = write_result {
        let cleanup_failed = fs::remove_file(&temporary).is_err();
        return Err(if cleanup_failed {
            export_error(
                "desktop.diagnostics.write_and_cleanup_failed",
                true,
                &format!(
                    "diagnostics export write failed and the temporary file could not be removed: {error}"
                ),
            )
        } else {
            map_export_io_error(&error, "write the diagnostics export")
        });
    }
    drop(writer);

    if let Err(error) = install_atomically(&temporary, destination) {
        let cleanup_failed = fs::remove_file(&temporary).is_err();
        return Err(if cleanup_failed {
            export_error(
                "desktop.diagnostics.install_and_cleanup_failed",
                true,
                &format!(
                    "diagnostics export could not be installed and the temporary file could not be removed: {error}"
                ),
            )
        } else {
            map_export_io_error(
                &error,
                "install the diagnostics export at the chosen destination",
            )
        });
    }
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                map_export_io_error(&error, "synchronize the destination directory")
            })?;
    }
    Ok(())
}

/// Installs `temporary` over `destination`. `std::fs::rename` already
/// replaces an existing regular-file destination atomically on POSIX; on
/// Windows it does not, so a pre-existing destination is removed first and
/// the rename retried once. Either way, the payload was already fully
/// written and synced to `temporary` before this is called.
fn install_atomically(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    match fs::rename(temporary, destination) {
        Ok(()) => Ok(()),
        Err(error) => {
            #[cfg(windows)]
            {
                if destination.is_file() {
                    fs::remove_file(destination)?;
                    return fs::rename(temporary, destination);
                }
            }
            Err(error)
        }
    }
}

fn enforce_bounded_export_size(encoded: &[u8]) -> Result<(), DesktopErrorDto> {
    if encoded.len() > MAX_EXPORT_BYTES {
        return Err(export_error(
            "desktop.diagnostics.export_too_large",
            false,
            "diagnostics export exceeded its bounded size and was not written",
        ));
    }
    Ok(())
}

fn map_export_io_error(error: &std::io::Error, operation: &str) -> DesktopErrorDto {
    let (code, retryable) = match error.kind() {
        std::io::ErrorKind::NotFound => ("desktop.diagnostics.destination_not_found", true),
        std::io::ErrorKind::PermissionDenied => ("desktop.diagnostics.permission_denied", true),
        std::io::ErrorKind::AlreadyExists => ("desktop.diagnostics.temporary_file_exists", true),
        _ => ("desktop.diagnostics.write_failed", true),
    };
    export_error(code, retryable, &format!("could not {operation}: {error}"))
}

fn export_error(code: &str, retryable: bool, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(code, "diagnostics", "error", retryable, message)
}

#[cfg(test)]
mod export_tests {
    use super::{
        DiagnosticsExportOutcome, DiagnosticsSaveDialog, enforce_bounded_export_size,
        export_with_dialog, write_diagnostics_export,
    };
    use crate::diagnostics_dto::{
        DesktopDiagnosticsDto, IdentityDiagnosticsDto, MonitorDiagnosticsDto,
        NotificationBridgeDiagnosticsDto, ProfileDiagnosticsDto, StorageDiagnosticsDto,
        TransportDiagnosticsDto, VersionsDiagnosticsDto,
    };
    use crate::dto::{CoreVersionDto, DesktopErrorDto};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "silent-disco-desktop-diagnostics-export-{}-{sequence}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).expect("remove stale test directory");
            }
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture_dto() -> DesktopDiagnosticsDto {
        DesktopDiagnosticsDto {
            versions: VersionsDiagnosticsDto {
                core_version: CoreVersionDto::from(silent_disco_core::core_version()),
                app_version: "0.1.0-test".to_owned(),
                export_schema_version: 2,
            },
            profile: ProfileDiagnosticsDto {
                profile_id: "profile-export-test".to_owned(),
                platform: std::env::consts::OS.to_owned(),
            },
            storage: StorageDiagnosticsDto {
                available: true,
                schema_version: Some(1),
                journal_mode: Some("wal".to_owned()),
                foreign_keys_enabled: Some(true),
                integrity_check: Some("ok".to_owned()),
                applied_migration_count: Some(1),
                failure_reason: None,
            },
            identity: IdentityDiagnosticsDto {
                device_identity_present: true,
                signing_identity_present: true,
                signing_key_fingerprint: Some("abcd1234".to_owned()),
            },
            endpoint: None,
            transport: TransportDiagnosticsDto {
                state: "idle".to_owned(),
                last_delivery: None,
                broadcast: None,
            },
            listeners: Vec::new(),
            listeners_truncated: false,
            synchronization: None,
            decode_queue: None,
            packetize_queue: None,
            monitor: MonitorDiagnosticsDto {
                enabled: false,
                active: false,
                failure_reason: None,
                callback_count: None,
                frames_written: None,
                frames_silence_filled: None,
            },
            notification_bridge: NotificationBridgeDiagnosticsDto {
                delivery_failure: None,
            },
            last_error: None,
            shutting_down: false,
            generated_at_ms: "0".to_owned(),
        }
    }

    struct FakeDialog(Option<PathBuf>);

    impl DiagnosticsSaveDialog for FakeDialog {
        fn pick_destination(
            &self,
            _suggested_file_name: &str,
        ) -> Result<Option<PathBuf>, DesktopErrorDto> {
            Ok(self.0.clone())
        }
    }

    /// Block 35.3 "cancellation distinct from failure": a `None` from the
    /// dialog must produce `Ok(Cancelled)`, never an `Err`.
    #[test]
    fn a_cancelled_dialog_produces_cancelled_not_an_error() {
        let outcome = export_with_dialog(&FakeDialog(None), &fixture_dto(), "diagnostics.json")
            .expect("cancellation is not a failure");
        assert_eq!(outcome, DiagnosticsExportOutcome::Cancelled);
    }

    /// The common path: a chosen destination is written and reported saved.
    #[test]
    fn a_chosen_destination_is_written_and_reported_saved() {
        let directory = TestDirectory::new();
        let destination = directory.0.join("diagnostics.json");
        let outcome = export_with_dialog(
            &FakeDialog(Some(destination.clone())),
            &fixture_dto(),
            "diagnostics.json",
        )
        .expect("write succeeds");
        assert_eq!(outcome, DiagnosticsExportOutcome::Saved);
        let written = fs::read_to_string(&destination).expect("read exported file");
        assert!(written.contains("profile-export-test"));
        assert!(!directory.0.join("diagnostics.json.tmp").exists());
    }

    /// Block 35.3 "existing file policy": a pre-existing destination (the
    /// native save dialog already confirmed the overwrite) is replaced, not
    /// rejected.
    #[test]
    fn a_pre_existing_destination_is_overwritten() {
        let directory = TestDirectory::new();
        let destination = directory.0.join("diagnostics.json");
        fs::write(&destination, "stale content").expect("seed existing destination");

        write_diagnostics_export(&destination, &fixture_dto()).expect("overwrite succeeds");

        let written = fs::read_to_string(&destination).expect("read exported file");
        assert!(!written.contains("stale content"));
        assert!(written.contains("profile-export-test"));
    }

    /// Block 35.3 "destination failure": a parent directory that does not
    /// exist must surface a structured error, not a panic or a false
    /// success.
    #[test]
    fn a_missing_destination_directory_is_a_reported_failure() {
        let directory = TestDirectory::new();
        let destination = directory.0.join("does-not-exist").join("diagnostics.json");
        let error = write_diagnostics_export(&destination, &fixture_dto())
            .expect_err("missing parent directory must fail");
        assert_eq!(error.code, "desktop.diagnostics.destination_not_found");
    }

    /// Block 35.3 "partial write cleanup": forces a real, portable install
    /// failure (the destination already exists as a directory, so the final
    /// rename cannot succeed) after the temporary file was already written,
    /// flushed, and synced -- and asserts the temporary file left behind by
    /// that successful write stage is cleaned up, not orphaned.
    #[test]
    fn an_install_failure_cleans_up_its_temporary_file() {
        let directory = TestDirectory::new();
        let destination = directory.0.join("diagnostics.json");
        fs::create_dir(&destination).expect("seed a directory occupying the destination path");

        let error = write_diagnostics_export(&destination, &fixture_dto())
            .expect_err("installing over an existing directory must fail");
        assert_eq!(error.code, "desktop.diagnostics.write_failed");
        assert!(
            !directory.0.join("diagnostics.json.tmp").exists(),
            "the temporary file must not be left behind after a failed install"
        );
        assert!(
            destination.is_dir(),
            "the pre-existing directory must be untouched"
        );
    }

    /// Block 35.1/35.3 "bounded size": an oversized payload must never be
    /// written -- defense in depth on top of the DTO's own bounding.
    #[test]
    fn an_oversized_payload_is_rejected_before_any_write() {
        let oversized = vec![0_u8; super::MAX_EXPORT_BYTES + 1];
        let error = enforce_bounded_export_size(&oversized)
            .expect_err("oversized payload must be rejected");
        assert_eq!(error.code, "desktop.diagnostics.export_too_large");
    }
}
