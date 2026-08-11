use super::assertions::{assertion_deadline, assertion_kind, assertion_node, evaluate_assertion};
use super::commands::{action_revision_delta, current_revision, submit_action};
use super::live_transport::{LiveScenarioObserver, LiveTransportDriver};
use super::{
    AssertionOutcome, AssertionResult, ClockAdvance, NodeId, Scenario, ScenarioExecutionError,
    ScenarioOutcome, ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
};
use crate::lab::recorder::{RecordedNotification, RecordedNotificationKind, ScenarioRecorder};
use crate::lab::{LabNodeId, LabRuntime};
use silent_disco_core::runtime::{CoreActorHandle, SnapshotRevision};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const STEP_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const STEP_SETTLE_POLL: Duration = Duration::from_millis(20);

pub(crate) fn run_scenario(
    lab: &LabRuntime,
    scenario: &Scenario,
) -> Result<ScenarioReport, ScenarioExecutionError> {
    run_scenario_with_trace(lab, scenario).map(|(report, _trace)| report)
}

pub(crate) fn run_scenario_with_trace(
    lab: &LabRuntime,
    scenario: &Scenario,
) -> Result<(ScenarioReport, ScenarioTrace), ScenarioExecutionError> {
    scenario
        .validate()
        .map_err(ScenarioExecutionError::Validation)?;
    run_live_scenario(lab, scenario)
}

fn run_live_scenario(
    lab: &LabRuntime,
    scenario: &Scenario,
) -> Result<(ScenarioReport, ScenarioTrace), ScenarioExecutionError> {
    let mut lab_node_ids: HashMap<&str, LabNodeId> = HashMap::new();
    let mut recorders: HashMap<&str, Arc<ScenarioRecorder>> = HashMap::new();
    let mut effect_receivers = HashMap::new();

    for node in &scenario.nodes {
        let clock = scenario
            .clocks
            .get(node.id.as_str())
            .copied()
            .unwrap_or(super::ScenarioClock {
                offset_ms: 0,
                drift_ppm: 0,
            });
        let recorder = ScenarioRecorder::new();
        let (observer, effect_receiver) = LiveScenarioObserver::new(Arc::clone(&recorder));
        let lab_node_id = lab
            .start_node_with_clock_and_observer(clock.offset_ms, clock.drift_ppm, observer)
            .map_err(ScenarioExecutionError::Lab)?;
        lab_node_ids.insert(node.id.as_str(), lab_node_id);
        recorders.insert(node.id.as_str(), recorder);
        effect_receivers.insert(node.id.clone(), effect_receiver);
    }

    let mut driver = LiveTransportDriver::new(lab, scenario, &lab_node_ids, effect_receivers)
        .map_err(ScenarioExecutionError::Lab)?;
    let mut clock_advances = Vec::new();
    let run_result = execute_steps_and_assertions(
        lab,
        scenario,
        &lab_node_ids,
        &recorders,
        &mut driver,
        &mut clock_advances,
    );

    let node_notifications = collect_notifications(scenario, &recorders);
    drop(driver);
    let teardown_ok = stop_scenario_nodes(lab, &lab_node_ids);

    let mut report = run_result?;
    if !teardown_ok && report.outcome == ScenarioOutcome::Completed {
        report.outcome = ScenarioOutcome::ExecutionError;
    }
    Ok((
        report,
        ScenarioTrace {
            clock_advances,
            node_notifications,
        },
    ))
}

fn execute_steps_and_assertions(
    lab: &LabRuntime,
    scenario: &Scenario,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    recorders: &HashMap<&str, Arc<ScenarioRecorder>>,
    driver: &mut LiveTransportDriver,
    clock_advances: &mut Vec<ClockAdvance>,
) -> Result<ScenarioReport, ScenarioExecutionError> {
    let mut current_ms = lab.now().get();
    let mut step_results = Vec::with_capacity(scenario.steps.len());

    for (index, step) in scenario.steps.iter().enumerate() {
        if step.at_ms >= scenario.timeout_ms {
            break;
        }
        advance_to(
            lab,
            driver,
            &mut current_ms,
            step.at_ms,
            clock_advances,
        )?;

        let lab_node_id = node_id_for(step.node.as_str(), &step.node, lab_node_ids)?;
        let handle = node_handle(lab, lab_node_id, &step.node)?;
        let recorder = recorders
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        let revision_before = current_revision(&handle);
        let sequence_before = recorder.next_sequence();
        let submit_error = submit_action(
            lab,
            scenario,
            &handle,
            lab_node_id,
            lab_node_ids,
            &step.action,
        )?;
        driver.pump().map_err(ScenarioExecutionError::Lab)?;

        let settlement = if submit_error.is_some() {
            StepSettlement::Settled
        } else {
            wait_for_step_settled(
                &handle,
                recorder,
                driver,
                revision_before,
                sequence_before,
                action_revision_delta(&step.action),
            )?
        };
        step_results.push(StepResult {
            index,
            at_ms: step.at_ms,
            node: step.node.clone(),
            submit_error,
            settlement,
        });
    }

    advance_to(
        lab,
        driver,
        &mut current_ms,
        scenario.timeout_ms,
        clock_advances,
    )?;
    let (outcome, assertion_results) = evaluate_assertions(
        lab,
        scenario,
        lab_node_ids,
        recorders,
    )?;

    Ok(ScenarioReport {
        schema_version: scenario.schema_version,
        seed: scenario.seed,
        outcome,
        final_time_ms: current_ms,
        step_results,
        assertion_results,
    })
}

fn advance_to(
    lab: &LabRuntime,
    driver: &mut LiveTransportDriver,
    current_ms: &mut u64,
    target_ms: u64,
    clock_advances: &mut Vec<ClockAdvance>,
) -> Result<(), ScenarioExecutionError> {
    if target_ms <= *current_ms {
        driver.pump().map_err(ScenarioExecutionError::Lab)?;
        return Ok(());
    }
    let delta = target_ms - *current_ms;
    let resulting = lab.advance(delta).map_err(ScenarioExecutionError::Lab)?;
    *current_ms = resulting.get();
    clock_advances.push(ClockAdvance {
        requested_delta_ms: delta,
        resulting_now_ms: *current_ms,
    });
    driver.pump().map_err(ScenarioExecutionError::Lab)
}

fn wait_for_step_settled(
    handle: &CoreActorHandle,
    recorder: &ScenarioRecorder,
    driver: &mut LiveTransportDriver,
    revision_before: SnapshotRevision,
    sequence_before: u64,
    minimum_revision_delta: u64,
) -> Result<StepSettlement, ScenarioExecutionError> {
    let target_revision = revision_before.get().saturating_add(minimum_revision_delta);
    let mut remaining = STEP_SETTLE_TIMEOUT;
    loop {
        driver.pump().map_err(ScenarioExecutionError::Lab)?;
        if handle
            .current_snapshot()
            .is_ok_and(|snapshot| snapshot.revision.get() >= target_revision)
        {
            return Ok(StepSettlement::Settled);
        }
        if recorder.entries().iter().any(|entry| {
            entry.sequence >= sequence_before
                && matches!(entry.kind, RecordedNotificationKind::Error { .. })
        }) {
            return Ok(StepSettlement::Settled);
        }
        if remaining.is_zero() {
            return Ok(StepSettlement::TimedOut);
        }
        let chunk = remaining.min(STEP_SETTLE_POLL);
        remaining -= chunk;
        recorder.wait_for_progress(sequence_before, chunk);
    }
}

fn evaluate_assertions(
    lab: &LabRuntime,
    scenario: &Scenario,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    recorders: &HashMap<&str, Arc<ScenarioRecorder>>,
) -> Result<(ScenarioOutcome, Vec<AssertionResult>), ScenarioExecutionError> {
    let mut outcome = ScenarioOutcome::Completed;
    let mut results = Vec::with_capacity(scenario.assertions.len());
    for assertion in &scenario.assertions {
        let node = assertion_node(assertion);
        let lab_node_id = node_id_for(node.as_str(), node, lab_node_ids)?;
        let handle = node_handle(lab, lab_node_id, node)?;
        let recorder = recorders
            .get(node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))?;
        let snapshot = handle.current_snapshot().ok();
        let held = evaluate_assertion(assertion, snapshot.as_ref(), &recorder.entries());
        let assertion_outcome = if held {
            AssertionOutcome::Held
        } else {
            outcome = ScenarioOutcome::TimedOut;
            AssertionOutcome::TimedOut
        };
        results.push(AssertionResult {
            kind: assertion_kind(assertion).to_owned(),
            node: node.clone(),
            by_ms: assertion_deadline(assertion),
            outcome: assertion_outcome,
        });
        if assertion_outcome != AssertionOutcome::Held
            && scenario.termination.stop_on_assertion_failure
        {
            break;
        }
    }
    Ok((outcome, results))
}

fn node_id_for(
    key: &str,
    node: &NodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
) -> Result<LabNodeId, ScenarioExecutionError> {
    lab_node_ids
        .get(key)
        .copied()
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))
}

fn node_handle(
    lab: &LabRuntime,
    node_id: LabNodeId,
    node: &NodeId,
) -> Result<CoreActorHandle, ScenarioExecutionError> {
    lab.node_handle(node_id)
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))
}

fn collect_notifications(
    scenario: &Scenario,
    recorders: &HashMap<&str, Arc<ScenarioRecorder>>,
) -> Vec<(String, Vec<RecordedNotification>)> {
    scenario
        .nodes
        .iter()
        .filter_map(|node| {
            recorders
                .get(node.id.as_str())
                .map(|recorder| (node.id.to_string(), recorder.entries()))
        })
        .collect()
}

fn stop_scenario_nodes(lab: &LabRuntime, lab_node_ids: &HashMap<&str, LabNodeId>) -> bool {
    let mut clean = true;
    for lab_node_id in lab_node_ids.values() {
        if lab.stop_node(*lab_node_id).is_err() {
            clean = false;
        }
    }
    clean
}
