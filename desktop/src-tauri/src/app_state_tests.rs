use super::{CloseAction, DesktopAppState};
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::identity::{DesktopIdentity, DesktopIdentityError, DesktopIdentityProvider};
use crate::platform::invitation_identity::{
    DesktopHostSigningIdentity, DesktopHostSigningIdentityError, DesktopHostSigningIdentityProvider,
};
use crate::platform::paths::DesktopProfilePaths;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use crate::profile::ProfileId;
use crate::shutdown::shutdown_owned_resources;
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

/// Never touches a real OS keyring -- these tests exercise profile open/close
/// lifecycle, not signing-identity storage (that lives in
/// `invitation_identity.rs`'s own test module).
struct FixedSigningIdentityProvider(u8);

impl DesktopHostSigningIdentityProvider for FixedSigningIdentityProvider {
    fn load_or_create(
        &self,
        _profile_id: &ProfileId,
    ) -> Result<DesktopHostSigningIdentity, DesktopHostSigningIdentityError> {
        Ok(crate::platform::invitation_identity::tests_support::fixed_identity_for_tests(self.0))
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
            &FixedSigningIdentityProvider(9),
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
            &FixedSigningIdentityProvider(4),
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
                &FixedSigningIdentityProvider(4),
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
                &FixedSigningIdentityProvider(5),
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

/// Block 35.4 "export after startup failure": diagnostics must surface the
/// real, stored startup failure -- the same `DesktopErrorDto` `open_profile`
/// itself returned -- not a fabricated generic error and not a false
/// success. There is no `ReadyRuntime` to query in this state, so an error
/// (not a synthesized partial snapshot) is the honest answer, matching
/// every other `DesktopAppState` accessor's behavior in `Failed` state.
#[test]
fn diagnostics_after_startup_failure_surfaces_the_stored_failure() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    paths.prepare_directories().expect("prepare paths");
    fs::create_dir(paths.domain_database()).expect("invalid database directory");
    let state = DesktopAppState::new();

    let open_error = state
        .open_profile_sync(
            &paths,
            id.clone(),
            &FixedIdentityProvider([7; 32]),
            &FixedSigningIdentityProvider(7),
            Arc::new(DesktopNotificationBuffer::new()),
        )
        .expect_err("storage failure");

    let diagnostics_error = state
        .host_diagnostics()
        .expect_err("diagnostics must surface the startup failure, not a false success");
    assert_eq!(diagnostics_error, open_error);

    state.close_sync().expect("clear failed state");
    ProfileLease::acquire(&paths, &id)
        .expect("lock released after startup failure")
        .release()
        .expect("release verification lease");
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
                &FixedSigningIdentityProvider(6),
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

/// Block 36.4 "shutdown during open": a close request arriving while a
/// profile is still opening must be reported, not silently accepted or
/// allowed to corrupt the in-flight open -- the `Opening` state survives
/// intact for the original open to finish or fail on its own.
#[test]
fn closing_while_still_opening_is_reported_and_leaves_the_opening_state_intact() {
    let root = TestDirectory::new();
    let (id, _paths) = profile(&root);
    let state = DesktopAppState::new();
    state.begin_open(&id).expect("begin open");

    let Err(error) = state.take_for_close() else {
        panic!("closing mid-open must be reported, not silently accepted");
    };
    assert_eq!(error.code, "desktop.profile.open_in_progress");

    // The Opening state survived the failed close attempt -- a fresh
    // begin_open is still rejected as already-open-or-changing, which
    // would not be true had the failed close reset the state to `Closed`.
    assert!(state.begin_open(&id).is_err());
}

/// Block 36.3 "duplicate close is idempotent": a second close request
/// arriving while the first is still tearing down the profile must not
/// attempt a second teardown and must not fail just because the caller
/// asked twice -- it is folded into the in-flight attempt's own outcome.
#[test]
fn a_duplicate_close_while_one_is_already_in_flight_is_idempotent() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    let state = DesktopAppState::new();
    state
        .open_profile_sync(
            &paths,
            id,
            &FixedIdentityProvider([9; 32]),
            &FixedSigningIdentityProvider(9),
            Arc::new(DesktopNotificationBuffer::new()),
        )
        .expect("open profile");

    // The first caller claims the real teardown...
    let first = state.take_for_close().expect("first close");
    assert!(matches!(first, CloseAction::Shutdown(_)));
    // ...and a second caller arriving before the first finishes observes
    // idempotent success, not an error, and never receives its own
    // `ReadyRuntime` to tear down again.
    let second = state.take_for_close().expect("second close");
    assert!(matches!(second, CloseAction::AlreadyInProgress));

    let CloseAction::Shutdown(ready) = first else {
        unreachable!("asserted Shutdown above");
    };
    state
        .finish_close(shutdown_owned_resources(ready.owned))
        .expect("finish what the first caller claimed");
}

/// Block 36.4 "profile can reopen after clean shutdown": a real, full
/// open -> close -> open cycle, not just the profile lock's own
/// acquire/release (already covered by
/// `second_open_is_rejected_and_profile_lock_is_retained_until_close`).
#[test]
fn a_profile_reopens_cleanly_after_a_full_close() {
    let root = TestDirectory::new();
    let (id, paths) = profile(&root);
    let state = DesktopAppState::new();

    let first_open = state
        .open_profile_sync(
            &paths,
            id.clone(),
            &FixedIdentityProvider([10; 32]),
            &FixedSigningIdentityProvider(10),
            Arc::new(DesktopNotificationBuffer::new()),
        )
        .expect("first open");
    assert_eq!(first_open.snapshot.revision, "1");
    state.close_sync().expect("close");

    let second_open = state
        .open_profile_sync(
            &paths,
            id,
            &FixedIdentityProvider([10; 32]),
            &FixedSigningIdentityProvider(10),
            Arc::new(DesktopNotificationBuffer::new()),
        )
        .expect("profile reopens cleanly after a full close");
    assert_eq!(second_open.snapshot.revision, "1");
    state.close_sync().expect("close again");
}
