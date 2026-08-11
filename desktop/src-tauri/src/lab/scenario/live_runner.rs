use super::legacy;
use super::live_transport::{LiveScenarioObserver, LiveTransportDriver};
use super::{
    AssertionOutcome, AssertionResult, ClockAdvance, FixtureId, NodeId, Scenario, ScenarioAction,
    ScenarioAssertion, ScenarioExecutionError, ScenarioLifecycleTarget, ScenarioOutcome,
    ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
};
use crate::lab::recorder::{RecordedNotification, RecordedNotificationKind, ScenarioRecorder};
use crate::lab::{LabNodeId, LabRuntime};
use silent_disco_core::domain::{ApprovalMode, DeviceId, OperationId, RequestId, SessionId};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreCommand, CoreCommandRequest,
    CoreSnapshot, DeliveryReport, HostDraftPatch, InviteCodePatch, PermissionCapability,
    SnapshotRevision, SynchronizationSummary, TransportEvent,
};
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

    if !scenario.requires_live_runner() {
        return legacy::run_scenario_with_trace(lab, &scenario.as_legacy_validation_scenario());
    }

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

    let node_notifications: Vec<(String, Vec<RecordedNotification>)> = scenario
        .nodes
        .iter()
        .filter_map(|node| {
            recorders
                .get(node.id.as_str())
                .map(|recorder| (node.id.to_string(), recorder.entries()))
        })
        .collect();

    let mut teardown_ok = true;
    for lab_node_id in lab_node_ids.values() {
        if lab.stop_node(*lab_node_id).is_err() {
            teardown_ok = false;
        }
    }
    drop(driver);

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
        if step.at_ms > current_ms {
            let delta = step.at_ms - current_ms;
            lab.advance(delta).map_err(ScenarioExecutionError::Lab)?;
            driver.pump().map_err(ScenarioExecutionError::Lab)?;
            current_ms = step.at_ms;
            clock_advances.push(ClockAdvance {
                requested_delta_ms: delta,
                resulting_now_ms: current_ms,
            });
        }

        let lab_node_id = *lab_node_ids
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        let handle = lab
            .node_handle(lab_node_id)
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
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

    if scenario.timeout_ms > current_ms {
        let delta = scenario.timeout_ms - current_ms;
        lab.advance(delta).map_err(ScenarioExecutionError::Lab)?;
        driver.pump().map_err(ScenarioExecutionError::Lab)?;
        current_ms = scenario.timeout_ms;
        clock_advances.push(ClockAdvance {
            requested_delta_ms: delta,
            resulting_now_ms: current_ms,
        });
    }

    let mut assertion_results = Vec::with_capacity(scenario.assertions.len());
    let mut outcome = ScenarioOutcome::Completed;
    for assertion in &scenario.assertions {
        let node = assertion_node(assertion);
        let lab_node_id = *lab_node_ids
            .get(node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))?;
        let handle = lab
            .node_handle(lab_node_id)
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))?;
        let recorder = recorders
            .get(node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(node.clone()))?;

        let snapshot = handle.current_snapshot().ok();
        let entries = recorder.entries();
        let held = evaluate_assertion(assertion, snapshot.as_ref(), &entries);
        let assertion_outcome = if held {
            AssertionOutcome::Held
        } else {
            AssertionOutcome::TimedOut
        };
        if assertion_outcome != AssertionOutcome::Held {
            outcome = ScenarioOutcome::TimedOut;
        }
        assertion_results.push(AssertionResult {
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

    Ok(ScenarioReport {
        schema_version: scenario.schema_version,
        seed: scenario.seed,
        outcome,
        final_time_ms: current_ms,
        step_results,
        assertion_results,
    })
}

fn action_revision_delta(action: &ScenarioAction) -> u64 {
    match action {
        ScenarioAction::CreateHostSession
        | ScenarioAction::EndHostSession
        | ScenarioAction::StartDiscovery
        | ScenarioAction::StopDiscovery
        | ScenarioAction::SubmitJoin { .. }
        | ScenarioAction::CancelJoin
        | ScenarioAction::ApproveJoin { .. }
        | ScenarioAction::RejectJoin { .. }
        | ScenarioAction::RemoveListener { .. }
        | ScenarioAction::StartPlayback { .. }
        | ScenarioAction::StopPlayback
        | ScenarioAction::ExportDiagnostics => 2,
        _ => 1,
    }
}

fn wait_for_step_settled(
    handle: &silent_disco_core::runtime::CoreActorHandle,
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
        if let Ok(snapshot) = handle.current_snapshot()
            && snapshot.revision.get() >= target_revision
        {
            return Ok(StepSettlement::Settled);
        }
        for entry in recorder.entries() {
            if entry.sequence < sequence_before {
                continue;
            }
            if matches!(entry.kind, RecordedNotificationKind::Error { .. }) {
                return Ok(StepSettlement::Settled);
            }
        }
        if remaining.is_zero() {
            return Ok(StepSettlement::TimedOut);
        }
        let chunk = remaining.min(STEP_SETTLE_POLL);
        remaining -= chunk;
        recorder.wait_for_progress(sequence_before, chunk);
    }
}

fn current_revision(handle: &silent_disco_core::runtime::CoreActorHandle) -> SnapshotRevision {
    handle
        .current_snapshot()
        .map_or(SnapshotRevision::new(0), |snapshot| snapshot.revision)
}

fn submit_action(
    lab: &LabRuntime,
    scenario: &Scenario,
    handle: &silent_disco_core::runtime::CoreActorHandle,
    lab_node_id: LabNodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    action: &ScenarioAction,
) -> Result<Option<String>, ScenarioExecutionError> {
    match action {
        ScenarioAction::RemoveListener { listener_node } => {
            let listener_id = resolve_remove_listener(lab, listener_node, lab_node_ids)?;
            return submit_command(handle, CoreCommand::RemoveListener { listener_id });
        }
        ScenarioAction::InjectUnderrun { missing_frames } => {
            return Ok(submit_audio(
                handle,
                AudioEvent::Underrun {
                    missing_frames: *missing_frames,
                },
            ));
        }
        ScenarioAction::InjectSynchronizationUpdated {
            confidence,
            offset_ms,
            round_trip_ms,
            drift_ppm,
        } => {
            let identity = lab.node_identity(lab_node_id).ok_or_else(|| {
                ScenarioExecutionError::IdentifierInvalid("node has no identity".to_owned())
            })?;
            let summary =
                SynchronizationSummary::new(*confidence, *offset_ms, *round_trip_ms, *drift_ppm)
                    .map_err(ScenarioExecutionError::Descriptor)?;
            return Ok(submit_audio(
                handle,
                AudioEvent::SynchronizationUpdated {
                    device_id: identity.device_id().clone(),
                    summary,
                },
            ));
        }
        ScenarioAction::InjectDeliveryCompleted {
            operation_id,
            intended_peers,
            successful_peers,
            failed_peers,
        } => {
            let operation_id = match operation_id {
                Some(value) => OperationId::new(value.clone()).map_err(|error| {
                    ScenarioExecutionError::IdentifierInvalid(error.to_string())
                })?,
                None => OperationId::new(format!("lab-scenario-delivery-{lab_node_id:?}"))
                    .map_err(|error| {
                        ScenarioExecutionError::IdentifierInvalid(error.to_string())
                    })?,
            };
            let report = DeliveryReport::new(*intended_peers, *successful_peers, *failed_peers)
                .map_err(ScenarioExecutionError::Descriptor)?;
            return Ok(submit_transport(
                handle,
                TransportEvent::DeliveryCompleted {
                    operation_id,
                    report,
                },
            ));
        }
        _ => {}
    }

    submit_command(handle, build_command(scenario, action)?)
}

fn submit_command(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    command: CoreCommand,
) -> Result<Option<String>, ScenarioExecutionError> {
    let request = CoreCommandRequest::new(current_revision(handle), command)
        .map_err(ScenarioExecutionError::CommandShape)?;
    Ok(match handle.submit_command(request) {
        Ok(_receipt) => None,
        Err(error) => Some(format!(
            "{}: {}",
            error.code.stable_name(),
            error.message
        )),
    })
}

fn build_command(
    scenario: &Scenario,
    action: &ScenarioAction,
) -> Result<CoreCommand, ScenarioExecutionError> {
    Ok(match action {
        ScenarioAction::SelectRole { role } => CoreCommand::SelectRole { role: *role },
        ScenarioAction::ConfigureHost {
            session_name,
            fixture,
        } => CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some(session_name.clone()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Clear,
            audio_source: AudioSourcePatch::Set(source_descriptor(scenario, fixture)?),
            remember_approved_devices: Some(false),
        }),
        ScenarioAction::CreateHostSession => CoreCommand::CreateHostSession,
        ScenarioAction::EndHostSession => CoreCommand::EndHostSession,
        ScenarioAction::StartDiscovery => CoreCommand::StartDiscovery,
        ScenarioAction::StopDiscovery => CoreCommand::StopDiscovery,
        ScenarioAction::SelectSession { session_id } => CoreCommand::SelectSession {
            session_id: SessionId::new(session_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
        },
        ScenarioAction::SubmitJoin { invite_code } => CoreCommand::SubmitJoin {
            invite_code: invite_code.clone(),
        },
        ScenarioAction::CancelJoin => CoreCommand::CancelJoin,
        ScenarioAction::ApproveJoin {
            request_id,
            remember_for_future,
        } => CoreCommand::ApproveJoin {
            request_id: RequestId::new(request_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
            remember_for_future: *remember_for_future,
        },
        ScenarioAction::RejectJoin { request_id } => CoreCommand::RejectJoin {
            request_id: RequestId::new(request_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
        },
        ScenarioAction::StartPlayback { fixture } => CoreCommand::StartPlayback {
            source: source_descriptor(scenario, fixture)?,
        },
        ScenarioAction::PausePlayback => CoreCommand::PausePlayback,
        ScenarioAction::ResumePlayback => CoreCommand::ResumePlayback,
        ScenarioAction::StopPlayback => CoreCommand::StopPlayback,
        ScenarioAction::SetLocalVolume { linear_gain } => CoreCommand::SetLocalVolume {
            linear_gain: *linear_gain,
        },
        ScenarioAction::RequestResync => CoreCommand::RequestResync,
        ScenarioAction::RetryRecoverableFailure => CoreCommand::RetryRecoverableFailure,
        ScenarioAction::ExportDiagnostics => CoreCommand::ExportDiagnostics,
        ScenarioAction::Shutdown => CoreCommand::Shutdown,
        ScenarioAction::RemoveListener { .. }
        | ScenarioAction::InjectUnderrun { .. }
        | ScenarioAction::InjectSynchronizationUpdated { .. }
        | ScenarioAction::InjectDeliveryCompleted { .. } => {
            unreachable!("direct actions are handled before command construction")
        }
    })
}

fn source_descriptor(
    scenario: &Scenario,
    fixture_id: &FixtureId,
) -> Result<AudioSourceDescriptor, ScenarioExecutionError> {
    let fixture = scenario
        .fixtures
        .iter()
        .find(|candidate| candidate.id == *fixture_id)
        .ok_or_else(|| {
            ScenarioExecutionError::IdentifierInvalid(format!(
                "fixture '{}' was not declared",
                fixture_id.as_str()
            ))
        })?;
    AudioSourceDescriptor::new(
        fixture.id.as_str(),
        fixture.display_name.clone(),
        fixture.byte_length,
        fixture.duration_ms,
    )
    .map_err(ScenarioExecutionError::Descriptor)
}

fn submit_audio(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    event: AudioEvent,
) -> Option<String> {
    match handle.submit_audio_event(event) {
        Ok(()) => None,
        Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
    }
}

fn submit_transport(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    event: TransportEvent,
) -> Option<String> {
    match handle.submit_transport_event(event) {
        Ok(()) => None,
        Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
    }
}

fn resolve_remove_listener(
    lab: &LabRuntime,
    listener_node: &NodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
) -> Result<DeviceId, ScenarioExecutionError> {
    let lab_node_id = *lab_node_ids
        .get(listener_node.as_str())
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(listener_node.clone()))?;
    let identity = lab
        .node_identity(lab_node_id)
        .ok_or_else(|| ScenarioExecutionError::UnknownNode(listener_node.clone()))?;
    Ok(identity.device_id().clone())
}

fn assertion_deadline(assertion: &ScenarioAssertion) -> u64 {
    match assertion {
        ScenarioAssertion::LifecycleReached { by_ms, .. }
        | ScenarioAssertion::CapabilityAvailable { by_ms, .. }
        | ScenarioAssertion::ListenerCountAtLeast { by_ms, .. }
        | ScenarioAssertion::SyncConfidenceAtLeast { by_ms, .. }
        | ScenarioAssertion::SynchronizationWithinBounds { by_ms, .. }
        | ScenarioAssertion::ErrorCodeObserved { by_ms, .. }
        | ScenarioAssertion::DeliverySeverityIs { by_ms, .. }
        | ScenarioAssertion::UnderrunFramesAtMost { by_ms, .. }
        | ScenarioAssertion::CleanShutdown { by_ms, .. }
        | ScenarioAssertion::NoUnexpectedFatalError { by_ms, .. } => *by_ms,
    }
}

fn assertion_node(assertion: &ScenarioAssertion) -> &NodeId {
    match assertion {
        ScenarioAssertion::LifecycleReached { node, .. }
        | ScenarioAssertion::CapabilityAvailable { node, .. }
        | ScenarioAssertion::ListenerCountAtLeast { node, .. }
        | ScenarioAssertion::SyncConfidenceAtLeast { node, .. }
        | ScenarioAssertion::SynchronizationWithinBounds { node, .. }
        | ScenarioAssertion::ErrorCodeObserved { node, .. }
        | ScenarioAssertion::DeliverySeverityIs { node, .. }
        | ScenarioAssertion::UnderrunFramesAtMost { node, .. }
        | ScenarioAssertion::CleanShutdown { node, .. }
        | ScenarioAssertion::NoUnexpectedFatalError { node, .. } => node,
    }
}

fn assertion_kind(assertion: &ScenarioAssertion) -> &'static str {
    match assertion {
        ScenarioAssertion::LifecycleReached { .. } => "lifecycleReached",
        ScenarioAssertion::CapabilityAvailable { .. } => "capabilityAvailable",
        ScenarioAssertion::ListenerCountAtLeast { .. } => "listenerCountAtLeast",
        ScenarioAssertion::SyncConfidenceAtLeast { .. } => "syncConfidenceAtLeast",
        ScenarioAssertion::SynchronizationWithinBounds { .. } => "synchronizationWithinBounds",
        ScenarioAssertion::ErrorCodeObserved { .. } => "errorCodeObserved",
        ScenarioAssertion::DeliverySeverityIs { .. } => "deliverySeverityIs",
        ScenarioAssertion::UnderrunFramesAtMost { .. } => "underrunFramesAtMost",
        ScenarioAssertion::CleanShutdown { .. } => "cleanShutdown",
        ScenarioAssertion::NoUnexpectedFatalError { .. } => "noUnexpectedFatalError",
    }
}

fn evaluate_assertion(
    assertion: &ScenarioAssertion,
    snapshot: Option<&CoreSnapshot>,
    entries: &[RecordedNotification],
) -> bool {
    match assertion {
        ScenarioAssertion::LifecycleReached { target, .. } => {
            let Some(snapshot) = snapshot else { return false };
            match target {
                ScenarioLifecycleTarget::Role(role) => snapshot.selected_role == Some(role.0),
                ScenarioLifecycleTarget::Host(state) => snapshot.host_lifecycle == state.0,
                ScenarioLifecycleTarget::Listener(state) => snapshot.listener_lifecycle == state.0,
                ScenarioLifecycleTarget::Playback(state) => snapshot.playback_state == state.0,
            }
        }
        ScenarioAssertion::CapabilityAvailable {
            capability,
            available,
            ..
        } => {
            let Some(snapshot) = snapshot else { return false };
            let actual = match capability {
                PermissionCapability::NearbyDiscovery => {
                    snapshot.capabilities.nearby_discovery_available
                }
                PermissionCapability::NearbyAdvertising => {
                    snapshot.capabilities.nearby_advertising_available
                }
                PermissionCapability::LocalNetwork => snapshot.capabilities.local_network_available,
                PermissionCapability::AudioSourceSelection => {
                    snapshot.capabilities.audio_source_selection_available
                }
                PermissionCapability::AudioOutput => snapshot.capabilities.audio_output_available,
                PermissionCapability::SecureStore => snapshot.capabilities.secure_store_available,
            };
            actual == *available
        }
        ScenarioAssertion::ListenerCountAtLeast { count, .. } => {
            let Some(snapshot) = snapshot else { return false };
            u32::try_from(snapshot.listeners.len()).unwrap_or(u32::MAX) >= *count
        }
        ScenarioAssertion::SyncConfidenceAtLeast { confidence, .. } => {
            let Some(snapshot) = snapshot else { return false };
            snapshot.synchronization.is_some_and(|summary| {
                summary.confidence.stable_code() >= confidence.stable_code()
            })
        }
        ScenarioAssertion::SynchronizationWithinBounds {
            max_abs_offset_ms,
            max_round_trip_ms,
            ..
        } => {
            let Some(snapshot) = snapshot else { return false };
            let Some(summary) = snapshot.synchronization else { return false };
            let offset_ok =
                max_abs_offset_ms.is_none_or(|bound| summary.offset_ms.abs() <= bound);
            let rtt_ok = max_round_trip_ms.is_none_or(|bound| summary.round_trip_ms <= bound);
            offset_ok && rtt_ok
        }
        ScenarioAssertion::ErrorCodeObserved { code, .. } => entries.iter().any(|entry| {
            matches!(&entry.kind, RecordedNotificationKind::Error { code: observed, .. } if observed == code)
        }),
        ScenarioAssertion::DeliverySeverityIs { severity, .. } => {
            let Some(snapshot) = snapshot else { return false };
            snapshot
                .last_delivery
                .is_some_and(|report| report.severity.stable_code() == severity.stable_code())
        }
        ScenarioAssertion::UnderrunFramesAtMost {
            max_total_missing_frames,
            ..
        } => {
            let mut total: u64 = 0;
            for entry in entries {
                let RecordedNotificationKind::Diagnostic { name, fields } = &entry.kind else {
                    continue;
                };
                if name != "audio_underrun" {
                    continue;
                }
                let Some((_, value)) = fields.iter().find(|(key, _)| key == "missing_frames")
                else {
                    continue;
                };
                let Ok(missing) = value.parse::<u64>() else {
                    return false;
                };
                total = total.saturating_add(missing);
            }
            total <= u64::from(*max_total_missing_frames)
        }
        ScenarioAssertion::CleanShutdown { .. }
        | ScenarioAssertion::NoUnexpectedFatalError { .. } => {
            !entries.iter().any(|entry| entry.kind.is_fatal_error())
        }
    }
}
