from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one replacement, found {count}")
    target.write_text(text.replace(old, new, 1))


# Return the exact post-publication authoritative snapshot from startup.
replace_once(
    "desktop/src-tauri/src/platform/capabilities.rs",
    "use silent_disco_core::runtime::{CapabilitySnapshot, CoreActorHandle, PlatformEvent};",
    "use silent_disco_core::runtime::{CapabilitySnapshot, CoreActorHandle, CoreSnapshot, PlatformEvent};",
)
replace_once(
    "desktop/src-tauri/src/platform/capabilities.rs",
    "pub(crate) fn publish_desktop_capabilities(handle: &CoreActorHandle) -> Result<(), CoreError> {",
    """pub(crate) fn publish_desktop_capabilities(
    handle: &CoreActorHandle,
) -> Result<CoreSnapshot, CoreError> {""",
)
replace_once(
    "desktop/src-tauri/src/platform/capabilities.rs",
    """        if handle.current_snapshot()?.capabilities == expected {
            return Ok(());
        }""",
    """        let snapshot = handle.current_snapshot()?;
        if snapshot.capabilities == expected {
            return Ok(snapshot);
        }""",
)
replace_once(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    """    pub(crate) fn start(
        inbox: DesktopPlatformEffectInbox,
        dispatcher: DesktopPlatformEffectDispatcher,
        handle: CoreActorHandle,
        paths: DesktopProfilePaths,
    ) -> Result<Self, CoreError> {""",
    """    pub(crate) fn start(
        inbox: DesktopPlatformEffectInbox,
        dispatcher: DesktopPlatformEffectDispatcher,
        handle: CoreActorHandle,
        paths: DesktopProfilePaths,
    ) -> Result<(Self, CoreSnapshot), CoreError> {""",
)
replace_once(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    """        if let Err(primary) = publish_desktop_capabilities(&handle) {
            return match runner.shutdown() {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(append_startup_cleanup(primary, cleanup)),
            };
        }
        Ok(runner)""",
    """        let snapshot = match publish_desktop_capabilities(&handle) {
            Ok(snapshot) => snapshot,
            Err(primary) => {
                return match runner.shutdown() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(append_startup_cleanup(primary, cleanup)),
                };
            }
        };
        Ok((runner, snapshot))""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """    let platform_runner = match DesktopPlatformEffectRunner::start(
        platform_inbox,
        platform_dispatcher,
        handle.clone(),
        paths.clone(),
    ) {
        Ok(runner) => runner,""",
    """    let (platform_runner, current_snapshot) = match DesktopPlatformEffectRunner::start(
        platform_inbox,
        platform_dispatcher,
        handle.clone(),
        paths.clone(),
    ) {
        Ok(started) => started,""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """        assert_eq!(response.snapshot.revision, "0");
        assert_eq!(state.current_snapshot().expect("snapshot").revision, "0");""",
    """        assert_eq!(response.snapshot.revision, "1");
        assert!(response.snapshot.capabilities.audio_source_selection_available);
        assert!(response.snapshot.capabilities.secure_store_available);
        assert!(!response.snapshot.capabilities.audio_output_available);
        assert!(!response.snapshot.capabilities.local_network_available);
        let current = state.current_snapshot().expect("snapshot");
        assert_eq!(current.revision, response.snapshot.revision);
        assert_eq!(current.capabilities, response.snapshot.capabilities);""",
)
replace_once(
    "desktop/src-tauri/src/platform/capability_tests.rs",
    """    publish_desktop_capabilities(&handle).expect("publish desktop capabilities");

    assert_eq!(
        handle
            .current_snapshot()
            .expect("authoritative snapshot")
            .capabilities,
        desktop_capabilities()
    );""",
    """    let published =
        publish_desktop_capabilities(&handle).expect("publish desktop capabilities");

    assert_eq!(published.capabilities, desktop_capabilities());
    assert_eq!(
        handle.current_snapshot().expect("authoritative snapshot"),
        published
    );""",
)

# Scope the retained native source path to the active profile lifecycle.
replace_once(
    "desktop/src-tauri/src/platform/file_picker.rs",
    """        Ok(selected.replace(source))
    }

    pub(crate) fn restore_if_current(""",
    """        Ok(selected.replace(source))
    }

    pub(crate) fn clear(&self) -> Result<Option<InspectedAudioSource>, DesktopErrorDto> {
        let mut selected = self.selected.lock().map_err(|_| registry_error())?;
        Ok(selected.take())
    }

    pub(crate) fn restore_if_current(""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker.rs",
    """    fn pick_file(&self) -> Result<Option<PathBuf>, DesktopErrorDto> {
        let selected = self""",
    """    fn pick_file(&self) -> Result<Option<PathBuf>, DesktopErrorDto> {
        // The pinned plugin reports native close/cancel as `None` and exposes no separate picker
        // backend-error channel. Path conversion and every post-selection failure remain explicit.
        let selected = self""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    "use crate::platform::identity::{",
    """use crate::platform::file_picker::SelectedSourceRegistry;
use crate::platform::identity::{""",
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    """    app.state::<DesktopAppState>().finish_close(result)?;
    Ok(BridgeLifecycleDto::Closed)
}

fn open_runtime(""",
    """    let registry_cleanup = app
        .state::<SelectedSourceRegistry>()
        .clear()
        .map(|_| ());
    app.state::<DesktopAppState>()
        .finish_close(merge_close_results(result, registry_cleanup))?;
    Ok(BridgeLifecycleDto::Closed)
}

fn merge_close_results(
    primary: Result<(), DesktopErrorDto>,
    registry_cleanup: Result<(), DesktopErrorDto>,
) -> Result<(), DesktopErrorDto> {
    match (primary, registry_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(cleanup)) => Err(cleanup),
        (Err(primary), Err(cleanup)) => Err(append_cleanup(primary, Some(cleanup))),
    }
}

fn open_runtime(""",
)

# Strengthen the secure-selection regression suite.
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    "use crate::dto::DesktopErrorDto;\n",
    """use crate::dto::DesktopErrorDto;
use silent_disco_core::runtime::MAX_AUDIO_SOURCE_DISPLAY_NAME_BYTES;
""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    """    fn open(&self, path: &Path) -> io::Result<OpenedAudioFile> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        Ok(OpenedAudioFile::new(
            Box::new(file),
            metadata.file_type().is_file(),
            metadata.len(),
        ))
    }""",
    """    fn open(&self, path: &Path) -> io::Result<OpenedAudioFile> {
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
    }""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    "struct SyntheticBoundary {",
    """struct CanonicalizeFailingBoundary(io::ErrorKind);

impl AudioFileBoundary for CanonicalizeFailingBoundary {
    fn open(&self, _path: &Path) -> io::Result<OpenedAudioFile> {
        panic!("open must not run after canonicalization failure");
    }

    fn canonicalize(&self, _path: &Path) -> io::Result<PathBuf> {
        Err(io::Error::from(self.0))
    }
}

struct SyntheticBoundary {""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    """    let error = select_and_inspect(
        &FixedDialog(Ok(Some(root.0.clone()))),
        &SyntheticBoundary {
            regular: false,
            byte_length: 4096,
            bytes: Vec::new(),
        },
    )""",
    """    let error = select_and_inspect(
        &FixedDialog(Ok(Some(root.0.clone()))),
        &SystemBoundary,
    )""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    """#[test]
fn oversized_file_is_rejected_before_reading_payload() {""",
    """#[test]
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
fn oversized_file_is_rejected_before_reading_payload() {""",
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    """#[test]
fn deceptive_extension_is_rejected_by_content_signature() {""",
    r'''#[test]
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
    assert!(source
        .descriptor()
        .display_name
        .is_char_boundary(source.descriptor().display_name.len()));
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
    assert_ne!(first.descriptor().source_id, different.descriptor().source_id);
}

#[test]
fn deceptive_extension_is_rejected_by_content_signature() {''',
)
replace_once(
    "desktop/src-tauri/src/platform/file_picker_tests.rs",
    """    assert!(
        registry
            .resolve(&second_id)
            .expect("resolve second")
            .is_none()
    );
}""",
    """    assert!(
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
}""",
)
