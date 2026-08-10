//! Replay: detecting an incompatible recording rather than silently
//! reinterpreting it (Block 40.2/spec section 29.5).
//!
//! Scope for Block 40: a [`ScenarioRecording`] pairs the schema version and
//! seed a scenario run was captured under with the [`super::scenario::ScenarioReport`]
//! that run produced. [`replay`] re-executes the *same* [`super::scenario::Scenario`]
//! document through the *same* [`super::LabRuntime::run_scenario`]-style
//! path (`super::scenario::run_scenario`) after checking that the document
//! being replayed against still declares the recorded schema version and
//! seed -- refusing outright, rather than silently reinterpreting, when it
//! does not (spec 29.5: "Replay must detect version incompatibility rather
//! than silently reinterpret an old recording").
//!
//! **Deliberately out of scope for Block 40**, and left to Block 41 (whose
//! own TODO section already lists these as its own bullets, not
//! duplicated here): persisting a recording to disk (protocol/core version
//! stamping, packet metadata/payload hashes, secret redaction), comparing a
//! fresh replay's report against the original to compute a bounded
//! divergence diff, and a conversion tool for an older, structurally
//! incompatible recording. Block 40's own "deterministic report" guarantee
//! (`super::scenario::tests`) is what makes a future divergence diff
//! meaningful in the first place: two runs of the same scenario and seed
//! already produce an equal [`super::scenario::ScenarioReport`], so any
//! future replay divergence is real, not incidental nondeterminism.

use super::LabRuntime;
use super::scenario::{Scenario, ScenarioExecutionError, ScenarioReport, run_scenario};
use std::fmt;

/// What a scenario run was recorded under, plus the report it produced.
/// Test/in-process only for Block 40 -- see the module doc comment.
#[derive(Debug, Clone)]
pub(crate) struct ScenarioRecording {
    pub(crate) schema_version: u32,
    pub(crate) seed: u64,
    pub(crate) report: ScenarioReport,
}

impl ScenarioRecording {
    #[must_use]
    pub(crate) fn capture(scenario: &Scenario, report: ScenarioReport) -> Self {
        Self {
            schema_version: scenario.schema_version,
            seed: scenario.seed,
            report,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ReplayError {
    SchemaVersionMismatch { recorded: u32, replaying: u32 },
    SeedMismatch { recorded: u64, replaying: u64 },
    Execution(ScenarioExecutionError),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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

/// Re-executes `scenario` and checks the fresh run against what
/// `recording` was captured under -- never silently reinterprets an
/// incompatible recording (spec 29.5).
///
/// # Errors
///
/// Returns [`ReplayError::SchemaVersionMismatch`] or
/// [`ReplayError::SeedMismatch`] when `scenario` no longer matches what
/// `recording` was captured under, or [`ReplayError::Execution`] when the
/// fresh run itself fails.
pub(crate) fn replay(
    lab: &LabRuntime,
    scenario: &Scenario,
    recording: &ScenarioRecording,
) -> Result<ScenarioReport, ReplayError> {
    if recording.schema_version != scenario.schema_version {
        return Err(ReplayError::SchemaVersionMismatch {
            recorded: recording.schema_version,
            replaying: scenario.schema_version,
        });
    }
    if recording.seed != scenario.seed {
        return Err(ReplayError::SeedMismatch {
            recorded: recording.seed,
            replaying: scenario.seed,
        });
    }
    run_scenario(lab, scenario).map_err(ReplayError::Execution)
}

#[cfg(test)]
mod tests;
