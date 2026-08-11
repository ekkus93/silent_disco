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

pub(crate) use assertions::evaluate_assertion;
pub(crate) use live_runner::{run_scenario, run_scenario_with_trace};
pub(crate) use report::{
    AssertionOutcome, AssertionResult, ClockAdvance, ScenarioExecutionError, ScenarioOutcome,
    ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
};
pub(crate) use schema::{
    FixtureId, MAX_ASSERTIONS, MAX_FIXTURES, MAX_ID_BYTES, MAX_LINKS, MAX_LINK_JITTER_MS,
    MAX_LINK_LATENCY_MS, MAX_LOSS_PERMILLE, MAX_NODES, MAX_SCENARIO_DURATION_MS,
    MAX_SCENARIO_FILE_BYTES, MAX_STEPS, NodeId, SCHEMA_VERSION, Scenario, ScenarioAction,
    ScenarioAssertion, ScenarioClock, ScenarioFixture, ScenarioLifecycleTarget, ScenarioLink,
    ScenarioParseError, ScenarioStep, ScenarioValidationError, TerminationPolicy,
    load_scenario_json,
};

#[cfg(test)]
mod tests;
