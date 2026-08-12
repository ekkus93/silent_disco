//! Bounded scenario file read and parse/validate helpers (Block 42
//! "scenario open ... through restricted dialogs"; Block 43.1/43.2
//! "bounded payload"). The `lab_open_scenario_file`/`lab_save_scenario_file`
//! Tauri commands that use these live in `mod.rs` -- see that module's own
//! doc comment for why every `#[tauri::command]`-annotated entry point is
//! defined directly there rather than re-exported from a submodule.

use super::errors::{
    edited_scenario_too_large_error, invalid_link_index_error, parse_error, scenario_encode_error,
    stale_link_selection_error, validation_error,
};
use crate::dto::DesktopErrorDto;
use crate::lab::scenario::{Scenario, load_scenario_json};
use serde_json::{Map, Value};

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

/// Rewrites one target node's initial receive-fault profile, then reparses and
/// revalidates the complete document before returning it.
///
/// The live transport applies latency/jitter/loss per receiving node. The
/// scenario schema therefore permits multiple inbound links only when they
/// share one profile. Selecting any one such declaration updates every link
/// targeting the same node atomically so a previously valid multi-inbound
/// scenario never has to pass through an invalid intermediate state.
///
/// The loaded scenario's original bytes remain byte-for-byte stable until an
/// operator actually edits a fault. Once edited, the saveable representation
/// becomes a compact JSON encoding of the same bounded document plus the new
/// validated values. Re-running the canonical parser/validator after the
/// rewrite prevents this UI-facing helper from becoming a second source of
/// scenario legality rules.
pub(super) fn rewrite_link_faults(
    raw_bytes: &[u8],
    link_index: u32,
    expected_from: &str,
    expected_to: &str,
    latency_ms: u64,
    jitter_ms: u64,
    loss_permille: u16,
) -> Result<(Scenario, Vec<u8>), DesktopErrorDto> {
    let mut value: Value = serde_json::from_slice(raw_bytes).map_err(|error| parse_error(
        &crate::lab::scenario::ScenarioParseError::NotUtf8OrJson(error),
    ))?;
    let links = value
        .get_mut("links")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid_link_index_error(link_index))?;
    let index = usize::try_from(link_index).map_err(|_| invalid_link_index_error(link_index))?;
    let selection_matches = {
        let link = links
            .get(index)
            .and_then(Value::as_object)
            .ok_or_else(|| invalid_link_index_error(link_index))?;
        string_field(link, "from") == Some(expected_from)
            && string_field(link, "to") == Some(expected_to)
    };
    if !selection_matches {
        return Err(stale_link_selection_error(link_index));
    }

    for candidate in links.iter_mut() {
        let Some(candidate) = candidate.as_object_mut() else {
            continue;
        };
        if string_field(candidate, "to") != Some(expected_to) {
            continue;
        }
        candidate.insert("latencyMs".to_owned(), Value::from(latency_ms));
        candidate.insert("jitterMs".to_owned(), Value::from(jitter_ms));
        candidate.insert("lossPermille".to_owned(), Value::from(loss_permille));
    }

    let bytes = serde_json::to_vec(&value).map_err(|error| scenario_encode_error(&error))?;
    if bytes.len() > crate::lab::scenario::MAX_SCENARIO_FILE_BYTES {
        return Err(edited_scenario_too_large_error());
    }
    let scenario = parse_and_validate(&bytes)?;
    Ok((scenario, bytes))
}

fn string_field<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    object.get(name).and_then(Value::as_str)
}
