use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::notification_channel::TauriNotificationSink;
use crate::platform::identity::{
    DesktopIdentity, DesktopIdentityProvider, SystemDesktopIdentityProvider,
};
use crate::platform::paths::{DesktopProfilePaths, resolve_profile_paths};
use crate::platform::profile_lock::ProfileLease;
use crate::profile::ProfileId;
use crate::runtime_dto::{
    AttachNotificationResponse, CoreNotificationDto, CoreSnapshotDto, OpenProfileRequest,
    OpenProfileResponse,
};
use crate::shutdown::{
    DesktopOwnedResources, cleanup_lease, cleanup_with_actor, cleanup_without_actor,
    shutdown_owned_resources,
};
use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreObserver,
};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

const INITIAL_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);

pub struct DesktopAppState {
    runtime: Mutex<DesktopRuntimeState>,
}

enum DesktopRuntimeState {
    Closed,
    Opening { profile_id: ProfileId },
    Ready(Box<ReadyRuntime>),
    Closing,
    Failed(DesktopErrorDto),
}

struct ReadyRuntime {
    profile_id: ProfileId,
    _identity: DesktopIdentity,
    handle: CoreActorHandle,
    notifications: Arc<DesktopNotificationBuffer>,
    owned: DesktopOwnedResources,
}

impl Default for DesktopAppState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(DesktopRuntimeState::Closed),
        }
    }
}

impl DesktopAppState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn begin_open(&self, profile_id: &ProfileId) -> Result<(), DesktopErrorDto> {
        let mut state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Closed => {
                *state = DesktopRuntimeState::Opening {
                    profile_id: profile_id.clone(),
                };
                Ok(())
            }
            DesktopRuntimeState::Opening { .. }
            | DesktopRuntimeState::Ready(_)
            | DesktopRuntimeState::Closing => Err(DesktopErrorDto::new(
                "desktop.profile.already_open",
                "runtime",
                "error",
                false,
                "a desktop profile is already open or changing lifecycle state",
            )),
            DesktopRuntimeState::Failed(error) => Err(DesktopErrorDto::new(
                "desktop.profile.failed_state",
                "runtime",
                "error",
                error.retryable,
                "the previous profile open failed; close the failed state before retrying",
            )),
        }
    }

    fn fail_open(&self, error: DesktopErrorDto) -> Result<(), DesktopErrorDto> {
        let mut state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        *state = DesktopRuntimeState::Failed(error);
        Ok(())
    }

    fn install_ready(
        &self,
        ready: ReadyRuntime,
        snapshot: CoreSnapshotDto,
    ) -> Result<OpenProfileResponse, Box<(DesktopErrorDto, ReadyRuntime)>> {
        let Ok(mut state) = self.runtime.lock() else {
            return Err(Box::new((poisoned_state_error(), ready)));
        };
        match &*state {
            DesktopRuntimeState::Opening { profile_id } if profile_id == &ready.profile_id => {
                let response = OpenProfileResponse {
                    lifecycle: BridgeLifecycleDto::Ready {
                        profile_id: ready.profile_id.as_str().to_owned(),
                    },
                    core_version: CoreVersionDto::from(silent_disco_core::core_version()),
                    snapshot,
                };
                *state = DesktopRuntimeState::Ready(Box::new(ready));
                Ok(response)
            }
            _ => Err(Box::new((
                DesktopErrorDto::new(
                    "desktop.profile.state_changed",
                    "runtime",
                    "fatal",
                    false,
                    "desktop profile lifecycle changed during startup",
                ),
                ready,
            ))),
        }
    }

    fn current_snapshot(&self) -> Result<CoreSnapshotDto, DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => {
                if let Some(error) = ready
                    .notifications
                    .delivery_failure()
                    .map_err(DesktopErrorDto::from)?
                {
                    return Err(DesktopErrorDto::from(error));
                }
                ready
                    .handle
                    .current_snapshot()
                    .map(CoreSnapshotDto::from)
                    .map_err(DesktopErrorDto::from)
            }
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

    fn notification_buffer(&self) -> Result<Arc<DesktopNotificationBuffer>, DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => Ok(Arc::clone(&ready.notifications)),
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

    fn take_for_close(&self) -> Result<CloseAction, DesktopErrorDto> {
        let mut state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match std::mem::replace(&mut *state, DesktopRuntimeState::Closing) {
            DesktopRuntimeState::Closed | DesktopRuntimeState::Failed(_) => {
                *state = DesktopRuntimeState::Closed;
                Ok(CloseAction::AlreadyClosed)
            }
            DesktopRuntimeState::Ready(ready) => Ok(CloseAction::Shutdown(ready)),
            DesktopRuntimeState::Opening { profile_id } => {
                *state = DesktopRuntimeState::Opening { profile_id };
                Err(DesktopErrorDto::new(
                    "desktop.profile.open_in_progress",
                    "runtime",
                    "error",
                    true,
                    "desktop profile open is still in progress",
                ))
            }
            DesktopRuntimeState::Closing => {
                *state = DesktopRuntimeState::Closing;
                Err(DesktopErrorDto::new(
                    "desktop.profile.close_in_progress",
                    "runtime",
                    "error",
                    true,
                    "desktop profile close is already in progress",
                ))
            }
        }
    }

    fn finish_close(&self, result: Result<(), DesktopErrorDto>) -> Result<(), DesktopErrorDto> {
        let mut state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match result {
            Ok(()) => {
                *state = DesktopRuntimeState::Closed;
                Ok(())
            }
            Err(error) => {
                *state = DesktopRuntimeState::Failed(error.clone());
                Err(error)
            }
        }
    }

    #[cfg(test)]
    fn open_profile_sync(
        &self,
        paths: &DesktopProfilePaths,
        profile_id: ProfileId,
        provider: &dyn DesktopIdentityProvider,
        notifications: Arc<DesktopNotificationBuffer>,
    ) -> Result<OpenProfileResponse, DesktopErrorDto> {
        self.begin_open(&profile_id)?;
        match open_runtime(paths, profile_id, provider, notifications) {
            Ok((ready, snapshot)) => match self.install_ready(ready, snapshot) {
                Ok(response) => Ok(response),
                Err(boxed) => {
                    let (primary, ready) = *boxed;
                    let cleanup = shutdown_owned_resources(ready.owned);
                    let error = append_cleanup(primary, cleanup.err());
                    if let Err(state_error) = self.fail_open(error.clone()) {
                        return Err(append_cleanup(error, Some(state_error)));
                    }
                    Err(error)
                }
            },
            Err(error) => {
                self.fail_open(error.clone())?;
                Err(error)
            }
        }
    }

    #[cfg(test)]
    fn close_sync(&self) -> Result<(), DesktopErrorDto> {
        match self.take_for_close()? {
            CloseAction::AlreadyClosed => Ok(()),
            CloseAction::Shutdown(ready) => {
                self.finish_close(shutdown_owned_resources(ready.owned))
            }
        }
    }
}

enum CloseAction {
    AlreadyClosed,
    Shutdown(Box<ReadyRuntime>),
}

/// Opens one production profile after lock, identity, storage, actor, and bridge startup.
///
/// # Errors
///
/// Returns a structured desktop error for invalid profile IDs, path or lock failures,
/// unavailable secure identity, storage or actor startup failure, bridge failure, or
/// lifecycle races. Partial startup is cleaned up before the error is returned.
#[tauri::command]
pub async fn open_profile(
    app: AppHandle,
    request: OpenProfileRequest,
) -> Result<OpenProfileResponse, DesktopErrorDto> {
    let profile_id = ProfileId::parse(&request.profile_id).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.profile.invalid_id",
            "validation",
            "error",
            false,
            &error.to_string(),
        )
    })?;

    let state = app.state::<DesktopAppState>();
    state.begin_open(&profile_id)?;
    let paths = match resolve_profile_paths(&app, &profile_id) {
        Ok(paths) => paths,
        Err(error) => {
            let failure = DesktopErrorDto::new(
                "desktop.profile.path_failed",
                "platform",
                "fatal",
                false,
                &error.to_string(),
            );
            state.fail_open(failure.clone())?;
            return Err(failure);
        }
    };

    let notifications = Arc::new(DesktopNotificationBuffer::new());
    let provider = SystemDesktopIdentityProvider;
    let task = tauri::async_runtime::spawn_blocking(move || {
        open_runtime(&paths, profile_id, &provider, notifications)
    });
    let result = task.await.map_err(|error| {
        DesktopErrorDto::new(
            "desktop.profile.open_worker_failed",
            "runtime",
            "fatal",
            false,
            &format!("desktop profile open worker failed: {error}"),
        )
    });

    let state = app.state::<DesktopAppState>();
    match result {
        Ok(Ok((ready, snapshot))) => match state.install_ready(ready, snapshot) {
            Ok(response) => Ok(response),
            Err(boxed) => {
                let (primary, ready) = *boxed;
                let cleanup = shutdown_owned_resources(ready.owned);
                let error = append_cleanup(primary, cleanup.err());
                if let Err(state_error) = state.fail_open(error.clone()) {
                    return Err(append_cleanup(error, Some(state_error)));
                }
                Err(error)
            }
        },
        Ok(Err(error)) | Err(error) => {
            state.fail_open(error.clone())?;
            Err(error)
        }
    }
}

/// Returns the latest authoritative Rust snapshot for the open profile.
///
/// # Errors
///
/// Returns a structured error when no profile is ready or when the actor/bridge failed.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command injection supplies AppHandle by value"
)]
#[tauri::command]
pub fn get_current_snapshot(app: AppHandle) -> Result<CoreSnapshotDto, DesktopErrorDto> {
    app.state::<DesktopAppState>().current_snapshot()
}

/// Attaches or replaces the frontend notification channel for the ready profile.
///
/// The current authoritative snapshot is dispatched first. Replacing a subscription stops
/// and joins the old worker before the new subscription becomes active.
///
/// # Errors
///
/// Returns a structured error when no profile is ready, the bridge has failed, the worker
/// cannot start, or the blocking attachment task fails.
#[tauri::command]
pub async fn attach_notifications(
    app: AppHandle,
    channel: Channel<CoreNotificationDto>,
) -> Result<AttachNotificationResponse, DesktopErrorDto> {
    let notifications = app.state::<DesktopAppState>().notification_buffer()?;
    let task = tauri::async_runtime::spawn_blocking(move || {
        notifications.attach_sink(Arc::new(TauriNotificationSink::new(channel)))
    });
    let subscription = task.await.map_err(|error| {
        DesktopErrorDto::new(
            "desktop.bridge.attach_worker_failed",
            "runtime",
            "fatal",
            false,
            &format!("desktop notification attachment worker failed: {error}"),
        )
    })??;
    Ok(AttachNotificationResponse {
        subscription_id: subscription.get().to_string(),
    })
}

/// Closes the active profile in reverse startup order.
///
/// # Errors
///
/// Returns a structured error when lifecycle state is unavailable, another open/close
/// is in progress, the close worker fails, or actor/database/profile-lock cleanup fails.
#[tauri::command]
pub async fn close_profile(app: AppHandle) -> Result<BridgeLifecycleDto, DesktopErrorDto> {
    let action = app.state::<DesktopAppState>().take_for_close()?;
    let result = match action {
        CloseAction::AlreadyClosed => Ok(()),
        CloseAction::Shutdown(ready) => {
            tauri::async_runtime::spawn_blocking(move || shutdown_owned_resources(ready.owned))
                .await
                .map_err(|error| {
                    DesktopErrorDto::new(
                        "desktop.profile.close_worker_failed",
                        "runtime",
                        "fatal",
                        false,
                        &format!("desktop profile close worker failed: {error}"),
                    )
                })?
        }
    };
    app.state::<DesktopAppState>().finish_close(result)?;
    Ok(BridgeLifecycleDto::Closed)
}

fn open_runtime(
    paths: &DesktopProfilePaths,
    profile_id: ProfileId,
    provider: &dyn DesktopIdentityProvider,
    notifications: Arc<DesktopNotificationBuffer>,
) -> Result<(ReadyRuntime, CoreSnapshotDto), DesktopErrorDto> {
    let lease = ProfileLease::acquire(paths, &profile_id).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.profile.lock_failed",
            "storage",
            "fatal",
            false,
            &error.to_string(),
        )
    })?;

    let identity = match provider.load_or_create(&profile_id) {
        Ok(identity) => identity,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.identity.unavailable",
                "platform",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };

    let database_config = match DatabaseConfig::new(paths.domain_database()) {
        Ok(config) => config,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.storage.configure_failed",
                "storage",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };
    let database = match DatabaseWorker::start(database_config) {
        Ok(database) => database,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.storage.open_failed",
                "storage",
                "fatal",
                false,
                &error.to_string(),
            );
            return Err(cleanup_lease(lease, primary));
        }
    };

    let observer_buffer = Arc::clone(&notifications);
    let observer = move |notification| observer_buffer.on_notification(notification);
    let actor =
        match CoreActorRuntime::start(CoreActorConfig::new(identity.device_id().clone()), observer)
        {
            Ok(actor) => actor,
            Err(error) => {
                let primary = DesktopErrorDto::from(error);
                return Err(cleanup_without_actor(database, lease, primary));
            }
        };
    let handle = actor.handle();

    let delivered_snapshot = match notifications.wait_for_initial_snapshot(INITIAL_SNAPSHOT_TIMEOUT)
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };
    let current_snapshot = match handle.current_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };
    if current_snapshot.revision != delivered_snapshot.revision {
        let primary = DesktopErrorDto::new(
            "desktop.bridge.initial_snapshot_mismatch",
            "runtime",
            "fatal",
            false,
            "initial delivered snapshot does not match the actor snapshot cache",
        );
        return Err(cleanup_with_actor(actor, database, lease, primary));
    }

    let snapshot = CoreSnapshotDto::from(current_snapshot);
    Ok((
        ReadyRuntime {
            profile_id,
            _identity: identity,
            handle,
            notifications: Arc::clone(&notifications),
            owned: DesktopOwnedResources {
                notifications,
                actor,
                database,
                lease,
            },
        },
        snapshot,
    ))
}

fn append_cleanup(primary: DesktopErrorDto, cleanup: Option<DesktopErrorDto>) -> DesktopErrorDto {
    let Some(cleanup) = cleanup else {
        return primary;
    };
    let cleanup_message = cleanup.message;
    DesktopErrorDto::new(
        &primary.code,
        &primary.subsystem,
        &primary.severity,
        primary.retryable,
        &format!("{}; {cleanup_message}", primary.message),
    )
}

fn poisoned_state_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.bridge.state_poisoned",
        "runtime",
        "fatal",
        false,
        "desktop application state mutex was poisoned",
    )
}

#[cfg(test)]
mod tests {
    use super::DesktopAppState;
    use crate::notification_buffer::DesktopNotificationBuffer;
    use crate::platform::identity::{
        DesktopIdentity, DesktopIdentityError, DesktopIdentityProvider,
    };
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

        assert_eq!(response.snapshot.revision, "0");
        assert_eq!(state.current_snapshot().expect("snapshot").revision, "0");
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
}
