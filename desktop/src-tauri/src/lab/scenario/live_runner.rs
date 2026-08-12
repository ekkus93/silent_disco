use super::assertions::{assertion_deadline, assertion_kind, assertion_node, evaluate_assertion};
use super::commands::{action_revision_delta, current_revision, submit_action};
use super::live_transport::{LiveScenarioObserver, LiveTransportDriver};
use super::{
    AssertionOutcome, AssertionResult, ClockAdvance, NodeId, Scenario, ScenarioExecutionError,
    ScenarioOutcome, ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
    scenario_node_parts,
};
use crate::dto::DesktopErrorDto;
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
        let clock =
            scenario
                .clocks
                .get(node.id.as_str())
                .copied()
                .unwrap_or(super::ScenarioClock {
                    offset_ms: 0,
                    drift_ppm: 0,
                });
        let recorder = ScenarioRecorder::new();
        let (observer, effect_receiver) = match LiveScenarioObserver::new(Arc::clone(&recorder)) {
            Ok(observer) => observer,
            Err(primary) => return Err(setup_failure(lab, &lab_node_ids, primary)),
        };
        let lab_node_id = match lab.start_node_with_clock_and_observer(
            clock.offset_ms,
            clock.drift_ppm,
            observer,
        ) {
            Ok(node_id) => node_id,
            Err(primary) => return Err(setup_failure(lab, &lab_node_ids, primary)),
        };
        lab_node_ids.insert(node.id.as_str(), lab_node_id);
        recorders.insert(node.id.as_str(), recorder);
        effect_receivers.insert(node.id.clone(), effect_receiver);
    }

    let mut driver = match LiveTransportDriver::new(lab, scenario, &lab_node_ids, effect_receivers)
    {
        Ok(driver) => driver,
        Err(primary) => return Err(setup_failure(lab, &lab_node_ids, primary)),
    };
    let mut clock_advances = Vec::new();
    let run_result = execute_steps_and_assertions(
        lab,
        scenario,
        &lab_node_ids,
        &recorders,
        &mut driver,
        &mut clock_advances,
    );

    let mut transport_cleanup = driver.shutdown().err();
    let transport_trace = match driver.transport_trace() {
        Ok(trace) => Some(trace),
        Err(error) => {
            transport_cleanup = merge_cleanup(
                transport_cleanup,
                Some(DesktopErrorDto::new(
                    "desktop.lab.transport_trace_failed",
                    "transport",
                    "error",
                    false,
                    &format!("Lab transport trace could not be finalized: {error}"),
                )),
            );
            None
        }
    };
    let node_notifications = collect_notifications(scenario, &recorders);
    drop(driver);
    let node_cleanup = stop_scenario_nodes(lab, &lab_node_ids).err();
    let cleanup = merge_cleanup(transport_cleanup, node_cleanup);
    let trace = transport_trace.map(|transport_trace| ScenarioTrace {
        clock_advances,
        node_notifications,
        transport_trace,
    });

    finish_run(run_result, cleanup, trace)
}

fn finish_run(
    run_result: Result<ScenarioReport, ScenarioExecutionError>,
    cleanup: Option<DesktopErrorDto>,
    trace: Option<ScenarioTrace>,
) -> Result<(ScenarioReport, ScenarioTrace), ScenarioExecutionError> {
    match (run_result, cleanup, trace) {
        (Ok(report), None, Some(trace)) => Ok((report, trace)),
        (Ok(_report), Some(cleanup), _) => Err(ScenarioExecutionError::Lab(cleanup)),
        (Err(ScenarioExecutionError::Lab(primary)), Some(cleanup), _) => Err(
            ScenarioExecutionError::Lab(primary.with_appended_cleanup(Some(cleanup))),
        ),
        (Err(primary), Some(cleanup), _) => Err(ScenarioExecutionError::Teardown {
            primary: Box::new(primary),
            cleanup,
        }),
        (Err(primary), None, _) => Err(primary),
        (Ok(_report), None, None) => Err(ScenarioExecutionError::Lab(DesktopErrorDto::new(
            "desktop.lab.transport_trace_missing",
            "transport",
            "fatal",
            false,
            "Lab scenario completed without a transport trace or a reported trace failure",
        ))),
    }
}

fn setup_failure(
    lab: &LabRuntime,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    primary: DesktopErrorDto,
) -> ScenarioExecutionError {
    let cleanup = stop_scenario_nodes(lab, lab_node_ids).err();
    ScenarioExecutionError::Lab(primary.with_appended_cleanup(cleanup))
}

fn merge_cleanup(
    primary: Option<DesktopErrorDto>,
    next: Option<DesktopErrorDto>,
) -> Option<DesktopErrorDto> {
    match (primary, next) {
        (Some(primary), Some(next)) => Some(primary.with_appended_cleanup(Some(next))),
        (Some(primary), None) => Some(primary),
        (None, Some(next)) => Some(next),
        (None, None) => None,
    }
}

include!("live_runner/execution.rs");
