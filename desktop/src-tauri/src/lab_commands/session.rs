//! Lazy [`LabRuntime`] construction/reuse for the current Lab session
//! (Block 42; mirrors `app_state::DesktopAppState`'s own lazy-open
//! discipline for production profiles).

use super::LabSessionState;
use crate::dto::DesktopErrorDto;
use crate::lab::LabRuntime;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub(super) fn ensure_runtime(
    app: &AppHandle,
    state: &mut LabSessionState,
) -> Result<Arc<LabRuntime>, DesktopErrorDto> {
    if let Some(runtime) = &state.runtime {
        return Ok(Arc::clone(runtime));
    }
    let app_local_data_root = app.path().app_local_data_dir().map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.path_resolution_failed",
            "platform",
            "fatal",
            false,
            &format!("could not resolve the application-local-data directory: {error}"),
        )
    })?;
    // Block 38.2 "deterministic starting time": every Lab session in this
    // process starts its shared virtual timeline at the same instant.
    let runtime = Arc::new(LabRuntime::new(&app_local_data_root, 0).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.runtime_start_failed",
            "runtime",
            "fatal",
            false,
            &error.to_string(),
        )
    })?);
    state.runtime = Some(Arc::clone(&runtime));
    Ok(runtime)
}
