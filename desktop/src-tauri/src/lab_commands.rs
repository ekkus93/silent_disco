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
use crate::lab::recorder::RecordedNotificationKind;
use crate::lab::recording::{RecordingIoError, ScenarioRecording};
use crate::lab::scenario::{
    AssertionOutcome, Scenario, ScenarioExecutionError, ScenarioOutcome, ScenarioParseError,
    ScenarioReport, ScenarioTrace, ScenarioValidationError, StepSettlement, load_scenario_json,
    run_scenario_with_trace,
};
use crate::lab::{LabNodeId, LabRuntime};
use crate::lab_dto::{
    LabAdvanceTimeRequest, LabAssertionResultDto, LabFileOutcomeDto, LabLinkDto, LabNodeDto,
    LabRunOutcomeDto, LabScenarioSummaryDto, LabStartNodeRequest, LabStateDto, LabStepResultDto,
    LabStopNodeRequest, LabTimelineEntryDto,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

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

fn poisoned_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.session_poisoned",
        "runtime",
        "fatal",
        false,
        "the Lab session mutex was poisoned",
    )
}

fn already_running_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.already_running",
        "runtime",
        "error",
        false,
        "a Lab scenario is already running; wait for it to finish or stop every node first",
    )
}

fn no_scenario_loaded_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.no_scenario_loaded",
        "runtime",
        "error",
        false,
        "no Lab scenario is currently open",
    )
}

fn no_run_to_export_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.no_run_to_export",
        "runtime",
        "error",
        false,
        "no completed Lab scenario run is available to export",
    )
}

fn invalid_node_id_error(raw: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.invalid_node_id",
        "validation",
        "error",
        false,
        &format!("'{raw}' is not a Lab node identifier this session issued"),
    )
}

/// Mirrors `platform::file_picker::TauriAudioFileDialog::pick_file`'s own
/// error-shape discipline: `FilePath::into_path`'s error type lives in
/// `tauri_plugin_fs`, a transitive (not direct) dependency of this crate,
/// so it is never named explicitly here -- only its `Display` output.
fn path_unavailable_error(error: &impl std::fmt::Display) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.path_unavailable",
        "platform",
        "error",
        false,
        &format!("selected path could not be represented safely: {error}"),
    )
}

fn ensure_runtime(
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

fn node_dto(runtime: &LabRuntime, node_id: LabNodeId) -> Option<LabNodeDto> {
    let clock = runtime.node_clock(node_id)?;
    Some(LabNodeDto {
        node_id: node_id.as_u32().to_string(),
        offset_ms: clock.offset_ms().to_string(),
        drift_ppm: clock.drift_ppm().to_string(),
    })
}

fn scenario_summary_dto(scenario: &Scenario) -> LabScenarioSummaryDto {
    LabScenarioSummaryDto {
        schema_version: scenario.schema_version,
        seed: scenario.seed.to_string(),
        node_ids: scenario
            .nodes
            .iter()
            .map(|node| node.id.as_str().to_owned())
            .collect(),
        link_count: u32::try_from(scenario.links.len()).unwrap_or(u32::MAX),
        fixture_count: u32::try_from(scenario.fixtures.len()).unwrap_or(u32::MAX),
        step_count: u32::try_from(scenario.steps.len()).unwrap_or(u32::MAX),
        assertion_count: u32::try_from(scenario.assertions.len()).unwrap_or(u32::MAX),
        timeout_ms: scenario.timeout_ms.to_string(),
        links: scenario
            .links
            .iter()
            .map(|link| LabLinkDto {
                from: link.from.as_str().to_owned(),
                to: link.to.as_str().to_owned(),
                latency_ms: link.latency_ms.to_string(),
                jitter_ms: link.jitter_ms.to_string(),
                loss_permille: link.loss_permille,
            })
            .collect(),
    }
}

fn bounded_summary_text(value: &str) -> String {
    if value.chars().count() <= MAX_SUMMARY_CHARS {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(MAX_SUMMARY_CHARS).collect();
    truncated.push('…');
    truncated
}

fn timeline_entry(
    node: &str,
    sequence: u64,
    kind: &RecordedNotificationKind,
) -> LabTimelineEntryDto {
    let (kind_name, summary) = match kind {
        RecordedNotificationKind::Snapshot { revision, summary } => (
            "snapshot",
            format!(
                "revision {revision}: host={} listener={} playback={}",
                summary.host_lifecycle, summary.listener_lifecycle, summary.playback_state
            ),
        ),
        RecordedNotificationKind::Effect { name } => ("effect", name.clone()),
        RecordedNotificationKind::TransportEffect { name } => ("transportEffect", name.clone()),
        RecordedNotificationKind::StorageEffect { name } => ("storageEffect", name.clone()),
        RecordedNotificationKind::Error {
            code,
            severity,
            message,
            ..
        } => ("error", format!("{code} ({severity}): {message}")),
        RecordedNotificationKind::Diagnostic { name, fields } => (
            "diagnostic",
            format!(
                "{name}: {}",
                fields
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
    };
    LabTimelineEntryDto {
        node: node.to_owned(),
        sequence: sequence.to_string(),
        kind: kind_name.to_owned(),
        summary: bounded_summary_text(&summary),
    }
}

fn run_outcome_dto(report: &ScenarioReport, trace: &ScenarioTrace) -> LabRunOutcomeDto {
    let outcome = match report.outcome {
        ScenarioOutcome::Completed => "completed",
        ScenarioOutcome::TimedOut => "timedOut",
        ScenarioOutcome::ExecutionError => "executionError",
    };
    let mut timeline = Vec::new();
    let mut truncated = false;
    for (node, entries) in &trace.node_notifications {
        for entry in entries.iter().take(MAX_TIMELINE_ENTRIES_PER_NODE) {
            timeline.push(timeline_entry(node, entry.sequence, &entry.kind));
        }
        if entries.len() > MAX_TIMELINE_ENTRIES_PER_NODE {
            truncated = true;
        }
    }
    LabRunOutcomeDto {
        outcome: outcome.to_owned(),
        final_time_ms: report.final_time_ms.to_string(),
        step_results: report
            .step_results
            .iter()
            .map(|step| LabStepResultDto {
                index: u32::try_from(step.index).unwrap_or(u32::MAX),
                at_ms: step.at_ms.to_string(),
                node: step.node.as_str().to_owned(),
                submit_error: step.submit_error.clone(),
                settlement: match step.settlement {
                    StepSettlement::Settled => "settled".to_owned(),
                    StepSettlement::TimedOut => "timedOut".to_owned(),
                },
            })
            .collect(),
        assertion_results: report
            .assertion_results
            .iter()
            .map(|assertion| LabAssertionResultDto {
                kind: assertion.kind.clone(),
                node: assertion.node.as_str().to_owned(),
                by_ms: assertion.by_ms.to_string(),
                outcome: match assertion.outcome {
                    AssertionOutcome::Held => "held".to_owned(),
                    AssertionOutcome::TimedOut => "timedOut".to_owned(),
                },
            })
            .collect(),
        timeline,
        timeline_truncated: truncated,
    }
}

fn state_dto(runtime: &LabRuntime, session: &LabSessionState) -> LabStateDto {
    let nodes = runtime
        .node_ids()
        .into_iter()
        .filter_map(|id| node_dto(runtime, id))
        .collect();
    LabStateDto {
        now_ms: runtime.now().get().to_string(),
        running: session.running,
        nodes,
        loaded_scenario: session
            .loaded
            .as_ref()
            .map(|loaded| scenario_summary_dto(&loaded.scenario)),
        last_run: session
            .last_run
            .as_ref()
            .map(|last_run| run_outcome_dto(&last_run.report, &last_run.trace)),
    }
}

fn parse_error(error: &ScenarioParseError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_parse_failed",
        "validation",
        "error",
        false,
        &error.to_string(),
    )
}

fn validation_error(error: &ScenarioValidationError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_invalid",
        "validation",
        "error",
        false,
        &error.to_string(),
    )
}

fn execution_error(error: &ScenarioExecutionError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_execution_failed",
        "runtime",
        "error",
        false,
        &error.to_string(),
    )
}

fn recording_io_error(error: &RecordingIoError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.recording_io_failed",
        "storage",
        "error",
        true,
        &error.to_string(),
    )
}

/// Reads a user-selected scenario file's bytes, rejecting an oversized file
/// from filesystem metadata *before* reading it into memory (Block 43.1/43.2
/// "bounded payload"). `load_scenario_json` also rejects an oversized
/// document, but only after the whole thing has already been read into a
/// `Vec<u8>` -- checking here first means a user accidentally selecting a
/// huge file cannot balloon this process's memory even transiently.
fn read_bounded_scenario_file(path: &std::path::Path) -> Result<Vec<u8>, DesktopErrorDto> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.scenario_read_failed",
            "platform",
            "error",
            true,
            &format!("could not read the selected scenario file: {error}"),
        )
    })?;
    if metadata.len() > crate::lab::scenario::MAX_SCENARIO_FILE_BYTES as u64 {
        return Err(DesktopErrorDto::new(
            "desktop.lab.scenario_too_large",
            "validation",
            "error",
            false,
            &format!(
                "the selected scenario file exceeds the {} byte limit",
                crate::lab::scenario::MAX_SCENARIO_FILE_BYTES
            ),
        ));
    }
    std::fs::read(path).map_err(|error| {
        DesktopErrorDto::new(
            "desktop.lab.scenario_read_failed",
            "platform",
            "error",
            true,
            &format!("could not read the selected scenario file: {error}"),
        )
    })
}

fn parse_and_validate(bytes: &[u8]) -> Result<Scenario, DesktopErrorDto> {
    let scenario = load_scenario_json(bytes).map_err(|error| parse_error(&error))?;
    scenario
        .validate()
        .map_err(|error| validation_error(&error))?;
    Ok(scenario)
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
