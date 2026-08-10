use crate::diagnostics_dto::{DesktopDiagnosticsDto, StorageDiagnosticsDto};
use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};
use crate::host_session_dto::{HostInvitationDto, HostSessionSnapshotDto};
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::notification_channel::TauriNotificationSink;
use crate::platform::effect_runner::{
    DesktopCoreObserver, DesktopPlatformEffectDispatcher, DesktopPlatformEffectRunner,
};
use crate::platform::file_picker::SelectedSourceRegistry;
use crate::platform::identity::{
    DesktopIdentity, DesktopIdentityProvider, SystemDesktopIdentityProvider,
};
use crate::platform::invitation::{build_signed_invitation, current_wall_clock_ms};
use crate::platform::invitation_identity::{
    DesktopHostSigningIdentity, DesktopHostSigningIdentityProvider,
    SystemDesktopHostSigningIdentityProvider,
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
    Opening {
        profile_id: ProfileId,
    },
    Ready(Box<ReadyRuntime>),
    Closing,
    /// A profile failed to open. Retryable per the stored error's own
    /// `retryable` field -- a new open attempt after `close`/reset is
    /// generally safe, since nothing was ever fully constructed.
    Failed(DesktopErrorDto),
    /// A shutdown was attempted and did not complete cleanly (Block 36.1
    /// "shutdown failed") -- distinct from [`Self::Failed`], which means a
    /// profile never became ready. Deliberately never treated as
    /// reopen-safe (see `begin_open`): on a genuine timeout, owned
    /// resources may still be alive on a detached background thread (Block
    /// 36.2/36.3 "timeout does not free callback-visible memory unsafely"),
    /// so a fresh open could race a still-tearing-down profile. Recovery
    /// requires restarting the application.
    ShutdownFailed(DesktopErrorDto),
}

struct ReadyRuntime {
    profile_id: ProfileId,
    sources: PathBuf,
    identity: DesktopIdentity,
    signing_identity: DesktopHostSigningIdentity,
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
            DesktopRuntimeState::ShutdownFailed(_) => Err(DesktopErrorDto::new(
                "desktop.profile.shutdown_failed_state",
                "runtime",
                "fatal",
                false,
                "a previous shutdown did not complete cleanly; restart the application before opening a profile",
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
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                Err(error.clone())
            }
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
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                Err(error.clone())
            }
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
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
            network.monitor_status(),
        ))
    }

    /// Assembles the bounded, redacted diagnostics snapshot (Block 35.1),
    /// shared by the live diagnostics screen and the file export.
    ///
    /// Holds `self.runtime`'s lock for the whole gather, including the
    /// database worker's own metadata round trip -- acceptable for a
    /// deliberate, infrequent diagnostics query, unlike the hot playback-
    /// control paths that only ever clone cheap `Arc` handles out of this
    /// same lock.
    ///
    /// # Errors
    ///
    /// Returns a structured error only when no profile is ready or the
    /// actor's own snapshot cannot be read -- a failure to query storage or
    /// the notification buffer is folded into the returned DTO's own
    /// `storage`/`notificationBridge` fields instead, since a diagnostics
    /// request that only partially succeeds is still far more useful than
    /// one that fails outright.
    pub(crate) fn host_diagnostics(&self) -> Result<DesktopDiagnosticsDto, DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        let ready = match &*state {
            DesktopRuntimeState::Ready(ready) => ready,
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                return Err(error.clone());
            }
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
        };

        let core_snapshot = ready
            .handle
            .current_snapshot()
            .map_err(DesktopErrorDto::from)?;
        let active = ready.network.active_host_session()?;
        let stream_diagnostics = ready.network.stream_diagnostics_snapshot();
        let monitor = ready.network.monitor_status_full();
        let storage = match ready.owned.database.client().metadata() {
            Ok(metadata) => StorageDiagnosticsDto {
                available: true,
                schema_version: Some(metadata.schema_version),
                journal_mode: Some(metadata.journal_mode),
                foreign_keys_enabled: Some(metadata.foreign_keys_enabled),
                integrity_check: Some(metadata.integrity_check),
                applied_migration_count: Some(
                    u32::try_from(metadata.applied_migrations.len()).unwrap_or(u32::MAX),
                ),
                failure_reason: None,
            },
            Err(error) => StorageDiagnosticsDto {
                available: false,
                schema_version: None,
                journal_mode: None,
                foreign_keys_enabled: None,
                integrity_check: None,
                applied_migration_count: None,
                failure_reason: Some(error.to_string()),
            },
        };
        // A failure to read the notification buffer's own failure state is
        // itself only a diagnostics-gathering detail, not surfaced as a
        // distinct error here -- folded to `None`, same severity as "no
        // failure observed".
        let notification_failure = ready.notifications.delivery_failure().ok().flatten();
        let device_identity_present = !ready.identity.device_id().as_str().is_empty();
        let signing_key_fingerprint = Some(silent_disco_core::p2::public_key_fingerprint(
            ready.signing_identity.public_key_der(),
        ));

        Ok(crate::platform::diagnostics::build_diagnostics_snapshot(
            &core_snapshot,
            ready.profile_id.as_str(),
            device_identity_present,
            true,
            signing_key_fingerprint,
            active.as_ref(),
            stream_diagnostics.as_ref(),
            &monitor,
            storage,
            notification_failure,
            env!("CARGO_PKG_VERSION"),
            crate::platform::invitation::current_wall_clock_ms(),
        ))
    }

    pub(crate) fn host_network_snapshot(
        &self,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let network = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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

    /// Builds and signs one fresh invitation for the active host session
    /// (Block 31.1). Never returns a cached previous invitation -- every
    /// call generates a new nonce and a new 5-minute expiry window, so the
    /// frontend's explicit "refresh" action always produces something
    /// genuinely new, and there is no server-side "current invitation" a
    /// caller could be handed stale by mistake.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no profile is ready, no host
    /// endpoint is currently bound (an invitation naming an endpoint that
    /// does not exist would be worse than none), the signing identity is
    /// unavailable, or the session data does not fit the shared core's
    /// invitation bounds.
    pub(crate) fn create_host_invitation(&self) -> Result<HostInvitationDto, DesktopErrorDto> {
        let (handle, network, signing_identity) = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => (
                    ready.handle.clone(),
                    Arc::clone(&ready.network),
                    ready.signing_identity.clone(),
                ),
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
        let active = network.active_host_session()?.ok_or_else(|| {
            DesktopErrorDto::new(
                "desktop.invitation.no_active_endpoint",
                "validation",
                "error",
                true,
                "start a host session with a bound network endpoint before creating an invitation",
            )
        })?;
        let snapshot = handle.current_snapshot().map_err(DesktopErrorDto::from)?;
        build_signed_invitation(
            &active.advertisement,
            active.endpoint,
            &snapshot.host_draft,
            &signing_identity,
            current_wall_clock_ms(),
        )
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.invitation.build_failed",
                "validation",
                "error",
                false,
                &error.to_string(),
            )
        })
    }

    /// Sets the desktop host's local-monitor preference (Block 34.2
    /// "monitor enable is explicit"). Never fails on its own -- disabling
    /// always succeeds, and enabling only records a preference that takes
    /// effect on the next stream start; any failure to actually stand up a
    /// monitor stream is surfaced through `HostSessionSnapshotDto.monitor`
    /// on the next snapshot, not as an error from this call.
    pub(crate) fn set_monitor_enabled(&self, enabled: bool) -> Result<(), DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => {
                ready.network.set_monitor_enabled(enabled);
                Ok(())
            }
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                Err(error.clone())
            }
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
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
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
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                Err(error.clone())
            }
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
            DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                Err(error.clone())
            }
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
            DesktopRuntimeState::ShutdownFailed(error) => {
                // Deliberately restored as `ShutdownFailed`, not downgraded
                // to `Closed` -- there is nothing live left to close, but
                // `begin_open` must keep refusing new opens until the
                // application restarts (see the variant's own doc comment).
                *state = DesktopRuntimeState::ShutdownFailed(error);
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
                // Block 36.3 "duplicate close is idempotent": a second
                // close request while one is already tearing down is not a
                // failure -- it never attempts a second teardown and never
                // reports an error just because the caller asked twice.
                // The in-flight attempt owns the real outcome.
                *state = DesktopRuntimeState::Closing;
                Ok(CloseAction::AlreadyInProgress)
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
                *state = DesktopRuntimeState::ShutdownFailed(error.clone());
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
        signing_provider: &dyn DesktopHostSigningIdentityProvider,
        notifications: Arc<DesktopNotificationBuffer>,
    ) -> Result<OpenProfileResponse, DesktopErrorDto> {
        self.begin_open(&profile_id)?;
        match open_runtime(paths, profile_id, provider, signing_provider, notifications) {
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
            CloseAction::AlreadyClosed | CloseAction::AlreadyInProgress => Ok(()),
            CloseAction::Shutdown(ready) => {
                self.finish_close(shutdown_owned_resources(ready.owned))
            }
        }
    }
}

enum CloseAction {
    AlreadyClosed,
    /// Another close is already tearing this profile down; this caller
    /// does not own that teardown and must not attempt a second one (Block
    /// 36.3 "duplicate close is idempotent").
    AlreadyInProgress,
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
    let signing_provider = SystemDesktopHostSigningIdentityProvider;
    let task = tauri::async_runtime::spawn_blocking(move || {
        open_runtime(
            &paths,
            profile_id,
            &provider,
            &signing_provider,
            notifications,
        )
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
    tauri::async_runtime::spawn_blocking(move || close_profile_sync(&app))
        .await
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.profile.close_worker_failed",
                "runtime",
                "fatal",
                false,
                &format!("desktop profile close worker failed: {error}"),
            )
        })??;
    Ok(BridgeLifecycleDto::Closed)
}

/// Synchronous, `AppHandle`-driven equivalent of [`close_profile`]'s body
/// -- shared by that command (via `spawn_blocking`, since it must not
/// block the async runtime) and by the window-close-triggered whole-
/// application shutdown (`app_shutdown.rs`, Block 36.3), which is already
/// running on its own dedicated thread and calls this directly. Keeping
/// one shared function means the two paths can never drift apart.
///
/// A duplicate call while another close is already in flight is
/// idempotent success, not an error (Block 36.3 "duplicate close is
/// idempotent").
///
/// # Errors
///
/// Returns a structured error for lifecycle, worker, actor, database, or
/// profile-lock cleanup failure.
pub(crate) fn close_profile_sync(app: &AppHandle) -> Result<(), DesktopErrorDto> {
    let state = app.state::<DesktopAppState>();
    let action = state.take_for_close()?;
    let ready = match action {
        // Nothing new to tear down: either already closed, or another
        // close already owns the real teardown -- this call must not
        // attempt a second teardown or touch staging/registry state the
        // original owns.
        CloseAction::AlreadyClosed | CloseAction::AlreadyInProgress => None,
        CloseAction::Shutdown(ready) => Some(ready),
    };
    let Some(ready) = ready else {
        return Ok(());
    };
    let staging_cleanup = app.state::<SourceStagingControl>().cancel_and_wait();
    let result = shutdown_owned_resources(ready.owned);
    let registry_cleanup = app.state::<SelectedSourceRegistry>().clear().map(|_| ());
    let result = merge_close_results(result, staging_cleanup);
    state.finish_close(merge_close_results(result, registry_cleanup))
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
    signing_provider: &dyn DesktopHostSigningIdentityProvider,
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

    let signing_identity = match signing_provider.load_or_create(&profile_id) {
        Ok(signing_identity) => signing_identity,
        Err(error) => {
            let primary = DesktopErrorDto::new(
                "desktop.invitation_identity.unavailable",
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
            identity,
            signing_identity,
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
