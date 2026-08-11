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

#[cfg(test)]
mod live_transport_proof_tests;
#[cfg(test)]
mod tests;
