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
        "a Lab scenario is already running; wait for it to finish or stop every node first",
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
