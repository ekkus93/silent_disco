//! `DesktopAppState`'s lifecycle transitions: the state-machine edges that
//! move a profile between `Closed`/`Opening`/`Ready`/`Closing`/`Failed`/
//! `ShutdownFailed`. Accessor/control methods that operate on an already-
//! `Ready` runtime live in `host_ops.rs`; the Tauri command bodies that
//! drive these transitions live in `mod.rs`.

use super::DesktopAppState;
use super::errors::poisoned_state_error;
use super::state::{CloseAction, DesktopRuntimeState, ReadyRuntime};
use crate::dto::{BridgeLifecycleDto, CoreVersionDto, DesktopErrorDto};
use crate::profile::ProfileId;
use crate::runtime_dto::{CoreSnapshotDto, OpenProfileResponse};

#[cfg(test)]
use super::construct::open_runtime;
#[cfg(test)]
use crate::notification_buffer::DesktopNotificationBuffer;
#[cfg(test)]
use crate::platform::identity::DesktopIdentityProvider;
#[cfg(test)]
use crate::platform::invitation_identity::DesktopHostSigningIdentityProvider;
#[cfg(test)]
use crate::platform::paths::DesktopProfilePaths;
#[cfg(test)]
use crate::shutdown::shutdown_owned_resources;
#[cfg(test)]
use std::sync::Arc;

impl DesktopAppState {
    pub(super) fn begin_open(&self, profile_id: &ProfileId) -> Result<(), DesktopErrorDto> {
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

    pub(super) fn fail_open(&self, error: DesktopErrorDto) -> Result<(), DesktopErrorDto> {
        let mut state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        *state = DesktopRuntimeState::Failed(error);
        Ok(())
    }

    pub(super) fn install_ready(
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

    pub(super) fn take_for_close(&self) -> Result<CloseAction, DesktopErrorDto> {
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

    pub(super) fn finish_close(
        &self,
        result: Result<(), DesktopErrorDto>,
    ) -> Result<(), DesktopErrorDto> {
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
    pub(super) fn open_profile_sync(
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
                    let error = primary.with_appended_cleanup(cleanup.err());
                    if let Err(state_error) = self.fail_open(error.clone()) {
                        return Err(error.with_appended_cleanup(Some(state_error)));
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
    pub(super) fn close_sync(&self) -> Result<(), DesktopErrorDto> {
        match self.take_for_close()? {
            CloseAction::AlreadyClosed | CloseAction::AlreadyInProgress => Ok(()),
            CloseAction::Shutdown(ready) => {
                self.finish_close(shutdown_owned_resources(ready.owned))
            }
        }
    }
}
