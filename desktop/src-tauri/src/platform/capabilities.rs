use super::failure::core_error;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CapabilitySnapshot, CoreActorHandle, CoreSnapshot, PlatformEvent,
};
use std::thread;
use std::time::{Duration, Instant};

const CAPABILITY_STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
const CAPABILITY_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Returns the exact desktop capabilities implemented by the current platform adapters.
#[must_use]
pub(crate) const fn desktop_capabilities() -> CapabilitySnapshot {
    CapabilitySnapshot {
        nearby_discovery_available: false,
        nearby_advertising_available: false,
        local_network_available: false,
        audio_source_selection_available: true,
        audio_output_available: false,
        secure_store_available: true,
    }
}

/// Publishes capabilities through the actor and returns the authoritative acknowledged snapshot.
pub(crate) fn publish_desktop_capabilities(
    handle: &CoreActorHandle,
) -> Result<CoreSnapshot, CoreError> {
    let expected = desktop_capabilities();
    handle.submit_platform_event(PlatformEvent::CapabilityStateChanged(expected))?;
    let deadline = Instant::now() + CAPABILITY_STARTUP_TIMEOUT;
    loop {
        let snapshot = handle.current_snapshot()?;
        if snapshot.capabilities == expected {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(core_error(
                CoreErrorCode::PlatformOperationFailed,
                "desktop capability snapshot was not acknowledged before startup timeout",
                ErrorSeverity::Fatal,
                false,
                None,
            ));
        }
        thread::sleep(CAPABILITY_POLL_INTERVAL);
    }
}
