//! Settings commands for the host's remembered ("Always allow") listeners.
//! Records live in the profile's Rust database; the UI only lists and revokes.

use crate::app_state::DesktopAppState;
use crate::dto::DesktopErrorDto;
use serde::{Deserialize, Serialize};
use silent_disco_core::transport::secure_trust_store::{
    RememberedListener, TrustStoreError, parse_device_id,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;
use ts_rs::TS;

/// A listener this host trusts: it joins Approved-devices sessions without
/// asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct RememberedListenerDto {
    pub device_id: String,
    pub display_name: String,
    /// Wall-clock approval time in Unix milliseconds, as a decimal string.
    pub approved_at_ms: String,
}

impl From<RememberedListener> for RememberedListenerDto {
    fn from(value: RememberedListener) -> Self {
        Self {
            device_id: value.device_id.as_str().to_owned(),
            display_name: value.display_name,
            approved_at_ms: value.enrolled_at_ms.to_string(),
        }
    }
}

fn trust_error(error: &TrustStoreError) -> DesktopErrorDto {
    let (code, category) = match error {
        TrustStoreError::Storage { .. } => ("desktop.trust.storage_failed", "storage"),
        _ => ("desktop.trust.invalid_listener", "validation"),
    };
    DesktopErrorDto::new(code, category, "error", false, &error.to_string())
}

/// The listeners this host trusts, oldest approval first.
///
/// # Errors
/// No ready profile, or the records cannot be read.
#[allow(clippy::needless_pass_by_value)] // Tauri command extraction requires State by value.
#[tauri::command]
pub fn list_remembered_listeners(
    state: State<'_, DesktopAppState>,
) -> Result<Vec<RememberedListenerDto>, DesktopErrorDto> {
    list(&state)
}

/// Revokes a remembered listener. It is untrusted from its next join, even
/// during a running session. Returns `false` if it was not trusted.
///
/// # Errors
/// No ready profile, a device ID that is not key-bound, or a storage failure.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn revoke_remembered_listener(
    state: State<'_, DesktopAppState>,
    device_id: String,
) -> Result<bool, DesktopErrorDto> {
    revoke(&state, &device_id)
}

pub(crate) fn list(state: &DesktopAppState) -> Result<Vec<RememberedListenerDto>, DesktopErrorDto> {
    Ok(state
        .ready_verified_trust()?
        .list()
        .map_err(|error| trust_error(&error))?
        .into_iter()
        .map(RememberedListenerDto::from)
        .collect())
}

pub(crate) fn revoke(state: &DesktopAppState, device_id: &str) -> Result<bool, DesktopErrorDto> {
    let device_id = parse_device_id(device_id).map_err(|error| trust_error(&error))?;
    let revoked = state
        .ready_verified_trust()?
        .revoke(&device_id, unix_time_ms())
        .map_err(|error| trust_error(&error))?;
    if revoked {
        // A connected listener is disconnected now, not only refused at its
        // next join (owner decision 2026-11-18).
        state.notify_listener_revoked(&device_id)?;
    }
    Ok(revoked)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
