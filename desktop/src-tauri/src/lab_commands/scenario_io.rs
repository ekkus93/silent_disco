//! Bounded scenario file read and parse/validate helpers (Block 42
//! "scenario open ... through restricted dialogs"; Block 43.1/43.2
//! "bounded payload"). The `lab_open_scenario_file`/`lab_save_scenario_file`
//! Tauri commands that use these live in `mod.rs` -- see that module's own
//! doc comment for why every `#[tauri::command]`-annotated entry point is
//! defined directly there rather than re-exported from a submodule.

use super::errors::{parse_error, validation_error};
use crate::dto::DesktopErrorDto;
use crate::lab::scenario::{Scenario, load_scenario_json};

/// Reads a user-selected scenario file's bytes, rejecting an oversized file
/// from filesystem metadata *before* reading it into memory (Block 43.1/43.2
/// "bounded payload"). `load_scenario_json` also rejects an oversized
/// document, but only after the whole thing has already been read into a
/// `Vec<u8>` -- checking here first means a user accidentally selecting a
/// huge file cannot balloon this process's memory even transiently.
pub(super) fn read_bounded_scenario_file(
    path: &std::path::Path,
) -> Result<Vec<u8>, DesktopErrorDto> {
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

pub(super) fn parse_and_validate(bytes: &[u8]) -> Result<Scenario, DesktopErrorDto> {
    let scenario = load_scenario_json(bytes).map_err(|error| parse_error(&error))?;
    scenario
        .validate()
        .map_err(|error| validation_error(&error))?;
    Ok(scenario)
}
