//! Lab scenario schema, deterministic runner, assertions, and live transport.
//!
//! Scenario links are operational: actor-emitted platform and transport
//! effects are executed against the real virtual transport/fault stack. The
//! public schema remains version 1 and keeps the original Block-40 bounds and
//! assertion vocabulary, with `configureHost` and `selectSession` added so a
//! complete host/discover/join/approve path is expressible without synthetic
//! transport-success events.

mod assertions;
mod commands;
mod live_runner;
mod live_transport;
mod report;
mod schema;

use crate::dto::DesktopErrorDto;
use crate::lab::clock::LabNodeClock;
use crate::lab::{LabNodeId, LabRuntime};
use crate::platform::identity::DesktopIdentity;
use silent_disco_core::runtime::CoreActorHandle;
use std::sync::Arc;

#[cfg(test)]
pub(crate) use assertions::evaluate_assertion;
#[cfg(test)]
pub(crate) use live_runner::run_scenario;
pub(crate) use live_runner::run_scenario_with_trace;
pub(crate) use report::{
    AssertionOutcome, AssertionResult, ClockAdvance, ScenarioExecutionError, ScenarioOutcome,
    ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
};
pub(crate) use schema::{
    FixtureId, MAX_ASSERTIONS, MAX_NODES, MAX_SCENARIO_FILE_BYTES, MAX_STEPS, NodeId, Scenario,
    ScenarioAction, ScenarioAssertion, ScenarioClock, ScenarioLifecycleTarget, ScenarioLink,
    ScenarioParseError, ScenarioValidationError, load_scenario_json,
};

/// Returns one scenario node's actor, identity, and clock without collapsing
/// Lab-registry failure into an ordinary missing-node result.
///
/// # Errors
///
/// Returns a fatal runtime error if the Lab node registry is poisoned, or a
/// structured missing-node error if the requested node no longer exists.
pub(in crate::lab::scenario) fn scenario_node_parts(
    lab: &LabRuntime,
    node_id: LabNodeId,
) -> Result<(CoreActorHandle, DesktopIdentity, Arc<LabNodeClock>), DesktopErrorDto> {
    let nodes = lab.nodes.lock().map_err(|_| {
        DesktopErrorDto::new(
            "desktop.lab.state_poisoned",
            "runtime",
            "fatal",
            false,
            "the Lab runtime's node registry mutex was poisoned",
        )
    })?;
    let node = nodes.get(&node_id).ok_or_else(|| {
        DesktopErrorDto::new(
            "desktop.lab.unknown_node",
            "runtime",
            "error",
            false,
            &format!(
                "Lab node {} does not exist or was already stopped",
                node_id.as_u32()
            ),
        )
    })?;
    Ok((node.handle(), node.identity().clone(), node.clock()))
}

#[cfg(test)]
mod live_transport_proof_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_recording_tests;
