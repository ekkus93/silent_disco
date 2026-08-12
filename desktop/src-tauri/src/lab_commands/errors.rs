//! Structured [`DesktopErrorDto`] constructors for every Lab Mode command
//! failure this module surfaces (Block 42).

use crate::dto::DesktopErrorDto;
use crate::lab::recording::RecordingIoError;
use crate::lab::scenario::{ScenarioExecutionError, ScenarioParseError, ScenarioValidationError};

pub(super) fn poisoned_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.session_poisoned",
        "runtime",
        "fatal",
        false,
        "the Lab session mutex was poisoned",
    )
}

pub(super) fn already_running_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.already_running",
        "runtime",
        "error",
        false,
        "a Lab scenario is already running; pause, resume, or stop that run before starting another",
    )
}

pub(super) fn no_active_run_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.no_active_run",
        "runtime",
        "error",
        false,
        "no Lab scenario run is currently active",
    )
}

pub(super) fn run_control_error(
    error: &crate::lab::scenario::ScenarioRunControlError,
) -> DesktopErrorDto {
    match error {
        crate::lab::scenario::ScenarioRunControlError::Stopped => DesktopErrorDto::new(
            "desktop.lab.run_already_stopping",
            "runtime",
            "info",
            false,
            "the Lab scenario is already stopping",
        ),
        crate::lab::scenario::ScenarioRunControlError::Poisoned => DesktopErrorDto::new(
            "desktop.lab.run_control_failed",
            "runtime",
            "fatal",
            false,
            &error.to_string(),
        ),
    }
}

pub(super) fn scenario_stopped_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_stopped",
        "runtime",
        "info",
        false,
        "the Lab scenario was stopped by the operator and cleaned up",
    )
}

pub(super) fn run_worker_error(error: &impl std::fmt::Display) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.run_worker_failed",
        "runtime",
        "fatal",
        false,
        &format!("the Lab scenario worker did not complete normally: {error}"),
    )
}

pub(super) fn no_scenario_loaded_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.no_scenario_loaded",
        "runtime",
        "error",
        false,
        "no Lab scenario is currently open",
    )
}

pub(super) fn no_run_to_export_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.no_run_to_export",
        "runtime",
        "error",
        false,
        "no completed Lab scenario run is available to export",
    )
}

pub(super) fn invalid_node_id_error(raw: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.invalid_node_id",
        "validation",
        "error",
        false,
        &format!("'{raw}' is not a Lab node identifier this session issued"),
    )
}

pub(super) fn invalid_fault_field_error(field: &str, raw: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.invalid_fault_value",
        "validation",
        "error",
        false,
        &format!("{field} must be a non-negative integer; received '{raw}'"),
    )
}

pub(super) fn invalid_link_index_error(index: u32) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.invalid_link_index",
        "validation",
        "error",
        false,
        &format!("Lab scenario link index {index} does not exist"),
    )
}

pub(super) fn stale_link_selection_error(index: u32) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.stale_link_selection",
        "validation",
        "error",
        true,
        &format!(
            "Lab scenario link {index} changed before the fault edit was applied; refresh the scenario state and try again"
        ),
    )
}

pub(super) fn scenario_encode_error(error: &serde_json::Error) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_encode_failed",
        "runtime",
        "fatal",
        false,
        &format!("the validated Lab scenario could not be re-encoded after editing faults: {error}"),
    )
}

pub(super) fn edited_scenario_too_large_error() -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_too_large",
        "validation",
        "error",
        false,
        &format!(
            "the edited scenario exceeds the {} byte limit",
            crate::lab::scenario::MAX_SCENARIO_FILE_BYTES
        ),
    )
}

/// Mirrors `platform::file_picker::TauriAudioFileDialog::pick_file`'s own
/// error-shape discipline: `FilePath::into_path`'s error type lives in
/// `tauri_plugin_fs`, a transitive (not direct) dependency of this crate,
/// so it is never named explicitly here -- only its `Display` output.
pub(super) fn path_unavailable_error(error: &impl std::fmt::Display) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.path_unavailable",
        "platform",
        "error",
        false,
        &format!("selected path could not be represented safely: {error}"),
    )
}

pub(super) fn parse_error(error: &ScenarioParseError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_parse_failed",
        "validation",
        "error",
        false,
        &error.to_string(),
    )
}

pub(super) fn validation_error(error: &ScenarioValidationError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_invalid",
        "validation",
        "error",
        false,
        &error.to_string(),
    )
}

pub(super) fn execution_error(error: &ScenarioExecutionError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.scenario_execution_failed",
        "runtime",
        "error",
        false,
        &error.to_string(),
    )
}

pub(super) fn recording_io_error(error: &RecordingIoError) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.lab.recording_io_failed",
        "storage",
        "error",
        true,
        &error.to_string(),
    )
}
