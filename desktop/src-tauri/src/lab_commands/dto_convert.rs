//! Domain state -> DTO conversion for the Lab Mode command surface (Block
//! 42), including the UI-facing timeline-entry and summary-text bounds
//! ([`super::MAX_TIMELINE_ENTRIES_PER_NODE`], [`super::MAX_SUMMARY_CHARS`])
//! that are deliberately tighter than the recorder's own 4096-entry cap.

use super::{LabSessionState, MAX_SUMMARY_CHARS, MAX_TIMELINE_ENTRIES_PER_NODE};
use crate::lab::recorder::RecordedNotificationKind;
use crate::lab::scenario::{
    AssertionOutcome, Scenario, ScenarioOutcome, ScenarioReport, ScenarioTrace, StepSettlement,
};
use crate::lab::{LabNodeId, LabRuntime};
use crate::lab_dto::{
    LabAssertionResultDto, LabLinkDto, LabNodeDto, LabRunOutcomeDto, LabScenarioSummaryDto,
    LabStateDto, LabStepResultDto, LabTimelineEntryDto,
};

pub(super) fn node_dto(runtime: &LabRuntime, node_id: LabNodeId) -> Option<LabNodeDto> {
    let clock = runtime.node_clock(node_id)?;
    Some(LabNodeDto {
        node_id: node_id.as_u32().to_string(),
        offset_ms: clock.offset_ms().to_string(),
        drift_ppm: clock.drift_ppm().to_string(),
    })
}

pub(super) fn scenario_summary_dto(scenario: &Scenario) -> LabScenarioSummaryDto {
    LabScenarioSummaryDto {
        schema_version: scenario.schema_version,
        seed: scenario.seed.to_string(),
        node_ids: scenario
            .nodes
            .iter()
            .map(|node| node.id.as_str().to_owned())
            .collect(),
        link_count: u32::try_from(scenario.links.len()).unwrap_or(u32::MAX),
        fixture_count: u32::try_from(scenario.fixtures.len()).unwrap_or(u32::MAX),
        step_count: u32::try_from(scenario.steps.len()).unwrap_or(u32::MAX),
        assertion_count: u32::try_from(scenario.assertions.len()).unwrap_or(u32::MAX),
        timeout_ms: scenario.timeout_ms.to_string(),
        links: scenario
            .links
            .iter()
            .map(|link| LabLinkDto {
                from: link.from.as_str().to_owned(),
                to: link.to.as_str().to_owned(),
                latency_ms: link.latency_ms.to_string(),
                jitter_ms: link.jitter_ms.to_string(),
                loss_permille: link.loss_permille,
            })
            .collect(),
    }
}

pub(super) fn bounded_summary_text(value: &str) -> String {
    if value.chars().count() <= MAX_SUMMARY_CHARS {
        return value.to_owned();
    }
    let mut truncated: String = value.chars().take(MAX_SUMMARY_CHARS).collect();
    truncated.push('…');
    truncated
}

fn timeline_entry(
    node: &str,
    sequence: u64,
    kind: &RecordedNotificationKind,
) -> LabTimelineEntryDto {
    let (kind_name, summary) = match kind {
        RecordedNotificationKind::Snapshot { revision, summary } => (
            "snapshot",
            format!(
                "revision {revision}: host={} listener={} playback={}",
                summary.host_lifecycle, summary.listener_lifecycle, summary.playback_state
            ),
        ),
        RecordedNotificationKind::Effect { name } => ("effect", name.clone()),
        RecordedNotificationKind::TransportEffect { name } => ("transportEffect", name.clone()),
        RecordedNotificationKind::StorageEffect { name } => ("storageEffect", name.clone()),
        RecordedNotificationKind::Error {
            code,
            severity,
            message,
            ..
        } => ("error", format!("{code} ({severity}): {message}")),
        RecordedNotificationKind::Diagnostic { name, fields } => (
            "diagnostic",
            format!(
                "{name}: {}",
                fields
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
    };
    LabTimelineEntryDto {
        node: node.to_owned(),
        sequence: sequence.to_string(),
        kind: kind_name.to_owned(),
        summary: bounded_summary_text(&summary),
    }
}

pub(super) fn run_outcome_dto(report: &ScenarioReport, trace: &ScenarioTrace) -> LabRunOutcomeDto {
    let outcome = match report.outcome {
        ScenarioOutcome::Completed => "completed",
        ScenarioOutcome::TimedOut => "timedOut",
        ScenarioOutcome::ExecutionError => "executionError",
    };
    let mut timeline = Vec::new();
    let mut truncated = false;
    for (node, entries) in &trace.node_notifications {
        for entry in entries.iter().take(MAX_TIMELINE_ENTRIES_PER_NODE) {
            timeline.push(timeline_entry(node, entry.sequence, &entry.kind));
        }
        if entries.len() > MAX_TIMELINE_ENTRIES_PER_NODE {
            truncated = true;
        }
    }
    LabRunOutcomeDto {
        outcome: outcome.to_owned(),
        final_time_ms: report.final_time_ms.to_string(),
        step_results: report
            .step_results
            .iter()
            .map(|step| LabStepResultDto {
                index: u32::try_from(step.index).unwrap_or(u32::MAX),
                at_ms: step.at_ms.to_string(),
                node: step.node.as_str().to_owned(),
                submit_error: step.submit_error.clone(),
                settlement: match step.settlement {
                    StepSettlement::Settled => "settled".to_owned(),
                    StepSettlement::TimedOut => "timedOut".to_owned(),
                },
            })
            .collect(),
        assertion_results: report
            .assertion_results
            .iter()
            .map(|assertion| LabAssertionResultDto {
                kind: assertion.kind.clone(),
                node: assertion.node.as_str().to_owned(),
                by_ms: assertion.by_ms.to_string(),
                outcome: match assertion.outcome {
                    AssertionOutcome::Held => "held".to_owned(),
                    AssertionOutcome::TimedOut => "timedOut".to_owned(),
                },
            })
            .collect(),
        timeline,
        timeline_truncated: truncated,
    }
}

pub(super) fn state_dto(runtime: &LabRuntime, session: &LabSessionState) -> LabStateDto {
    let nodes = runtime
        .node_ids()
        .into_iter()
        .filter_map(|id| node_dto(runtime, id))
        .collect();
    LabStateDto {
        now_ms: runtime.now().get().to_string(),
        running: session.running,
        nodes,
        loaded_scenario: session
            .loaded
            .as_ref()
            .map(|loaded| scenario_summary_dto(&loaded.scenario)),
        last_run: session
            .last_run
            .as_ref()
            .map(|last_run| run_outcome_dto(&last_run.report, &last_run.trace)),
    }
}
