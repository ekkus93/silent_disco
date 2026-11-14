//! The ready profile's remembered listeners, for the Settings commands.

use super::DesktopAppState;
use super::errors::poisoned_state_error;
use super::state::DesktopRuntimeState;
use crate::dto::DesktopErrorDto;
use crate::platform::verified_trust::DesktopVerifiedTrust;
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
}
