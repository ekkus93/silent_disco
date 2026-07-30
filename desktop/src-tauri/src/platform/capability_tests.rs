use super::effect_runner::{DesktopPlatformAdapters, DesktopPlatformEffectExecutor};
use super::paths::DesktopProfilePaths;
use crate::profile::ProfileId;
use silent_disco_core::domain::OperationId;
use silent_disco_core::runtime::{
    PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformOperationCompletion,
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
fn production_adapter_advertises_only_implemented_block16_audio_selection() {
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
    assert!(capabilities.audio_source_selection_available);
    assert!(capabilities.secure_store_available);
    assert!(!capabilities.audio_output_available);
    assert!(!capabilities.local_network_available);
}
