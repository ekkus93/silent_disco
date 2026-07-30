#!/usr/bin/env python3
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
PAYLOAD = ROOT / "scripts" / "block17-payload"


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one anchor, found {count}: {old[:80]!r}")
    target.write_text(text.replace(old, new, 1))


def copy_payload(name: str, destination: str) -> None:
    source = PAYLOAD / name
    target = ROOT / destination
    if not source.is_file():
        raise SystemExit(f"missing Block 17 payload: {source}")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


for payload, destination in [
    ("source_staging.rs", "desktop/src-tauri/src/platform/source_staging.rs"),
    (
        "source_staging_control.rs",
        "desktop/src-tauri/src/platform/source_staging_control.rs",
    ),
    (
        "source_staging_tests.rs",
        "desktop/src-tauri/src/platform/source_staging_tests.rs",
    ),
]:
    copy_payload(payload, destination)

replace_once(
    "desktop/src-tauri/Cargo.toml",
    'sha2 = "=0.10.9"\n',
    'sha2 = "=0.10.9"\ntempfile = "=3.27.0"\n',
)

replace_once(
    "desktop/src-tauri/src/platform/mod.rs",
    "pub(crate) mod profile_lock;\n",
    "pub(crate) mod profile_lock;\npub(crate) mod source_staging;\npub(crate) mod source_staging_control;\n",
)

replace_once(
    "desktop/src-tauri/src/platform/file_picker.rs",
    "const SOURCE_SIGNATURE_BYTES: usize = 16;\n",
    "pub(crate) const SOURCE_SIGNATURE_BYTES: usize = 16;\n",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker.rs",
    "impl AudioContainer {\n    const fn identity_tag(self) -> &'static [u8] {\n",
    """impl AudioContainer {
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
        }
    }

    const fn identity_tag(self) -> &'static [u8] {
""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker.rs",
    """    pub(crate) fn descriptor(&self) -> &AudioSourceDescriptor {
        &self.descriptor
    }

""",
    """    pub(crate) fn descriptor(&self) -> &AudioSourceDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub(crate) fn from_staged(
        descriptor: AudioSourceDescriptor,
        canonical_path: PathBuf,
        container: AudioContainer,
    ) -> Self {
        Self {
            descriptor,
            canonical_path,
            container,
        }
    }

""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker.rs",
    "fn detect_container(signature: &[u8]) -> Option<AudioContainer> {\n",
    "pub(crate) fn detect_container(signature: &[u8]) -> Option<AudioContainer> {\n",
)

app_state = ROOT / "desktop/src-tauri/src/app_state.rs"
app_text = app_state.read_text()
test_marker = "\n#[cfg(test)]\nmod tests {\n"
if test_marker not in app_text:
    raise SystemExit("app_state.rs: test module marker not found")
production, tests = app_text.split(test_marker, 1)
if not tests.endswith("}\n"):
    raise SystemExit("app_state.rs: test module did not end with one closing brace")
(ROOT / "desktop/src-tauri/src/app_state_tests.rs").write_text(tests[:-2] + "\n")
app_state.write_text(
    production
    + '\n#[cfg(test)]\n#[path = "app_state_tests.rs"]\nmod tests;\n'
)

replace_once(
    "desktop/src-tauri/src/app_state.rs",
    "use crate::platform::profile_lock::ProfileLease;\n",
    """use crate::platform::profile_lock::ProfileLease;
use crate::platform::source_staging::cleanup_incomplete_sources;
use crate::platform::source_staging_control::SourceStagingControl;
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    "use std::sync::{Arc, Mutex};\n",
    "use std::path::PathBuf;\nuse std::sync::{Arc, Mutex};\n",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """struct ReadyRuntime {
    profile_id: ProfileId,
    _identity: DesktopIdentity,
""",
    """struct ReadyRuntime {
    profile_id: ProfileId,
    sources: PathBuf,
    _identity: DesktopIdentity,
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """    pub(crate) fn submit_core_command(
        &self,
""",
    """    pub(crate) fn source_staging_directory(&self) -> Result<PathBuf, DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => Ok(ready.sources.clone()),
            DesktopRuntimeState::Failed(error) => Err(error.clone()),
            DesktopRuntimeState::Closed
            | DesktopRuntimeState::Opening { .. }
            | DesktopRuntimeState::Closing => Err(DesktopErrorDto::new(
                "desktop.profile.not_ready",
                "runtime",
                "error",
                true,
                "no desktop profile is ready",
            )),
        }
    }

    pub(crate) fn submit_core_command(
        &self,
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """pub async fn close_profile(app: AppHandle) -> Result<BridgeLifecycleDto, DesktopErrorDto> {
    let action = app.state::<DesktopAppState>().take_for_close()?;
    let result = match action {
""",
    """pub async fn close_profile(app: AppHandle) -> Result<BridgeLifecycleDto, DesktopErrorDto> {
    let action = app.state::<DesktopAppState>().take_for_close()?;
    let staging_cleanup = app.state::<SourceStagingControl>().cancel_and_wait();
    let result = match action {
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """    let registry_cleanup = app.state::<SelectedSourceRegistry>().clear().map(|_| ());
    app.state::<DesktopAppState>()
        .finish_close(merge_close_results(result, registry_cleanup))?;
""",
    """    let registry_cleanup = app.state::<SelectedSourceRegistry>().clear().map(|_| ());
    let result = merge_close_results(result, staging_cleanup);
    app.state::<DesktopAppState>()
        .finish_close(merge_close_results(result, registry_cleanup))?;
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """    let identity = match provider.load_or_create(&profile_id) {
""",
    """    if let Err(primary) = cleanup_incomplete_sources(paths.sources()) {
        return Err(cleanup_lease(lease, primary));
    }

    let identity = match provider.load_or_create(&profile_id) {
""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """        ReadyRuntime {
            profile_id,
            _identity: identity,
""",
    """        ReadyRuntime {
            profile_id,
            sources: paths.sources().to_path_buf(),
            _identity: identity,
""",
)

replace_once(
    "desktop/src-tauri/src/host_commands.rs",
    "use crate::platform::file_picker::{SelectedSourceRegistry, pick_and_inspect};\n",
    """use crate::platform::file_picker::{SelectedSourceRegistry, pick_and_inspect};
use crate::platform::source_staging::stage_audio_source;
use crate::platform::source_staging_control::{
    SourceStagingControl, TauriSourceStagingProgressSink,
};
""",
)

old_select = """/// Opens one backend-owned file dialog, inspects one source, and registers its opaque descriptor.
///
/// Cancellation returns `Ok(None)` and is never reported as success or failure. Queue admission of
/// the resulting draft patch returns `Some(receipt)`; the frontend still waits for a newer snapshot.
///
/// # Errors
///
/// Returns a structured validation, dialog, filesystem, source-inspection, registry, actor, or
/// blocking-worker failure. Native paths never enter the command response or core command.
#[tauri::command]
pub async fn select_audio_source(
    app: AppHandle,
    request: RevisionCommandRequest,
) -> Result<Option<CommandReceiptDto>, DesktopErrorDto> {
    let revision = parse_snapshot_revision(&request.expected_revision)?;
    let picker_app = app.clone();
    let inspected = tauri::async_runtime::spawn_blocking(move || pick_and_inspect(picker_app))
        .await
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.audio_source.worker_failed",
                "audio_source",
                "error",
                true,
                &format!("audio source selection worker failed: {error}"),
            )
        })??;
    let Some(source) = inspected else {
        return Ok(None);
    };

    let descriptor = source.descriptor().clone();
    let source_id = descriptor.source_id.clone();
    let registry = app.state::<SelectedSourceRegistry>();
    let previous = registry.replace(source)?;
    let result = app.state::<DesktopAppState>().submit_core_command(
        revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: None,
        }),
    );
    match result {
        Ok(receipt) => Ok(Some(receipt)),
        Err(primary) => {
            if let Err(rollback) = registry.restore_if_current(&source_id, previous) {
                return Err(append_registration_rollback(&primary, &rollback));
            }
            Err(primary)
        }
    }
}
"""
new_select = """/// Opens one backend-owned file dialog, stages one source, and registers its opaque descriptor.
///
/// Dialog cancellation returns `Ok(None)`. Staging copies into the active profile through an owned
/// temporary file, verifies content, and atomically publishes before the actor sees the descriptor.
///
/// # Errors
///
/// Returns a structured validation, dialog, filesystem, staging, registry, actor, cancellation, or
/// blocking-worker failure. Native paths never enter the command response or core command.
#[tauri::command]
pub async fn select_audio_source(
    app: AppHandle,
    request: RevisionCommandRequest,
) -> Result<Option<CommandReceiptDto>, DesktopErrorDto> {
    let revision = parse_snapshot_revision(&request.expected_revision)?;
    let picker_app = app.clone();
    let inspected = tauri::async_runtime::spawn_blocking(move || pick_and_inspect(picker_app))
        .await
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.audio_source.worker_failed",
                "audio_source",
                "error",
                true,
                &format!("audio source selection worker failed: {error}"),
            )
        })??;
    let Some(source) = inspected else {
        return Ok(None);
    };

    let operation = app.state::<SourceStagingControl>().begin()?;
    let sources_directory = app
        .state::<DesktopAppState>()
        .source_staging_directory()?;
    let staging_app = app.clone();
    let staged = tauri::async_runtime::spawn_blocking(move || {
        let progress = TauriSourceStagingProgressSink::new(staging_app);
        stage_audio_source(source, &sources_directory, &operation, &progress)
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.audio_source.staging_worker_failed",
            "audio_source",
            "error",
            true,
            &format!("audio source staging worker failed: {error}"),
        )
    })??;
    let source = staged.source;
    let descriptor = source.descriptor().clone();
    let source_id = descriptor.source_id.clone();
    let registry = app.state::<SelectedSourceRegistry>();
    let previous = registry.replace(source)?;
    let result = app.state::<DesktopAppState>().submit_core_command(
        revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: None,
        }),
    );
    match result {
        Ok(receipt) => Ok(Some(receipt)),
        Err(primary) => {
            if let Err(rollback) = registry.restore_if_current(&source_id, previous) {
                return Err(append_registration_rollback(&primary, &rollback));
            }
            Err(primary)
        }
    }
}

/// Requests cancellation of the active bounded source-staging operation.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn cancel_audio_source_staging(state: State<'_, SourceStagingControl>) -> bool {
    state.cancel()
}
"""
replace_once("desktop/src-tauri/src/host_commands.rs", old_select, new_select)

replace_once(
    "desktop/src-tauri/src/lib.rs",
    """        .manage(platform::file_picker::SelectedSourceRegistry::new())
        .invoke_handler""",
    """        .manage(platform::file_picker::SelectedSourceRegistry::new())
        .manage(platform::source_staging_control::SourceStagingControl::new())
        .invoke_handler""",
)
replace_once(
    "desktop/src-tauri/src/lib.rs",
    """            host_commands::select_audio_source,
            host_commands::update_host_draft,
""",
    """            host_commands::select_audio_source,
            host_commands::cancel_audio_source_staging,
            host_commands::update_host_draft,
""",
)

print("Desktop Block 17 source transform applied")
