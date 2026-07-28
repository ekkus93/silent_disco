#![allow(
    clippy::too_many_lines,
    reason = "the serialized reducer keeps each source-ordered event family auditable in one place"
)]

mod audio;
mod commands;
mod platform;
mod storage;
mod support;
mod transport;

use super::errors::{invalid_argument, invalid_state, resource_limit};
use crate::domain::{
    AppRole, DeviceId, HostLifecycle, ListenerLifecycle, OperationId, PlaybackState, SessionId,
    TransportState,
};
use crate::error::CoreError;
use crate::runtime::records::{
    AudioEvent, CoreActorInput, CoreCommand, CoreDiagnostic, CoreNotification, CoreSnapshot,
    DiagnosticField, DiscoveryRequest, MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS,
    MAX_PENDING_JOIN_REQUESTS, NetworkEstablishmentRequest, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, RecoverableAction, SessionAdvertisement,
    StorageCompletion, StorageEvent, TransportEvent, current_protocol_version,
};

const MAX_PENDING_PLATFORM_OPERATIONS: usize = 128;

#[derive(Debug, Clone)]
pub(super) struct ActorState {
    pub(super) snapshot: CoreSnapshot,
    local_device_id: DeviceId,
    host_session_id: Option<SessionId>,
    pending_platform: Vec<(OperationId, PendingPlatformOperation)>,
    next_effect_sequence: u64,
    next_session_sequence: u64,
    next_export_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingPlatformOperation {
    StartAdvertising { session_id: SessionId },
    StopAdvertising,
    StartDiscovery,
    StopDiscovery,
    EstablishNetwork { session_id: SessionId },
    ReleaseNetwork,
    ShareDiagnostics { export_id: String },
}

#[derive(Debug, Default)]
struct ApplyOutcome {
    notifications: Vec<CoreNotification>,
    changed: bool,
    stop_requested: bool,
}

impl ApplyOutcome {
    fn changed() -> Self {
        Self {
            changed: true,
            ..Self::default()
        }
    }

    fn effect(effect: PlatformEffect) -> Self {
        Self {
            notifications: vec![CoreNotification::Effect(effect)],
            changed: true,
            stop_requested: false,
        }
    }
}

pub(super) struct ProcessResult {
    pub(super) notifications: Vec<CoreNotification>,
    pub(super) stop_requested: bool,
}

impl ActorState {
    pub(super) fn new(local_device_id: DeviceId) -> Self {
        Self {
            snapshot: CoreSnapshot::default(),
            local_device_id,
            host_session_id: None,
            pending_platform: Vec::new(),
            next_effect_sequence: 1,
            next_session_sequence: 1,
            next_export_sequence: 1,
        }
    }

    pub(super) fn process(&mut self, input: CoreActorInput) -> ProcessResult {
        let operation_id = input.operation_id().cloned();
        let mut candidate = self.clone();
        match candidate.apply(input) {
            Ok(mut outcome) => {
                if outcome.changed {
                    let Ok(revision) = candidate.snapshot.revision.checked_next() else {
                        return ProcessResult {
                            notifications: vec![CoreNotification::Error(resource_limit(
                                "snapshot revision overflow",
                                operation_id,
                            ))],
                            stop_requested: true,
                        };
                    };
                    candidate.snapshot.revision = revision;
                    if let Err(error) = candidate.snapshot.validate() {
                        return ProcessResult {
                            notifications: vec![CoreNotification::Error(invalid_argument(
                                error.to_string(),
                                operation_id,
                            ))],
                            stop_requested: false,
                        };
                    }
                    outcome
                        .notifications
                        .insert(0, CoreNotification::Snapshot(candidate.snapshot.clone()));
                }
                *self = candidate;
                ProcessResult {
                    notifications: outcome.notifications,
                    stop_requested: outcome.stop_requested,
                }
            }
            Err(error) => ProcessResult {
                notifications: vec![CoreNotification::Error(error)],
                stop_requested: false,
            },
        }
    }

    fn apply(&mut self, input: CoreActorInput) -> Result<ApplyOutcome, CoreError> {
        match input {
            CoreActorInput::Command {
                operation_id,
                request,
            } => {
                if request.expected_revision != self.snapshot.revision {
                    return Err(invalid_state(
                        format!(
                            "command expected snapshot revision {}, but current revision is {}",
                            request.expected_revision, self.snapshot.revision
                        ),
                        Some(operation_id),
                    ));
                }
                self.apply_command(operation_id, request.command)
            }
            CoreActorInput::Platform(event) => self.apply_platform(event),
            CoreActorInput::Transport(event) => self.apply_transport(event),
            CoreActorInput::Audio(event) => self.apply_audio(event),
            CoreActorInput::Storage(event) => self.apply_storage(event),
        }
    }
}
