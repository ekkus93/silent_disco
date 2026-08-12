//! Replay: re-executing a persisted [`super::recording::ScenarioRecording`]
//! and reporting how (if at all) it diverges from what actually happened
//! (Block 41.2; spec section 29.5).
//!
//! Block 40 built a first-cut version of this that only checked a
//! scenario's own `schemaVersion`/`seed` before re-running it. This module
//! is Block 41's extension: it now checks the recorded on-disk format
//! version too, and -- the genuinely new capability -- compares the fresh
//! run's [`super::scenario::ScenarioReport`] against the one captured in
//! the recording, surfacing the first point they disagree
//! ([`super::recording::Divergence`]) rather than only reporting pass/fail.
//! See `super::recording`'s own module doc comment ("Which versions gate
//! replay") for exactly which recorded versions block replay outright
//! versus which are informational -- that distinction is what makes "replay
//! against a later core build" (this block's own acceptance criterion) and
//! "detect version incompatibility" (spec 29.5) simultaneously true rather
//! than contradictory.

use super::LabRuntime;
use super::recording::{
    Divergence, RECORDING_FORMAT_VERSION, ScenarioRecording, first_divergence,
    first_trace_divergence,
};
use super::scenario::{Scenario, ScenarioExecutionError, ScenarioReport, run_scenario_with_trace};
use std::fmt;

#[derive(Debug)]
pub(crate) enum ReplayError {
    RecordingFormatVersionMismatch { recorded: u32, current: u32 },
    SchemaVersionMismatch { recorded: u32, replaying: u32 },
    SeedMismatch { recorded: u64, replaying: u64 },
    Execution(ScenarioExecutionError),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordingFormatVersionMismatch { recorded, current } => write!(
                formatter,
                "recording was saved under recordingFormatVersion {recorded}, but this build \
                 understands {current}; refusing to reinterpret it -- a version-specific \
                 conversion tool would be a separate, explicitly versioned addition, not an \
                 automatic reinterpretation here"
            ),
            Self::SchemaVersionMismatch {
                recorded,
                replaying,
            } => write!(
                formatter,
                "recording was captured under schemaVersion {recorded}, but the scenario being \
                 replayed against declares {replaying}; refusing to reinterpret it"
            ),
            Self::SeedMismatch {
                recorded,
                replaying,
            } => write!(
                formatter,
                "recording was captured with seed {recorded}, but the scenario being replayed \
                 against declares seed {replaying}; refusing to reinterpret it"
            ),
            Self::Execution(error) => write!(formatter, "replay execution failed: {error}"),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Everything a replay run produces: the fresh report itself, whether it
/// diverges from what was recorded, and both the recorded and current
/// protocol/core versions for a caller (a future Block 42 UI, or a
/// maintainer reading test output) to present alongside the divergence --
/// see `super::recording`'s own doc comment for why these two versions are
/// informational rather than a hard gate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReplayOutcome {
    pub(crate) report: ScenarioReport,
    pub(crate) divergence: Option<Divergence>,
    pub(crate) recorded_protocol_version: u16,
    pub(crate) current_protocol_version: u16,
    pub(crate) recorded_core_version: super::recording::RecordedCoreVersion,
    pub(crate) current_core_version: super::recording::RecordedCoreVersion,
}

/// Re-executes `scenario` and compares the fresh run against `recording` --
/// never silently reinterprets a structurally incompatible recording (spec
/// 29.5), but a differing protocol/core version alone is not refused (see
/// this module's own doc comment).
///
/// # Errors
///
/// Returns [`ReplayError::RecordingFormatVersionMismatch`],
/// [`ReplayError::SchemaVersionMismatch`], or [`ReplayError::SeedMismatch`]
/// when `scenario`/the current build no longer structurally matches what
/// `recording` was captured under, or [`ReplayError::Execution`] when the
/// fresh run itself fails.
pub(crate) fn replay(
    lab: &LabRuntime,
    scenario: &Scenario,
    recording: &ScenarioRecording,
) -> Result<ReplayOutcome, ReplayError> {
    if recording.recording_format_version != RECORDING_FORMAT_VERSION {
        return Err(ReplayError::RecordingFormatVersionMismatch {
            recorded: recording.recording_format_version,
            current: RECORDING_FORMAT_VERSION,
        });
    }
    if recording.scenario_schema_version != scenario.schema_version {
        return Err(ReplayError::SchemaVersionMismatch {
            recorded: recording.scenario_schema_version,
            replaying: scenario.schema_version,
        });
    }
    if recording.seed != scenario.seed {
        return Err(ReplayError::SeedMismatch {
            recorded: recording.seed,
            replaying: scenario.seed,
        });
    }

    let (report, trace) = run_scenario_with_trace(lab, scenario).map_err(ReplayError::Execution)?;
    let divergence = first_divergence(&recording.report, &report)
        .or_else(|| first_trace_divergence(&recording.trace, &trace));
    let current_core_version =
        super::recording::RecordedCoreVersion::from(silent_disco_core::core_version());

    Ok(ReplayOutcome {
        report,
        divergence,
        recorded_protocol_version: recording.protocol_version,
        current_protocol_version: silent_disco_core::protocol::PROTOCOL_VERSION,
        recorded_core_version: recording.core_version,
        current_core_version,
    })
}

#[cfg(test)]
mod tests;
