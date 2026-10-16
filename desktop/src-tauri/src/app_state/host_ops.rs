//! Ready-state accessor and control methods on `DesktopAppState`: everything
//! that reads or drives an already-`Ready` runtime. Lifecycle transitions
//! (`Closed`/`Opening`/`Ready`/`Closing`/`Failed`/`ShutdownFailed`) live in
//! `lifecycle.rs`.

use super::DesktopAppState;
use super::errors::{invitation_error_dto, poisoned_state_error};
use super::state::DesktopRuntimeState;
use crate::diagnostics_dto::{DesktopDiagnosticsDto, StorageDiagnosticsDto};
use crate::dto::{DesktopErrorDto, StorageInspectionDto};
use crate::host_session_dto::{HostInvitationDto, HostSessionSnapshotDto};
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::invitation::{build_signed_invitation, current_wall_clock_ms};
use crate::platform::network_dto::{NetworkInterfaceSnapshotDto, SetNetworkBindPreferenceRequest};
use crate::platform::storage_inspection::inspect_database_client;
use crate::runtime_dto::{CommandReceiptDto, CoreSnapshotDto};
use silent_disco_core::domain::TuningSettings;
use silent_disco_core::runtime::{CoreCommand, CoreCommandRequest, SnapshotRevision};
use silent_disco_core::storage::StoredSettings;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl DesktopAppState {
    pub(crate) fn wait_for_queue_revision_change(
        &self,
        previous_revision: &str,
    ) -> Result<Option<CoreSnapshotDto>, DesktopErrorDto> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = self.current_snapshot()?;
            if snapshot.host_queue.revision != previous_revision {
                return Ok(Some(snapshot));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) fn retarget_active_host_queue(
        &self,
        snapshot: &CoreSnapshotDto,
        registry: &crate::platform::file_picker::SelectedSourceRegistry,
    ) -> Result<(), DesktopErrorDto> {
        use silent_disco_core::audio::{
            StreamingDecodeConfig, StreamingDecodeHandle, StreamingPlaylistTrackFactory,
        };

        let mut upcoming = Vec::new();
        for entry in snapshot.host_queue.entries.iter().skip(1) {
            let source = registry.resolve(&entry.source.source_id)?.ok_or_else(|| {
                DesktopErrorDto::new(
                    "desktop.queue.source_unavailable",
                    "audio",
                    "error",
                    true,
                    "an upcoming queue source is no longer staged in this desktop profile",
                )
            })?;
            let path = source.canonical_path().to_owned();
            let display_name = entry.source.display_name.clone();
            upcoming.push(
                StreamingPlaylistTrackFactory::new(
                    entry.entry_id.clone(),
                    display_name.clone(),
                    move || {
                        StreamingDecodeHandle::open(path, StreamingDecodeConfig::default()).map_err(
                            |error| silent_disco_core::audio::DecodeError {
                                kind: error.kind,
                                message: format!("{display_name}: {}", error.message),
                            },
                        )
                    },
                )
                .map_err(|error| {
                    DesktopErrorDto::new(
                        "desktop.queue.invalid_source",
                        "audio",
                        "error",
                        false,
                        &error.to_string(),
                    )
                })?,
            );
        }

        let network = {
            let runtime = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*runtime {
                DesktopRuntimeState::Ready(ready) => Arc::clone(&ready.network),
                DesktopRuntimeState::Failed(error) | DesktopRuntimeState::ShutdownFailed(error) => {
                    return Err(error.clone());
                }
                DesktopRuntimeState::Closed
                | DesktopRuntimeState::Opening { .. }
                | DesktopRuntimeState::Closing => return Ok(()),
            }
        };
        network.replace_upcoming_playlist(upcoming)
    }

    pub(crate) fn current_snapshot(&self) -> Result<CoreSnapshotDto, DesktopErrorDto> {
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

    /// Returns a bounded read-only inspection of the already-open Rust-owned database.
    ///
    /// No second database connection or profile lease is opened here; the ready
    /// runtime's existing worker remains the sole `SQLite` owner.
    pub(crate) fn storage_inspection(&self) -> Result<StorageInspectionDto, DesktopErrorDto> {
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
        let inspection = inspect_database_client(&ready.owned.database.client())
            .map_err(|error| DesktopErrorDto::from(error.to_core_error()))?;
        StorageInspectionDto::try_from(inspection).map_err(|error| {
            DesktopErrorDto::new(
                "desktop.storage.inspection_unbounded",
                "storage",
                "error",
                false,
                &error.to_string(),
            )
        })
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
        // Block 44 audit fix: a poisoned notification state mutex is itself
        // a real delivery failure (something already panicked while holding
        // the lock this diagnostics call needs), not "no failure observed"
        // -- collapsing `Err(poisoned)` and `Ok(None)` to the same `None`
        // would hide exactly the failure this diagnostics field exists to
        // surface. `unwrap_or_else(Some)` keeps `Ok(existing)` as-is and
        // turns a read failure into a visible one, mirroring how the
        // storage branch just above surfaces its own read failure via
        // `failure_reason` rather than silently reporting "available".
        let notification_failure = ready.notifications.delivery_failure().unwrap_or_else(Some);
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
        .map_err(|error| invitation_error_dto(&error))
    }

    /// Sets the desktop host's local-monitor preference (Block 34.2
    /// "monitor enable is explicit"). Enabling records a preference for the
    /// next stream start. Disabling tears down an active monitor immediately
    /// and fails visibly if that teardown cannot complete; transmission to
    /// listeners is independent of this local preference.
    pub(crate) fn set_monitor_enabled(&self, enabled: bool) -> Result<(), DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => {
                ready.network.set_monitor_enabled(enabled).map_err(|error| {
                    DesktopErrorDto::new(
                        "desktop.monitor.preference_failed",
                        "audio",
                        "error",
                        true,
                        &error.to_string(),
                    )
                })
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

    /// Applies the shared-address-space opt-in, then persists it.
    ///
    /// Apply first so an invalid change (for example, while hosting) cannot be
    /// persisted. If storage then fails, restore the previous in-memory value.
    pub(crate) fn set_allow_shared_address_space(
        &self,
        allow: bool,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let (network, database) = {
            let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
            match &*state {
                DesktopRuntimeState::Ready(ready) => {
                    (Arc::clone(&ready.network), ready.owned.database.client())
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

        let previous = network.snapshot()?.allow_shared_address_space;
        network.set_allow_shared_address_space(allow)?;

        let existing = database
            .load_settings()
            .map_err(|error| DesktopErrorDto::from(error.to_core_error()));
        let existing = match existing {
            Ok(existing) => existing,
            Err(error) => {
                network.set_allow_shared_address_space(previous)?;
                return Err(error);
            }
        };
        let settings = existing.map_or_else(
            || StoredSettings {
                tuning: TuningSettings::default(),
                allow_shared_address_space: allow,
                updated_at_ms: wall_clock_ms(),
            },
            |current| StoredSettings {
                allow_shared_address_space: allow,
                updated_at_ms: wall_clock_ms(),
                ..current
            },
        );
        if let Err(error) = database.save_settings(&settings) {
            network.set_allow_shared_address_space(previous)?;
            return Err(DesktopErrorDto::from(error.to_core_error()));
        }
        network.snapshot()
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

    pub(super) fn notification_buffer(
        &self,
    ) -> Result<Arc<DesktopNotificationBuffer>, DesktopErrorDto> {
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
}
