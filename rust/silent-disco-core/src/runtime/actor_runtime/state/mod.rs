#![allow(
    clippy::too_many_lines,
    reason = "the serialized reducer keeps each source-ordered event family auditable in one place"
)]

mod admission;
mod audio;
mod commands;
mod host_audio_control;
mod platform;
mod storage;
mod support;
mod transport;
mod tuning;

use super::errors::{invalid_argument, invalid_state, resource_limit, transport_delivery_failed};
use crate::domain::{
    AppRole, DeviceId, HostLifecycle, ListenerLifecycle, OperationId, PlaybackState, RequestId,
    SessionId, TransportState, TuningSettings,
};
use crate::error::CoreError;
use crate::runtime::records::{
    AudioEvent, CoreActorInput, CoreCommand, CoreDiagnostic, CoreNotification, CoreSnapshot,
    DiagnosticField, DiscoveryRequest, MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS,
    MAX_PENDING_JOIN_REQUESTS, NetworkEstablishmentRequest, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, RecoverableAction, SessionAdvertisement,
    StorageCompletion, StorageEvent, TransportEvent, current_protocol_version,
};
use crate::runtime::{
    ApprovalDelivery, StorageEffect, StorageEffectRequest, TransportEffect, TransportEffectRequest,
    TrustPersistenceRequest,
};

const MAX_PENDING_EFFECT_OPERATIONS: usize = 128;

#[derive(Debug, Clone)]
pub(super) struct ActorState {
    pub(super) snapshot: CoreSnapshot,
    local_device_id: DeviceId,
    host_session_id: Option<SessionId>,
    pending_platform: Vec<(OperationId, PendingPlatformOperation)>,
    pending_transport: Vec<(OperationId, PendingTransportOperation)>,
    pending_storage: Vec<(OperationId, PendingStorageOperation)>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingTransportOperation {
    ApproveJoin(ApprovalDelivery),
    RejectJoin {
        request_id: RequestId,
        listener_id: DeviceId,
    },
    DisconnectListener {
        listener_id: DeviceId,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum PendingStorageOperation {
    PersistApprovalTrust(TrustPersistenceRequest),
    PersistTuning(TuningSettings),
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

    fn transport_effect(effect: TransportEffect) -> Self {
        Self {
            notifications: vec![CoreNotification::TransportEffect(effect)],
            changed: true,
            stop_requested: false,
        }
    }

    fn storage_effect(effect: StorageEffect) -> Self {
        Self {
            notifications: vec![CoreNotification::StorageEffect(effect)],
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
            pending_transport: Vec::new(),
            pending_storage: Vec::new(),
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
                match request.command {
                    CoreCommand::ApproveJoin {
                        request_id,
                        remember_for_future,
                    } => self.approve_join(operation_id, request_id, remember_for_future),
                    CoreCommand::RejectJoin { request_id } => {
                        self.reject_join(operation_id, request_id)
                    }
                    CoreCommand::RemoveListener { listener_id } => {
                        self.remove_listener(operation_id, listener_id)
                    }
                    command => self.apply_command(operation_id, command),
                }
            }
            CoreActorInput::Platform(event) => self.apply_platform(event),
            CoreActorInput::Transport(event) => match event {
                TransportEvent::JoinRequested(request) => self.handle_join_request(request),
                TransportEvent::DeliveryCompleted {
                    operation_id,
                    report,
                } => self.complete_transport_delivery(operation_id, report),
                TransportEvent::StateChanged(state) => self.apply_transport_state(state),
                event => self.apply_transport(event),
            },
            CoreActorInput::Audio(event) => self.apply_audio_control(event),
            CoreActorInput::Storage(event) => {
                let tuning_completion = matches!(
                    self.pending_storage(event.operation_id()),
                    Some(PendingStorageOperation::PersistTuning(_))
                );
                let admission_completion = matches!(
                    self.pending_storage(event.operation_id()),
                    Some(PendingStorageOperation::PersistApprovalTrust(_))
                ) || matches!(
                    &event,
                    StorageEvent::OperationSucceeded {
                        completion: StorageCompletion::TrustedDeviceUpdated { .. },
                        ..
                    }
                );
                if tuning_completion {
                    self.apply_tuning_storage(event)
                } else if admission_completion {
                    self.apply_admission_storage(event)
                } else {
                    self.apply_storage(event)
                }
            }
        }
    }
}
