//! The ready profile's remembered listeners, for the Settings commands.

use super::DesktopAppState;
use super::errors::poisoned_state_error;
use super::state::DesktopRuntimeState;
use crate::dto::DesktopErrorDto;
use crate::platform::verified_trust::DesktopVerifiedTrust;
use silent_disco_core::domain::DeviceId;
use silent_disco_core::runtime::{CoreCommand, CoreCommandRequest};
use std::sync::Arc;

impl DesktopAppState {
    /// The open profile's trust store, shared with its host transport, so a
    /// change here reaches the next join at once.
    pub(crate) fn ready_verified_trust(
        &self,
    ) -> Result<Arc<DesktopVerifiedTrust>, DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        match &*state {
            DesktopRuntimeState::Ready(ready) => Ok(Arc::clone(&ready.verified_trust)),
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

    /// Tells the host actor a remembered listener was revoked, so a running
    /// session disconnects it now if it is connected (owner decision
    /// 2026-11-18); the actor leaves a listener that is not connected alone.
    /// The command carries no revision precondition, so the current one is
    /// used as is.
    pub(crate) fn notify_listener_revoked(
        &self,
        device_id: &DeviceId,
    ) -> Result<(), DesktopErrorDto> {
        let state = self.runtime.lock().map_err(|_| poisoned_state_error())?;
        let DesktopRuntimeState::Ready(ready) = &*state else {
            return Ok(()); // no running host to tell
        };
        let revision = ready
            .handle
            .current_snapshot()
            .map_err(DesktopErrorDto::from)?
            .revision;
        let request = CoreCommandRequest::new(
            revision,
            CoreCommand::ListenerRevoked {
                listener_id: device_id.clone(),
            },
        )
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.command.invalid_payload",
                "validation",
                "error",
                false,
                &error.to_string(),
            )
        })?;
        ready
            .handle
            .submit_command(request)
            .map(|_| ())
            .map_err(DesktopErrorDto::from)
    }
}
