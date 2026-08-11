//! Lab scenario schema and execution surface.
//!
//! Block 40's original deterministic runner is preserved in
//! `scenario_legacy.rs` and remains the implementation for scenarios that do
//! not request live links. Scenarios that declare a link, or use one of the
//! two live-flow commands (`configureHost` / `selectSession`), are executed by
//! `live_runner` against the real virtual transport/fault stack. This keeps
//! the already-gated Block 40 behavior available as a regression baseline
//! while making link declarations operational instead of decorative.

#[path = "scenario_legacy.rs"]
mod legacy;
mod live_runner;
mod live_transport;
mod schema;

pub(crate) use legacy::{
    AssertionOutcome, AssertionResult, ClockAdvance, ScenarioExecutionError, ScenarioOutcome,
    ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
};
pub(crate) use live_runner::{run_scenario, run_scenario_with_trace};
pub(crate) use schema::{
    FixtureId, MAX_ASSERTIONS, MAX_FIXTURES, MAX_ID_BYTES, MAX_LINKS, MAX_LINK_JITTER_MS,
    MAX_LINK_LATENCY_MS, MAX_LOSS_PERMILLE, MAX_NODES, MAX_SCENARIO_DURATION_MS,
    MAX_SCENARIO_FILE_BYTES, MAX_STEPS, NodeId, SCHEMA_VERSION, Scenario, ScenarioAction,
    ScenarioAssertion, ScenarioClock, ScenarioFixture, ScenarioLifecycleTarget, ScenarioLink,
    ScenarioParseError, ScenarioStep, ScenarioValidationError, TerminationPolicy,
    load_scenario_json,
};
