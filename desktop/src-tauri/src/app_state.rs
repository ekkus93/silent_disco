use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};
use crate::host_session_dto::HostSessionSnapshotDto;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::notification_channel::TauriNotificationSink;
use crate::platform::effect_runner::{
    DesktopCoreObserver, DesktopPlatformEffectDispatcher, DesktopPlatformEffectRunner,
};
use crate::platform::file_picker::SelectedSourceRegistry;
use crate::platform::identity::{
    DesktopIdentity, DesktopIdentityProvider, SystemDesktopIdentityProvider,
};
use crate::platform::network::DesktopHostNetworkControl;
use crate::platform::network_dto::{NetworkInterfaceSnapshotDto, SetNetworkBindPreferenceRequest};
use crate::platform::paths::{DesktopProfilePaths, resolve_profile_paths};
use crate::platform::profile_lock::ProfileLease;
use crate::platform::source_staging::cleanup_incomplete_sources;
use crate::platform::source_staging_control::SourceStagingControl;
use crate::platform::storage_effect_runner::{
    DesktopStorageEffectDispatcher, DesktopStorageEffectRunner,
};
use crate::profile::ProfileId;
use crate::runtime_dto::{
    AttachNotificationResponse, CommandReceiptDto, CoreNotificationDto, CoreSnapshotDto,
    OpenProfileRequest, OpenProfileResponse,
};
use crate::shutdown::{
    DesktopOwnedResources, cleanup_lease, cleanup_with_actor, cleanup_without_actor,
    shutdown_owned_resources,
};
use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand, CoreCommandRequest,
    SnapshotRevision,
};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use std::path::PathBuf;
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
    sources: PathBuf,
    _identity: DesktopIdentity,
    handle: CoreActorHandle,
    notifications: Arc<DesktopNotificationBuffer>,
    network: Arc<DesktopHostNetworkControl>,
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

    pub(crate) fn source_staging_directory(&self) -> Result<PathBuf, DesktopErrorDto> {
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

    pub(crate) fn host_session_snapshot(&self) -> Result<HostSessionSnapshotDto, DesktopErrorDto> {
        let (handle, network) = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => {
                    (ready.handle.clone(), Arc::clone(&ready.network))
                }
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        let snapshot = handle.current_snapshot().map_err(DesktopErrorDto::from)?;
        let active = network.active_host_session()?;
        Ok(HostSessionSnapshotDto::from_parts(
            &snapshot,
            active.as_ref(),
        ))
    }

    pub(crate) fn host_network_snapshot(
        &self,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let network = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        network.snapshot()
    }

    pub(crate) fn start_host_playback(
        &self,
        registry: &crate::platform::file_picker::SelectedSourceRegistry,
    ) -> Result<(), DesktopErrorDto> {
        let (handle, network) = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => {
                    (ready.handle.clone(), Arc::clone(&ready.network))
                }
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        crate::platform::start_playback::start(&handle, &network, registry)
    }

    pub(crate) fn pause_host_playback(&self) -> Result<(), DesktopErrorDto> {
        let network = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        network.pause_playback()
    }

    pub(crate) fn resume_host_playback(&self) -> Result<(), DesktopErrorDto> {
        let network = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        network.resume_playback()
    }

    pub(crate) fn stop_host_playback(&self) -> Result<(), DesktopErrorDto> {
        let network = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        network.stop_playback()
    }

    pub(crate) fn set_host_network_preference(
        &self,
        request: &SetNetworkBindPreferenceRequest,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let network = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) => return Err(error.clone()),
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => {
                    return Err(DesktopErrorDto::new(
                        "desktop.profile.not_ready",
                        "runtime",
                        "error",
                        true,
                        "no desktop profile is ready",
                    ));
                }
            }
        };
        network.set_preference(request)
    }

    pub(crate) fn submit_core_command(
        &self,
        expected_revision: SnapshotRevision,
        command: CoreCommand,
    ) -> Result<CommandReceiptDto, DesktopErrorDto> {
        let request = CoreCommandRequest::new(expected_revision, command).map_err(|error| {
            DesktopErrorDto::new(
                "desktop.command.invalid_payload",
                "validation",
                "error",
                false,
                &error.to_string(),
            )
        })?;
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => ready
                .handle
                .submit_command(request)
                .map(CommandReceiptDto::from)
                .map_err(DesktopErrorDto::from),
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
/// Returns a structured error for profile, path, lock, identity, storage, actor, bridge, or lifecycle failure; partial startup is cleaned up before returning.
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
/// The current snapshot is dispatched first; replacing a subscription stops and joins the old worker before the new subscription becomes active.
///
/// # Errors
///
/// Returns a structured error when no profile is ready or bridge, worker start, or blocking attachment fails.
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
/// Returns a structured error for lifecycle, concurrent open/close, worker, actor, database, or profile-lock cleanup failure.
#[tauri::command]
pub async fn close_profile(app: AppHandle) -> Result<BridgeLifecycleDto, DesktopErrorDto> {
    let action = app.state::<DesktopAppState>().take_for_close()?;
    let staging_cleanup = app.state::<SourceStagingControl>().cancel_and_wait();
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
    let registry_cleanup = app.state::<SelectedSourceRegistry>().clear().map(|_| ());
    let result = merge_close_results(result, staging_cleanup);
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

    if let Err(primary) = cleanup_incomplete_sources(paths.sources()) {
        return Err(cleanup_lease(lease, primary));
    }

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

    let network = Arc::new(DesktopHostNetworkControl::production());
    let (platform_dispatcher, platform_inbox) = DesktopPlatformEffectDispatcher::channel();
    let (storage_dispatcher, storage_inbox) = DesktopStorageEffectDispatcher::channel();
    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
        Arc::clone(&network),
        storage_dispatcher.clone(),
    );
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

    let storage_runner = match DesktopStorageEffectRunner::start(
        storage_inbox,
        storage_dispatcher,
        Arc::new(handle.clone()),
        database.client(),
    ) {
        Ok(runner) => runner,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };

    let (platform_runner, current_snapshot) = match DesktopPlatformEffectRunner::start(
        platform_inbox,
        platform_dispatcher,
        handle.clone(),
        paths.clone(),
        Arc::clone(&network),
    ) {
        Ok(started) => started,
        Err(error) => {
            let mut primary = DesktopErrorDto::from(error);
            if let Err(storage_error) = storage_runner.shutdown() {
                primary = append_cleanup(primary, Some(DesktopErrorDto::from(storage_error)));
            }
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };

    let snapshot = CoreSnapshotDto::from(current_snapshot);
    Ok((
        ReadyRuntime {
            profile_id,
            sources: paths.sources().to_path_buf(),
            _identity: identity,
            handle,
            notifications: Arc::clone(&notifications),
            network,
            owned: DesktopOwnedResources {
                platform_runner,
                storage_runner,
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
#[path = "app_state_tests.rs"]
mod tests;
