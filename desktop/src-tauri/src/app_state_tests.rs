use super::DesktopAppState;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::identity::{DesktopIdentity, DesktopIdentityError, DesktopIdentityProvider};
use crate::platform::paths::DesktopProfilePaths;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use crate::profile::ProfileId;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-desktop-app-state-{}-{sequence}",
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
        if let Err(error) = fs::remove_dir_all(&self.0) {
            assert!(
                error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                "failed to remove test directory: {error}"
            );
        }
    }
}

struct FixedIdentityProvider([u8; 32]);

impl DesktopIdentityProvider for FixedIdentityProvider {
    fn load_or_create(
        &self,
        _profile_id: &ProfileId,
    ) -> Result<DesktopIdentity, DesktopIdentityError> {
        DesktopIdentity::from_secret(&self.0)
    }
}

fn profile(root: &TestDirectory) -> (ProfileId, DesktopProfilePaths) {
    let id = ProfileId::parse("main").expect("valid profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &id)
        .expect("valid profile paths");
    (id, paths)
}

#[test]
fn opens_real_storage_actor_and_snapshot_then_shuts_down_idempotently() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    let state = DesktopAppState::new();
    let response = state
        .open_profile_sync(
            &paths,
            id,
            &FixedIdentityProvider([9; 32]),
            Arc::new(DesktopNotificationBuffer::new()),
        )
        .expect("open profile");

    assert_eq!(response.snapshot.revision, "1");
    assert!(
        response
            .snapshot
            .capabilities
            .audio_source_selection_available
    );
    assert!(response.snapshot.capabilities.secure_store_available);
    assert!(response.snapshot.capabilities.local_network_available);
    assert!(!response.snapshot.capabilities.audio_output_available);
    let current = state.current_snapshot().expect("snapshot");
    assert_eq!(current.revision, response.snapshot.revision);
    assert_eq!(current.capabilities, response.snapshot.capabilities);
    state.close_sync().expect("first close");
    state.close_sync().expect("idempotent second close");
}

#[test]
fn second_open_is_rejected_and_profile_lock_is_retained_until_close() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    let state = DesktopAppState::new();
    state
        .open_profile_sync(
            &paths,
            id.clone(),
            &FixedIdentityProvider([4; 32]),
            Arc::new(DesktopNotificationBuffer::new()),
        )
        .expect("open profile");

    assert!(matches!(
        ProfileLease::acquire(&paths, &id),
        Err(ProfileLockError::ProfileInUse)
    ));
    assert!(
        state
            .open_profile_sync(
                &paths,
                id.clone(),
                &FixedIdentityProvider([4; 32]),
                Arc::new(DesktopNotificationBuffer::new()),
            )
            .is_err()
    );

    state.close_sync().expect("close");
    ProfileLease::acquire(&paths, &id)
        .expect("lock released")
        .release()
        .expect("release verification lease");
}

#[test]
fn storage_failure_releases_profile_lock_without_fallback() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    paths.prepare_directories().expect("prepare paths");
    fs::create_dir(paths.domain_database()).expect("invalid database directory");
    let state = DesktopAppState::new();

    assert!(
        state
            .open_profile_sync(
                &paths,
                id.clone(),
                &FixedIdentityProvider([5; 32]),
                Arc::new(DesktopNotificationBuffer::new()),
            )
            .is_err()
    );
    state.close_sync().expect("clear failed state");
    ProfileLease::acquire(&paths, &id)
        .expect("lock released after storage failure")
        .release()
        .expect("release verification lease");
    assert!(!paths.root().join("fallback.sqlite3").exists());
}

#[test]
fn observer_setup_failure_releases_actor_database_and_lock() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    let state = DesktopAppState::new();

    assert!(
        state
            .open_profile_sync(
                &paths,
                id.clone(),
                &FixedIdentityProvider([6; 32]),
                Arc::new(DesktopNotificationBuffer::failing_initial_notification()),
            )
            .is_err()
    );
    state.close_sync().expect("clear failed state");
    ProfileLease::acquire(&paths, &id)
        .expect("lock released after observer failure")
        .release()
        .expect("release verification lease");
}
