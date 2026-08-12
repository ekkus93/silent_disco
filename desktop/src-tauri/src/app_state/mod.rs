//! Desktop application lifecycle state, split by responsibility:
//! - [`state`]: the runtime state-machine types (`DesktopRuntimeState`,
//!   `ReadyRuntime`, `CloseAction`).
//! - [`lifecycle`]: the state-machine transition methods (`begin_open`,
//!   `fail_open`, `install_ready`, `take_for_close`, `finish_close`, and the
//!   `#[cfg(test)]` synchronous open/close wrappers).
//! - [`host_ops`]: accessor/control methods on an already-`Ready` runtime
//!   (snapshot/diagnostics reads, playback control, invitation creation,
//!   command submission).
//! - [`commands`]: the shared `close_profile_sync` body used by both the
//!   `close_profile` Tauri command below and `app_shutdown.rs`.
//! - [`construct`]: `open_runtime`, the multi-step profile bring-up
//!   pipeline.
//! - [`errors`]: error-mapping helpers (`poisoned_state_error`,
//!   `invitation_error_dto`).
//!
//! The four `#[tauri::command]`-annotated entry points are defined directly
//! in this file, not re-exported from a submodule: Tauri's command macro
//! generates hidden companion items (e.g. `__cmd__open_profile`) that
//! `generate_handler!` looks up at the exact path `lib.rs` names
//! (`app_state::open_profile`), and a `pub use commands::open_profile;`
//! re-export would not carry those hidden items along -- see `commands.rs`'s
//! module doc for the full explanation. `close_profile_sync` (a plain,
//! non-macro `pub(crate) fn`) has no such restriction and is re-exported
//! normally so `app_shutdown.rs`'s direct
//! `crate::app_state::close_profile_sync` call keeps compiling unchanged.
//! `CloseAction` and `invitation_error_dto` are also re-exported (privately)
//! so `app_state_tests.rs`'s
//! `use super::{CloseAction, DesktopAppState, invitation_error_dto};` keeps
//! resolving without editing that file.

mod commands;
mod construct;
mod errors;
mod host_ops;
mod lifecycle;
mod state;

use construct::open_runtime;
use state::DesktopRuntimeState;
use std::sync::{Arc, Mutex};

#[cfg(test)]
use errors::invitation_error_dto;
#[cfg(test)]
use state::CloseAction;

use crate::dto::{BridgeLifecycleDto, DesktopErrorDto, StorageInspectionDto};
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::notification_channel::TauriNotificationSink;
use crate::platform::identity::SystemDesktopIdentityProvider;
use crate::platform::invitation_identity::SystemDesktopHostSigningIdentityProvider;
use crate::platform::paths::resolve_profile_paths;
use crate::profile::ProfileId;
use crate::runtime_dto::{
    AttachNotificationResponse, CoreNotificationDto, CoreSnapshotDto, OpenProfileRequest,
    OpenProfileResponse,
};
use crate::shutdown::shutdown_owned_resources;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

pub(crate) use commands::close_profile_sync;

pub struct DesktopAppState {
    runtime: Mutex<DesktopRuntimeState>,
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
                let error = primary.with_appended_cleanup(cleanup.err());
                if let Err(state_error) = state.fail_open(error.clone()) {
                    return Err(error.with_appended_cleanup(Some(state_error)));
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

/// Returns a bounded read-only inspection from the already-open Rust database worker.
///
/// # Errors
///
/// Returns a structured error when no profile is ready or any typed storage query fails.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command injection supplies AppHandle by value"
)]
#[tauri::command]
pub async fn get_storage_inspection(
    app: AppHandle,
) -> Result<StorageInspectionDto, DesktopErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopAppState>().storage_inspection()
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.storage.inspection_worker_failed",
            "runtime",
            "error",
            true,
            &format!("desktop storage inspection worker failed: {error}"),
        )
    })?
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

#[cfg(test)]
#[path = "../app_state_tests.rs"]
mod tests;
