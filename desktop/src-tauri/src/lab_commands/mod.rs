//! Lab Mode Tauri command surface (Block 42; spec section 29).
//!
//! Compiled only under the `lab-mode` Cargo feature -- mirroring
//! `crate::lab` itself, which this module is the only Tauri-reachable
//! entry point into. See `crate::lab_dto`'s own module doc comment for why
//! the DTOs these commands produce/consume live in a separate,
//! *unconditionally* compiled module rather than here.
//!
//! `LabScreen.tsx` (the frontend) never mutates node domain state
//! directly. Commands here read bounded, already-computed state from
//! [`LabAppState`]/[`LabRuntime`], edit the loaded scenario only through
//! its canonical parser/validator, or submit scenario/test commands through
//! the same production `LabRuntime`/`scenario::run_scenario_with_trace`
//! entry points Block 40 already proved deterministic. No command in this
//! file reconstructs or bypasses those domain rules.
//!
//! ## Module layout
//!
//! The twelve `#[tauri::command]`-annotated entry points
//! (`lab_get_state`/`lab_open_scenario_file`/`lab_save_scenario_file`/
//! `lab_set_link_faults`/
//! `lab_run_loaded_scenario`/`lab_pause_loaded_scenario`/
//! `lab_resume_loaded_scenario`/`lab_advance_virtual_time`/`lab_start_node`/
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
//! - [`scenario_io`]: bounded scenario file read/parse/validate helpers and
//!   canonical-revalidation-backed fault-profile edits.
//!
//! ## Scope: deterministic start/pause/step/stop
//!
//! Block 42 now layers cooperative control over the proven atomic scenario
//! runner instead of replacing its deterministic execution model. A pause is
//! observed only between completed scenario steps, so the current step is
//! allowed to settle atomically before future virtual-time progression stops.
//! Stop is also checked inside the bounded settlement poll so cancellation
//! does not have to wait for the full five-second safety timeout. The runner's
//! existing transport/node teardown remains the sole owner of scenario-run
//! cleanup; `lab_stop_all_nodes` never races `LabRuntime::shutdown` against a
//! live runner.
//!
//! - **start** = [`lab_run_loaded_scenario`], executed on a blocking worker so
//!   Tauri remains able to accept pause/resume/stop commands while it runs;
//! - **pause/resume** = [`lab_pause_loaded_scenario`] /
//!   [`lab_resume_loaded_scenario`], controlling the next deterministic step
//!   boundary without advancing virtual time while paused;
//! - **step** = [`lab_advance_virtual_time`], still the literal manual
//!   `LabRuntime::advance` primitive and refused while a scenario owns the
//!   runtime;
//! - **stop** = [`lab_stop_all_nodes`]. During a run it requests cooperative
//!   cancellation and lets the runner clean its own nodes first; after that
//!   cleanup it shuts down any remaining manually-started Lab nodes.

use crate::dto::DesktopErrorDto;
use crate::lab::recording::ScenarioRecording;
use crate::lab::scenario::{
    Scenario, ScenarioExecutionError, ScenarioReport, ScenarioRunControl, ScenarioRunControlError,
    ScenarioTrace, run_scenario_with_trace_controlled,
};
use crate::lab::{LabNodeId, LabRuntime};
use crate::lab_dto::{
    LabAdvanceTimeRequest, LabFileOutcomeDto, LabNodeDto, LabRunOutcomeDto, LabScenarioSummaryDto,
    LabSetLinkFaultsRequest, LabStartNodeRequest, LabStateDto, LabStopNodeRequest,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use dto_convert::{node_dto, run_outcome_dto, scenario_summary_dto, state_dto};
use errors::{
    already_running_error, execution_error, invalid_fault_field_error, invalid_node_id_error,
    no_active_run_error, no_run_to_export_error, no_scenario_loaded_error, path_unavailable_error,
    poisoned_error, recording_io_error, run_control_error, run_worker_error, scenario_stopped_error,
};
use scenario_io::{parse_and_validate, read_bounded_scenario_file, rewrite_link_faults};
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
    paused: bool,
    run_control: Option<Arc<ScenarioRunControl>>,
    stop_all_after_run: bool,
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
    if session.running {
        return Err(already_running_error());
    }
    session.loaded = Some(LoadedScenario {
        scenario,
        raw_bytes: bytes,
    });
    Ok(Some(summary))
}

/// Opens a native save dialog restricted to `.json` and writes the
/// currently loaded scenario's own bytes to the chosen destination (Block
/// 42 "scenario ... save through restricted dialogs"). There is no
/// scenario editor outside the explicit fault controls: before an edit this
/// writes back exactly the bytes validated on open; after an operator changes
/// a link fault profile it writes the backend-revalidated edited JSON. No
/// command silently changes scenario content.
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

/// Updates one loaded scenario target's initial receive-side latency, jitter,
/// and loss profile (Block 42 "fault configuration"). If multiple declared
/// links target that same receiver, they are updated together because the
/// live transport applies one receive-fault profile per target node.
///
/// This edits the validated scenario document that the existing live scenario
/// runner will consume on its *next* run. It does not mutate an in-flight run;
/// scheduled `setLinkFaults` scenario steps remain the deterministic mechanism
/// for changing faults at a specific virtual time once a run has started when
/// the target has the single-inbound topology those steps require.
///
/// # Errors
///
/// Returns [`no_scenario_loaded_error`] when nothing is loaded,
/// [`already_running_error`] while a scenario is executing, a structured
/// numeric-field error for malformed input, or the canonical scenario
/// validation error if the edited profile violates schema bounds or the
/// receive-fault topology constraints.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_set_link_faults(
    app: AppHandle,
    request: LabSetLinkFaultsRequest,
) -> Result<LabScenarioSummaryDto, DesktopErrorDto> {
    let latency_ms = parse_fault_u64("latencyMs", &request.latency_ms)?;
    let jitter_ms = parse_fault_u64("jitterMs", &request.jitter_ms)?;
    let loss_permille = request.loss_permille.parse::<u16>().map_err(|_| {
        invalid_fault_field_error("lossPermille", request.loss_permille.as_str())
    })?;

    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    if session.running {
        return Err(already_running_error());
    }
    let loaded = session
        .loaded
        .as_mut()
        .ok_or_else(no_scenario_loaded_error)?;
    let (scenario, raw_bytes) = rewrite_link_faults(
        &loaded.raw_bytes,
        request.link_index,
        &request.from,
        &request.to,
        latency_ms,
        jitter_ms,
        loss_permille,
    )?;
    let summary = scenario_summary_dto(&scenario);
    loaded.scenario = scenario;
    loaded.raw_bytes = raw_bytes;
    Ok(summary)
}

fn parse_fault_u64(field: &str, raw: &str) -> Result<u64, DesktopErrorDto> {
    raw.parse::<u64>()
        .map_err(|_| invalid_fault_field_error(field, raw))
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
pub async fn lab_run_loaded_scenario(app: AppHandle) -> Result<LabRunOutcomeDto, DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let (runtime, scenario, control) = {
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
        let control = Arc::new(ScenarioRunControl::default());
        session.running = true;
        session.paused = false;
        session.stop_all_after_run = false;
        session.run_control = Some(Arc::clone(&control));
        (runtime, scenario, control)
    };

    let run_runtime = Arc::clone(&runtime);
    let run_scenario = scenario.clone();
    let run_control = Arc::clone(&control);
    let worker_result = tauri::async_runtime::spawn_blocking(move || {
        run_scenario_with_trace_controlled(&run_runtime, &run_scenario, &run_control)
    })
    .await;

    let worker_failed = worker_result.is_err();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    // A cooperative Stop requests this explicitly. A blocking-worker failure
    // is different: the runner may have panicked before reaching its normal
    // teardown, so fail closed and force runtime shutdown rather than leaving
    // possibly-live Lab nodes behind. Holding the session mutex prevents a new
    // Lab command from racing this recovery cleanup.
    let shutdown_result = if session.stop_all_after_run || worker_failed {
        runtime.shutdown().err()
    } else {
        None
    };
    session.running = false;
    session.paused = false;
    session.stop_all_after_run = false;
    session.run_control = None;

    let run_result = worker_result.map_err(|error| {
        run_worker_error(&error).with_appended_cleanup(shutdown_result.clone())
    })?;
    let (report, trace) = match run_result {
        Ok(outcome) => outcome,
        Err(ScenarioExecutionError::RunControl(ScenarioRunControlError::Stopped)) => {
            return Err(scenario_stopped_error().with_appended_cleanup(shutdown_result));
        }
        Err(error) => {
            return Err(execution_error(&error).with_appended_cleanup(shutdown_result));
        }
    };
    if let Some(cleanup) = shutdown_result {
        return Err(cleanup);
    }
    let dto = run_outcome_dto(&report, &trace);
    session.last_run = Some(LastRun {
        scenario,
        report,
        trace,
    });
    Ok(dto)
}

/// Pauses an active scenario at its next deterministic step boundary. The
/// current step is allowed to settle atomically before the pause takes effect.
///
/// # Errors
///
/// Returns [`no_active_run_error`] when no scenario is running or a structured
/// run-control failure if the control mutex was poisoned/stopped concurrently.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_pause_loaded_scenario(app: AppHandle) -> Result<(), DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    if !session.running {
        return Err(no_active_run_error());
    }
    let control = session.run_control.as_ref().ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.lab.run_control_missing",
            "runtime",
            "fatal",
            false,
            "a Lab scenario is marked running but has no run-control handle",
        )
    })?;
    control.pause().map_err(|error| run_control_error(&error))?;
    session.paused = true;
    Ok(())
}

/// Resumes a scenario paused by [`lab_pause_loaded_scenario`].
///
/// # Errors
///
/// Returns [`no_active_run_error`] when no scenario is running or a structured
/// run-control failure if the control mutex was poisoned/stopped concurrently.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri command extraction requires AppHandle/request DTOs by value"
)]
#[tauri::command]
pub fn lab_resume_loaded_scenario(app: AppHandle) -> Result<(), DesktopErrorDto> {
    let lab_state = app.state::<LabAppState>();
    let mut session = lab_state.inner.lock().map_err(|_| poisoned_error())?;
    if !session.running {
        return Err(no_active_run_error());
    }
    let control = session.run_control.as_ref().ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.lab.run_control_missing",
            "runtime",
            "fatal",
            false,
            "a Lab scenario is marked running but has no run-control handle",
        )
    })?;
    control.resume().map_err(|error| run_control_error(&error))?;
    session.paused = false;
    Ok(())
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
    if session.running {
        return Err(already_running_error());
    }
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
    if session.running {
        return Err(already_running_error());
    }
    let runtime = ensure_runtime(&app, &mut session)?;
    runtime.stop_node(node_id)
}

/// Stops and releases every currently active Lab node (Block 42 "stop
/// ... controls"). If a scenario is running, this first requests cooperative
/// cancellation; the runner remains the sole owner of its own transport/node
/// cleanup, then this command's deferred shutdown request releases any
/// manually-started nodes left in the runtime.
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
    if session.running {
        let control = session.run_control.as_ref().ok_or_else(|| {
            DesktopErrorDto::new(
                "desktop.lab.run_control_missing",
                "runtime",
                "fatal",
                false,
                "a Lab scenario is marked running but has no run-control handle",
            )
        })?;
        control
            .request_stop()
            .map_err(|error| run_control_error(&error))?;
        session.paused = false;
        session.stop_all_after_run = true;
        return Ok(());
    }
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
