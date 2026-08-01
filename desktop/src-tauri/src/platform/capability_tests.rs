use super::capabilities::{desktop_capabilities, publish_desktop_capabilities};
use super::effect_runner::{DesktopPlatformAdapters, DesktopPlatformEffectExecutor};
use super::paths::DesktopProfilePaths;
use crate::profile::ProfileId;
use silent_disco_core::domain::{DeviceId, OperationId};
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorRuntime, PermissionCapability, PlatformEffect, PlatformEffectRequest,
    PlatformOperationCompletion,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-block16-capabilities-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale capability fixture");
        }
        fs::create_dir_all(&path).expect("create capability fixture");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            assert!(
                error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                "failed to remove capability fixture: {error}"
            );
        }
    }
}

#[test]
fn production_adapter_advertises_only_implemented_capabilities() {
    let root = TestDirectory::new();
    let profile_id = ProfileId::parse("main").expect("valid profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("valid profile paths");
    paths.prepare_directories().expect("prepare profile paths");
    let adapter = DesktopPlatformAdapters::new(paths);
    let effect = PlatformEffect::new(
        OperationId::new("block16-capabilities").expect("valid operation ID"),
        PlatformEffectRequest::RequestCapabilities(vec![
            PermissionCapability::AudioSourceSelection,
            PermissionCapability::AudioOutput,
            PermissionCapability::LocalNetwork,
            PermissionCapability::SecureStore,
        ]),
    )
    .expect("valid capability request");

    let completion = adapter
        .execute(&effect, None)
        .expect("capability resolution succeeds");
    let PlatformOperationCompletion::CapabilitiesResolved(capabilities) = completion else {
        panic!("unexpected capability completion");
    };
    assert_eq!(capabilities, desktop_capabilities());
    assert!(capabilities.audio_source_selection_available);
    assert!(capabilities.secure_store_available);
    assert!(capabilities.local_network_available);
    assert!(!capabilities.audio_output_available);
}

#[test]
fn startup_publication_updates_the_authoritative_actor_snapshot() {
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-capability-test").expect("valid device ID")),
        |_notification| Ok::<(), CoreError>(()),
    )
    .expect("start actor");
    let handle = actor.handle();

    let published = publish_desktop_capabilities(&handle).expect("publish desktop capabilities");

    assert_eq!(published.capabilities, desktop_capabilities());
    assert_eq!(
        handle.current_snapshot().expect("authoritative snapshot"),
        published
    );
    actor.shutdown().expect("shutdown actor");
}
