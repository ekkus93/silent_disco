use super::{FixtureId, NodeId, Scenario, ScenarioAction, ScenarioExecutionError};
use crate::lab::{LabNodeId, LabRuntime};
use silent_disco_core::domain::{ApprovalMode, DeviceId, OperationId, RequestId, SessionId};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorHandle, CoreCommand,
    CoreCommandRequest, DeliveryReport, HostDraftPatch, InviteCodePatch, SnapshotRevision,
    SynchronizationSummary, TransportEvent,
};
use std::collections::HashMap;

pub(super) fn action_revision_delta(action: &ScenarioAction) -> u64 {
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

pub(super) fn current_revision(
    handle: &CoreActorHandle,
) -> Result<SnapshotRevision, ScenarioExecutionError> {
    handle
        .current_snapshot()
        .map(|snapshot| snapshot.revision)
        .map_err(|error| ScenarioExecutionError::Lab(error.into()))
}

pub(super) fn submit_action(
    lab: &LabRuntime,
    scenario: &Scenario,
    handle: &CoreActorHandle,
    lab_node_id: LabNodeId,
    lab_node_ids: &HashMap<&str, LabNodeId>,
    action: &ScenarioAction,
) -> Result<Option<String>, ScenarioExecutionError> {
    match action {
        ScenarioAction::RemoveListener { listener_node } => {
            let listener_id = resolve_remove_listener(lab, listener_node, lab_node_ids)?;
            submit_command(handle, CoreCommand::RemoveListener { listener_id })
        }
        ScenarioAction::InjectUnderrun { missing_frames } => Ok(submit_audio(
            handle,
            AudioEvent::Underrun {
                missing_frames: *missing_frames,
            },
        )),
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
            Ok(submit_audio(
                handle,
                AudioEvent::SynchronizationUpdated {
                    device_id: identity.device_id().clone(),
                    summary,
                },
            ))
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
            Ok(submit_transport(
                handle,
                TransportEvent::DeliveryCompleted {
                    operation_id,
                    report,
                },
            ))
        }
        _ => submit_command(handle, build_command(scenario, action)?),
    }
}

fn submit_command(
    handle: &CoreActorHandle,
    command: CoreCommand,
) -> Result<Option<String>, ScenarioExecutionError> {
    let request = CoreCommandRequest::new(current_revision(handle)?, command)
        .map_err(ScenarioExecutionError::CommandShape)?;
    Ok(match handle.submit_command(request) {
        Ok(_receipt) => None,
        Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
    })
}

fn build_command(
    scenario: &Scenario,
    action: &ScenarioAction,
) -> Result<CoreCommand, ScenarioExecutionError> {
    match action {
        ScenarioAction::SelectRole { role } => Ok(CoreCommand::SelectRole { role: *role }),
        ScenarioAction::ConfigureHost {
            session_name,
            fixture,
        } => Ok(CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some(session_name.clone()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Clear,
            audio_source: AudioSourcePatch::Set(source_descriptor(scenario, fixture)?),
            remember_approved_devices: Some(false),
        })),
        ScenarioAction::CreateHostSession => Ok(CoreCommand::CreateHostSession),
        ScenarioAction::EndHostSession => Ok(CoreCommand::EndHostSession),
        ScenarioAction::StartDiscovery => Ok(CoreCommand::StartDiscovery),
        ScenarioAction::StopDiscovery => Ok(CoreCommand::StopDiscovery),
        ScenarioAction::SelectSession { session_id } => Ok(CoreCommand::SelectSession {
            session_id: SessionId::new(session_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
        }),
        ScenarioAction::SubmitJoin { invite_code } => Ok(CoreCommand::SubmitJoin {
            invite_code: invite_code.clone(),
        }),
        ScenarioAction::CancelJoin => Ok(CoreCommand::CancelJoin),
        ScenarioAction::ApproveJoin {
            request_id,
            remember_for_future,
        } => Ok(CoreCommand::ApproveJoin {
            request_id: RequestId::new(request_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
            remember_for_future: *remember_for_future,
        }),
        ScenarioAction::RejectJoin { request_id } => Ok(CoreCommand::RejectJoin {
            request_id: RequestId::new(request_id.clone())
                .map_err(|error| ScenarioExecutionError::IdentifierInvalid(error.to_string()))?,
        }),
        ScenarioAction::StartPlayback { fixture } => Ok(CoreCommand::StartPlayback {
            source: source_descriptor(scenario, fixture)?,
        }),
        ScenarioAction::PausePlayback => Ok(CoreCommand::PausePlayback),
        ScenarioAction::ResumePlayback => Ok(CoreCommand::ResumePlayback),
        ScenarioAction::StopPlayback => Ok(CoreCommand::StopPlayback),
        ScenarioAction::SetLocalVolume { linear_gain } => Ok(CoreCommand::SetLocalVolume {
            linear_gain: *linear_gain,
        }),
        ScenarioAction::RequestResync => Ok(CoreCommand::RequestResync),
        ScenarioAction::RetryRecoverableFailure => Ok(CoreCommand::RetryRecoverableFailure),
        ScenarioAction::ExportDiagnostics => Ok(CoreCommand::ExportDiagnostics),
        ScenarioAction::Shutdown => Ok(CoreCommand::Shutdown),
        ScenarioAction::RemoveListener { .. }
        | ScenarioAction::InjectUnderrun { .. }
        | ScenarioAction::InjectSynchronizationUpdated { .. }
        | ScenarioAction::InjectDeliveryCompleted { .. } => {
            Err(ScenarioExecutionError::IdentifierInvalid(
                "internal Lab action routing reached command construction".to_owned(),
            ))
        }
    }
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

fn submit_audio(handle: &CoreActorHandle, event: AudioEvent) -> Option<String> {
    match handle.submit_audio_event(event) {
        Ok(()) => None,
        Err(error) => Some(format!("{}: {}", error.code.stable_name(), error.message)),
    }
}

fn submit_transport(handle: &CoreActorHandle, event: TransportEvent) -> Option<String> {
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
