//! Lab Mode Tauri command surface (Block 42; spec section 29).
//!
//! Compiled only under the `lab-mode` Cargo feature -- mirroring
//! `crate::lab` itself, which this module is the only Tauri-reachable
//! entry point into. See `crate::lab_dto`'s own module doc comment for why
//! the DTOs these commands produce/consume live in a separate,
//! *unconditionally* compiled module rather than here.
//!
//! `LabScreen.tsx` (the frontend) never mutates node domain state
//! directly -- every command here either reads bounded, already-computed
//! state out of [`LabAppState`]/[`LabRuntime`], or submits a whole
//! scenario/test command through the exact same production
//! `LabRuntime`/`scenario::run_scenario_with_trace` entry points Block 40
//! already proved deterministic. No command in this file reconstructs or
//! bypasses that logic.
//!
//! ## Module layout
//!
//! The nine `#[tauri::command]`-annotated entry points
//! (`lab_get_state`/`lab_open_scenario_file`/`lab_save_scenario_file`/
//! `lab_run_loaded_scenario`/`lab_advance_virtual_time`/`lab_start_node`/
//! `lab_stop_node`/`lab_stop_all_nodes`/`lab_export_recording_file`) live
//! directly in this file, not re-exported from a submodule: Tauri's
//! command macro generates hidden companion items (e.g.
//! `__cmd__lab_run_loaded_scenario`) alongside each function that
//! `generate_handler!` looks up at the exact path `lib.rs` names
//! (`lab_commands::lab_run_loaded_scenario`) -- a `pub use
//! run_control::lab_run_loaded_scenario;` re-export only re-exports the
//! function itself, not those hidden macro companions, so
//! `generate_handler!` would fail to find them (confirmed the hard way
//! during this split: this exact failure mode is also documented in
//! `app_state::commands`'s own module doc). Keeping the annotated
//! functions defined directly where `lib.rs` names them avoids that
//! without resorting to a wildcard `pub use submodule::*;` re-export.
//!
//! Everything each command's *body* calls into is still split by
//! responsibility:
//!
//! - [`errors`]: every structured `DesktopErrorDto` constructor this
//!   command surface returns.
//! - [`session`]: lazy [`LabRuntime`] construction/reuse for the current
//!   session.
//! - [`dto_convert`]: domain state -> DTO conversion (node/scenario
//!   summary/run outcome/session state), including the UI-facing
//!   timeline-entry and summary-text bounds.
//! - [`scenario_io`]: bounded scenario file read/parse/validate helpers.
//!
//! ## Scope: what "start/pause/step/stop" honestly maps to
//!
//! `scenario::run_scenario_with_trace` is architected as one atomic,
//! synchronous run (Block 40.4's own proven "two runs of the same
//! scenario and seed produce an equal report" -- the very thing Block 41's
//! replay/divergence detection depends on). It has no interior
//! cancellation point, and none is added here: a mid-run "pause" or "stop"
//! would either have to silently no-op (dishonest) or introduce a new,
//! untested non-determinism into a module whose entire value proposition
//! is determinism. Instead:
//!
//! - **start** = [`lab_run_loaded_scenario`], which runs a validated,
//!   already-open scenario to completion (typically well under a second
//!   of real wall-clock time for a realistic scenario -- see
//!   `scenario.rs`'s own "Step settlement" doc comment for the bounded
//!   real-time safety valve this rests on);
//! - **step** = [`lab_advance_virtual_time`], the literal
//!   `LabRuntime::advance` primitive spec section 29.2 calls "manual
//!   advancement" -- the *only* way virtual time ever moves in this
//!   architecture;
//! - **stop** = [`lab_stop_all_nodes`], tearing down every node this
//!   runtime is currently holding (both scenario-run nodes, if a run just
//!   finished and left artifacts, and any node started interactively
//!   through [`lab_start_node`]);
//! - **pause** is a frontend-only gate (`LabScreen.tsx`) over the *step*
//!   action -- there is no backend "auto-running clock" to pause, since
//!   virtual time never advances except through an explicit `advance`
//!   call in the first place.
//!
//! [`lab_run_loaded_scenario`]'s own `running` guard (visible through
//! [`crate::lab_dto::LabStateDto::running`]) still gives the frontend a
//! real, backend-enforced "running-state command disablement" signal (a
//! second run cannot be submitted while one is in flight, and
//! [`lab_advance_virtual_time`] is refused while a run owns the shared
//! clock) -- it is the *interruption* of an in-flight run that is
//! deliberately out of scope, not the disablement itself.

use crate::dto::DesktopErrorDto;
use crate::lab::recording::ScenarioRecording;
use crate::lab::scenario::{Scenario, ScenarioReport, ScenarioTrace, run_scenario_with_trace};
use crate::lab::{LabNodeId, LabRuntime};
use crate::lab_dto::{
    LabAdvanceTimeRequest, LabFileOutcomeDto, LabNodeDto, LabRunOutcomeDto, LabScenarioSummaryDto,
    LabStartNodeRequest, LabStateDto, LabStopNodeRequest,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use dto_convert::{node_dto, run_outcome_dto, scenario_summary_dto, state_dto};
use errors::{
    already_running_error, execution_error, invalid_node_id_error, no_run_to_export_error,
    no_scenario_loaded_error, path_unavailable_error, poisoned_error, recording_io_error,
};
use scenario_io::{parse_and_validate, read_bounded_scenario_file};
use session::ensure_runtime;

mod dto_convert;
mod errors;
mod scenario_io;
mod session;

/// Hard cap on rendered timeline entries per node (Block 42 "bounded event
/// timeline"). The underlying recorder is already bounded to
/// `recorder::MAX_RECORDED_NOTIFICATIONS` (4096); this is a *further*,
/// UI-facing bound -- a scenario producing thousands of notifications
/// should never hand the frontend an unbounded list to render, even
/// though the backend trace itself would technically fit in memory.
const MAX_TIMELINE_ENTRIES_PER_NODE: usize = 50;

/// Hard cap on a rendered timeline entry's own summary text.
const MAX_SUMMARY_CHARS: usize = 200;

struct LoadedScenario {
    scenario: Scenario,
    raw_bytes: Vec<u8>,
}

struct LastRun {
    scenario: Scenario,
    report: ScenarioReport,
    trace: ScenarioTrace,
}

#[derive(Default)]
struct LabSessionState {
    runtime: Option<Arc<LabRuntime>>,
    loaded: Option<LoadedScenario>,
    running: bool,
    last_run: Option<LastRun>,
}

/// Tauri-managed Lab session state (Block 42). One instance per
/// application process -- deliberately independent of
/// [`crate::app_state::DesktopAppState`] (Block 37.2 "no global production
/// singleton reuse" still applies to this later block's own command
/// layer).
pub struct LabAppState {
    inner: Mutex<LabSessionState>,
}

impl LabAppState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LabSessionState::default()),
        }
    }
}

impl Default for LabAppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the complete Lab session state (Block 42 "node list and state
/// panels", "virtual time"). Also the natural point every Lab command
/// lazily constructs the underlying [`LabRuntime`] at, matching the same
/// lazy-open discipline `app_state::DesktopAppState` already uses for
/// production profiles.
///
/// # Errors
///
/// Returns a structured error when the application-local-data directory
/// cannot be resolved or the Lab runtime fails to start.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_get_state(app: AppHandle) -> Result<LabStateDto, DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    let runtime = ensure_runtime(&app, &mut session)?;
    Ok(state_dto(&runtime, &session))
}

/// Opens a native file dialog restricted to `.json`, parses and validates
/// the chosen scenario, and stores it as this session's loaded scenario
/// (Block 42 "scenario open ... through restricted dialogs").
///
/// Dialog cancellation returns `Ok(None)`. A malformed or invalid scenario
/// is a reported [`DesktopErrorDto`], never silently accepted or swallowed
/// (Block 42's own "invalid scenario display" test).
///
/// # Errors
///
/// Returns a structured error for a dialog/path failure or an invalid
/// scenario document.
#[tauri::command]
pub async fn lab_open_scenario_file(
    app: AppHandle,
) -> Result<Option<LabScenarioSummaryDto>, DesktopErrorDto> {
    let dialog_app = app.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_title("Open Lab scenario")
            .add_filter("Lab scenario", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.dialog_worker_failed",
            "platform",
            "error",
            true,
            &format!("scenario open dialog worker failed: {error}"),
        )
    })?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|error| path_unavailable_error(&error))?;
    let bytes = read_bounded_scenario_file(&path)?;
    let scenario = parse_and_validate(&bytes)?;
    let summary = scenario_summary_dto(&scenario);

    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    session.loaded = Some(LoadedScenario {
        scenario,
        raw_bytes: bytes,
    });
    Ok(Some(summary))
}

/// Opens a native save dialog restricted to `.json` and writes the
/// currently loaded scenario's own bytes to the chosen destination (Block
/// 42 "scenario ... save through restricted dialogs"). There is no
/// scenario editor in this block, so "save" writes back exactly the bytes
/// that were validated on open -- a deliberate "save a copy", never a
/// silent mutation of scenario content.
///
/// # Errors
///
/// Returns [`no_scenario_loaded_error`] when nothing is loaded, or a
/// structured dialog/write failure.
#[tauri::command]
pub async fn lab_save_scenario_file(app: AppHandle) -> Result<LabFileOutcomeDto, DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let raw_bytes = {
        let session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
        session
            .loaded
            .as_ref()
            .map(|loaded| loaded.raw_bytes.clone())
            .ok_or_else(no_scenario_loaded_error)?
    };

    let dialog_app = app.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_title("Save Lab scenario")
            .add_filter("Lab scenario", &["json"])
            .set_file_name("scenario.json")
            .blocking_save_file()
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.dialog_worker_failed",
            "platform",
            "error",
            true,
            &format!("scenario save dialog worker failed: {error}"),
        )
    })?;
    let Some(selected) = selected else {
        return Ok(LabFileOutcomeDto::Cancelled);
    };
    let path = selected
        .into_path()
        .map_err(|error| path_unavailable_error(&error))?;
    std::fs::write(&path, &raw_bytes).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.scenario_write_failed",
            "platform",
            "error",
            true,
            &format!("could not write the scenario file: {error}"),
        )
    })?;
    Ok(LabFileOutcomeDto::Saved)
}

/// Runs the currently loaded scenario to completion (Block 42 "start").
/// See this module's own doc comment for exactly what "start" honestly
/// maps to.
///
/// # Errors
///
/// Returns [`no_scenario_loaded_error`] when nothing is loaded,
/// [`already_running_error`] when a run is already in flight, or a
/// structured execution failure.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_run_loaded_scenario(app: AppHandle) -> Result<LabRunOutcomeDto, DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let (runtime, scenario) = {
        let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
        if session.running {
            return Err(already_running_error());
        }
        let runtime = ensure_runtime(&app, &mut session)?;
        let scenario = session
            .loaded
            .as_ref()
            .map(|loaded| loaded.scenario.clone())
            .ok_or_else(no_scenario_loaded_error)?;
        session.running = true;
        (runtime, scenario)
    };

    let run_result = run_scenario_with_trace(&runtime, &scenario);

    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    session.running = false;
    let (report, trace) = match run_result {
        Ok(outcome) => outcome,
        Err(error) => return Err(execution_error(&error)),
    };
    let dto = run_outcome_dto(&report, &trace);
    session.last_run = Some(LastRun {
        scenario,
        report,
        trace,
    });
    Ok(dto)
}

/// Manually advances the shared scenario virtual clock (Block 42 "step";
/// spec 29.2 "manual advancement").
///
/// # Errors
///
/// Returns [`already_running_error`] while a scenario run owns the shared
/// clock, or a structured overflow error.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_advance_virtual_time(
    app: AppHandle,
    request: LabAdvanceTimeRequest,
) -> Result<String, DesktopErrorDto> {
    let delta_ms: u64 = request.delta_ms.parse().map_err(|_| {
        DesktopErrorDto::new(
            "desktop.lab.invalid_delta_ms",
            "validation",
            "error",
            false,
            "deltaMs must be a non-negative integer",
        )
    })?;
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    if session.running {
        return Err(already_running_error());
    }
    let runtime = ensure_runtime(&app, &mut session)?;
    let now = runtime.advance(delta_ms)?;
    Ok(now.get().to_string())
}

/// Starts one new, fully isolated Lab node under manual/interactive
/// control (Block 42 "node list and state panels", "start ... controls").
///
/// # Errors
///
/// Returns a structured error when the bounded node count is reached or
/// the requested clock configuration is invalid.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_start_node(
    app: AppHandle,
    request: LabStartNodeRequest,
) -> Result<LabNodeDto, DesktopErrorDto> {
    let offset_ms: i64 = request.offset_ms.parse().map_err(|_| {
        DesktopErrorDto::new(
            "desktop.lab.invalid_offset_ms",
            "validation",
            "error",
            false,
            "offsetMs must be an integer",
        )
    })?;
    let drift_ppm: i64 = request.drift_ppm.parse().map_err(|_| {
        DesktopErrorDto::new(
            "desktop.lab.invalid_drift_ppm",
            "validation",
            "error",
            false,
            "driftPpm must be an integer",
        )
    })?;
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    let runtime = ensure_runtime(&app, &mut session)?;
    let node_id = runtime.start_node_with_clock(offset_ms, drift_ppm)?;
    node_dto(&runtime, node_id).ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.lab.node_vanished",
            "runtime",
            "fatal",
            false,
            "the Lab node was started but its state could not be read back",
        )
    })
}

/// Stops exactly one Lab node (Block 42 "stop ... controls").
///
/// # Errors
///
/// Returns an invalid-identifier error, or a structured error when the
/// node does not exist.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_stop_node(app: AppHandle, request: LabStopNodeRequest) -> Result<(), DesktopErrorDto> {
    let raw = request.node_id.as_str();
    let node_id = raw
        .parse::<u32>()
        .map(LabNodeId::from_u32)
        .map_err(|_| invalid_node_id_error(raw))?;
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    let runtime = ensure_runtime(&app, &mut session)?;
    runtime.stop_node(node_id)
}

/// Stops and releases every currently active Lab node (Block 42 "stop
/// ... controls"). See this module's own doc comment for why this, not an
/// interior scenario-run cancellation, is what "stop" honestly maps to.
///
/// # Errors
///
/// Returns one bounded structured error naming every node that failed to
/// tear down cleanly.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_stop_all_nodes(app: AppHandle) -> Result<(), DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    let runtime = ensure_runtime(&app, &mut session)?;
    runtime.shutdown()
}

/// Opens a native save dialog restricted to `.json` and writes a complete,
/// bounded, redacted recording of the last completed scenario run (Block
/// 42 "recording export"; Block 41's `ScenarioRecording`).
///
/// # Errors
///
/// Returns [`no_run_to_export_error`] when no run has completed yet, or a
/// structured dialog/encode/write failure.
#[tauri::command]
pub async fn lab_export_recording_file(
    app: AppHandle,
) -> Result<LabFileOutcomeDto, DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let recording = {
        let session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
        let last_run = session
            .last_run
            .as_ref()
            .ok_or_else(no_run_to_export_error)?;
        ScenarioRecording::capture(
            &last_run.scenario,
            last_run.report.clone(),
            last_run.trace.clone(),
        )
    };

    let dialog_app = app.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_title("Save Lab scenario recording")
            .add_filter("Lab recording", &["json"])
            .set_file_name("lab-recording.json")
            .blocking_save_file()
    })
    .await
    .map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.dialog_worker_failed",
            "platform",
            "error",
            true,
            &format!("recording export dialog worker failed: {error}"),
        )
    })?;
    let Some(selected) = selected else {
        return Ok(LabFileOutcomeDto::Cancelled);
    };
    let path = selected
        .into_path()
        .map_err(|error| path_unavailable_error(&error))?;
    crate::lab::recording::save_recording_to_path(&recording, &path)
        .map_err(|error| recording_io_error(&error))?;
    Ok(LabFileOutcomeDto::Saved)
}

#[cfg(test)]
mod tests;
