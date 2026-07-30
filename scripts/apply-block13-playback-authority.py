#!/usr/bin/env python3
# Activate Rust-authoritative host playback commands and correlated effects for Desktop Block 13.

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    target = ROOT / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content.rstrip() + "\n", encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}")
    write(path, content.replace(old, new))


def create_new(path: str, content: str) -> None:
    target = ROOT / path
    if target.exists():
        raise SystemExit(f"{path}: expected new file")
    write(path, content)


def main() -> None:
    replace_once(
        "rust/silent-disco-core/src/runtime/effects.rs",
        "use crate::domain::{DeviceId, OperationId, RequestId, SessionId, TuningSettings};\n",
        "use super::types::AudioSourceDescriptor;\n"
        "use crate::domain::{DeviceId, OperationId, RequestId, SessionId, TuningSettings};\n",
    )
    replace_once(
        "rust/silent-disco-core/src/runtime/effects.rs",
        '''    DisconnectListener {
        session_id: SessionId,
        listener_id: DeviceId,
        reason_code: String,
    },
}
''',
        '''    DisconnectListener {
        session_id: SessionId,
        listener_id: DeviceId,
        reason_code: String,
    },
    StartHostPlayback {
        session_id: SessionId,
        source: AudioSourceDescriptor,
    },
    PauseHostPlayback {
        session_id: SessionId,
    },
    ResumeHostPlayback {
        session_id: SessionId,
    },
    StopHostPlayback {
        session_id: SessionId,
    },
}
''',
    )
    replace_once(
        "rust/silent-disco-core/src/runtime/effects.rs",
        "            Self::DeliverJoinApproval { .. } => Ok(()),\n",
        '''            Self::DeliverJoinApproval { .. }
            | Self::StartHostPlayback { .. }
            | Self::PauseHostPlayback { .. }
            | Self::ResumeHostPlayback { .. }
            | Self::StopHostPlayback { .. } => Ok(()),
''',
    )

    replace_once(
        "rust/silent-disco-core/src/runtime/records.rs",
        '''    DeliveryCompleted {
        operation_id: OperationId,
        report: DeliveryReport,
    },
    SessionEnded {
''',
        '''    DeliveryCompleted {
        operation_id: OperationId,
        report: DeliveryReport,
    },
    OperationFailed {
        operation_id: OperationId,
        error: CoreError,
    },
    SessionEnded {
''',
    )
    replace_once(
        "rust/silent-disco-core/src/runtime/records_v2.rs",
        '''    DeliveryCompleted { operation_id: OperationId, report: DeliveryReport },
    SessionEnded { session_id: SessionId },
''',
        '''    DeliveryCompleted { operation_id: OperationId, report: DeliveryReport },
    OperationFailed { operation_id: OperationId, error: CoreError },
    SessionEnded { session_id: SessionId },
''',
    )

    replace_once(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/mod.rs",
        "mod commands;\nmod platform;\n",
        "mod commands;\nmod host_playback;\nmod platform;\n",
    )
    replace_once(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/mod.rs",
        '''    DisconnectListener {
        listener_id: DeviceId,
    },
}
''',
        '''    DisconnectListener {
        listener_id: DeviceId,
    },
    StartHostPlayback,
    PauseHostPlayback,
    ResumeHostPlayback,
    StopHostPlayback,
}
''',
    )
    replace_once(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/mod.rs",
        '''                TransportEvent::DeliveryCompleted {
                    operation_id,
                    report,
                } => self.complete_transport_delivery(operation_id, report),
                TransportEvent::StateChanged(state) => self.apply_transport_state(state),
''',
        '''                TransportEvent::DeliveryCompleted {
                    operation_id,
                    report,
                } => {
                    if self.pending_transport_is_playback(&operation_id) {
                        self.complete_host_playback_delivery(operation_id, report)
                    } else {
                        self.complete_transport_delivery(operation_id, report)
                    }
                }
                TransportEvent::OperationFailed {
                    operation_id,
                    error,
                } => self.complete_transport_operation_failure(operation_id, error),
                TransportEvent::StateChanged(state) => self.apply_transport_state(state),
''',
    )

    replace_once(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/commands.rs",
        '''            CoreCommand::StartPlayback { .. }
            | CoreCommand::PausePlayback
            | CoreCommand::ResumePlayback
            | CoreCommand::StopPlayback
            | CoreCommand::SetLocalVolume { .. }
            | CoreCommand::RequestResync => Err(invalid_state(
                "playback requires the shared packetizer and scheduler blocks",
                Some(operation_id),
            )),
''',
        '''            CoreCommand::StartPlayback { source } => {
                self.start_host_playback(operation_id, source)
            }
            CoreCommand::PausePlayback => self.pause_host_playback(operation_id),
            CoreCommand::ResumePlayback => self.resume_host_playback(operation_id),
            CoreCommand::StopPlayback => self.stop_host_playback(operation_id),
            CoreCommand::SetLocalVolume { .. } | CoreCommand::RequestResync => Err(invalid_state(
                "listener playback controls require the shared packetizer and scheduler blocks",
                Some(operation_id),
            )),
''',
    )

    create_new(
        "rust/silent-disco-core/src/runtime/actor_runtime/state/host_playback.rs",
        r'''use super::{
    ActorState, AppRole, ApplyOutcome, CoreError, CoreNotification, HostLifecycle, OperationId,
    PendingTransportOperation, PlaybackState, RecoverableAction, TransportEffectRequest,
    invalid_state,
};
use crate::runtime::{AudioSourceDescriptor, DeliveryReport};

impl PendingTransportOperation {
    pub(super) const fn is_host_playback(&self) -> bool {
        matches!(
            self,
            Self::StartHostPlayback
                | Self::PauseHostPlayback
                | Self::ResumeHostPlayback
                | Self::StopHostPlayback
        )
    }
}

impl ActorState {
    pub(super) fn start_host_playback(
        &mut self,
        operation_id: OperationId,
        source: AudioSourceDescriptor,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        let session_id = self.active_host_session(&operation_id)?;
        if self.snapshot.host_lifecycle != HostLifecycle::Ready
            || !matches!(
                self.snapshot.playback_state,
                PlaybackState::Stopped | PlaybackState::Ready
            )
        {
            return Err(invalid_state(
                "host playback can start only from a ready stopped session",
                Some(operation_id),
            ));
        }
        if self.snapshot.host_draft.audio_source.as_ref() != Some(&source) {
            return Err(invalid_state(
                "host playback source does not match the authoritative host draft",
                Some(operation_id),
            ));
        }
        self.ensure_host_playback_operation_idle(&operation_id)?;
        let effect = self.start_transport_operation(
            TransportEffectRequest::StartHostPlayback { session_id, source },
            PendingTransportOperation::StartHostPlayback,
        )?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    pub(super) fn pause_host_playback(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        let session_id = self.active_host_session(&operation_id)?;
        if self.snapshot.host_lifecycle != HostLifecycle::Streaming
            || self.snapshot.playback_state != PlaybackState::Playing
        {
            return Err(invalid_state(
                "host playback can pause only while streaming",
                Some(operation_id),
            ));
        }
        self.ensure_host_playback_operation_idle(&operation_id)?;
        let effect = self.start_transport_operation(
            TransportEffectRequest::PauseHostPlayback { session_id },
            PendingTransportOperation::PauseHostPlayback,
        )?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    pub(super) fn resume_host_playback(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        let session_id = self.active_host_session(&operation_id)?;
        if self.snapshot.host_lifecycle != HostLifecycle::Paused
            || self.snapshot.playback_state != PlaybackState::Paused
        {
            return Err(invalid_state(
                "host playback can resume only from paused",
                Some(operation_id),
            ));
        }
        self.ensure_host_playback_operation_idle(&operation_id)?;
        let effect = self.start_transport_operation(
            TransportEffectRequest::ResumeHostPlayback { session_id },
            PendingTransportOperation::ResumeHostPlayback,
        )?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    pub(super) fn stop_host_playback(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        let session_id = self.active_host_session(&operation_id)?;
        if !matches!(
            self.snapshot.host_lifecycle,
            HostLifecycle::Streaming | HostLifecycle::Paused
        ) || !matches!(
            self.snapshot.playback_state,
            PlaybackState::Playing | PlaybackState::Paused
        ) {
            return Err(invalid_state(
                "host playback can stop only while streaming or paused",
                Some(operation_id),
            ));
        }
        self.ensure_host_playback_operation_idle(&operation_id)?;
        let effect = self.start_transport_operation(
            TransportEffectRequest::StopHostPlayback { session_id },
            PendingTransportOperation::StopHostPlayback,
        )?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    pub(super) fn pending_transport_is_playback(&self, operation_id: &OperationId) -> bool {
        self.pending_transport
            .iter()
            .find(|(candidate, _)| candidate == operation_id)
            .is_some_and(|(_, pending)| pending.is_host_playback())
    }

    pub(super) fn complete_host_playback_delivery(
        &mut self,
        operation_id: OperationId,
        report: DeliveryReport,
    ) -> Result<ApplyOutcome, CoreError> {
        let pending = self
            .remove_pending_transport(&operation_id)
            .ok_or_else(|| {
                invalid_state(
                    "stale or duplicate host playback completion",
                    Some(operation_id.clone()),
                )
            })?;
        if !pending.is_host_playback() {
            return Err(invalid_state(
                "host playback completion does not match the pending operation",
                Some(operation_id),
            ));
        }

        self.snapshot.last_delivery = Some(report);
        match pending {
            PendingTransportOperation::StartHostPlayback
            | PendingTransportOperation::ResumeHostPlayback => {
                self.snapshot.playback_state = PlaybackState::Playing;
                self.snapshot.host_lifecycle = HostLifecycle::Streaming;
            }
            PendingTransportOperation::PauseHostPlayback => {
                self.snapshot.playback_state = PlaybackState::Paused;
                self.snapshot.host_lifecycle = HostLifecycle::Paused;
            }
            PendingTransportOperation::StopHostPlayback => {
                self.snapshot.playback_state = PlaybackState::Stopped;
                self.snapshot.host_lifecycle = if self.snapshot.listeners.is_empty() {
                    HostLifecycle::WaitingForListeners
                } else {
                    HostLifecycle::Ready
                };
            }
            PendingTransportOperation::ApproveJoin(_)
            | PendingTransportOperation::RejectJoin { .. }
            | PendingTransportOperation::DisconnectListener { .. } => {
                unreachable!("non-playback operation was rejected above");
            }
        }
        self.clear_failure();

        let mut outcome = if report.intended_peers == 0 {
            self.diagnostic(
                "host_playback_no_listener_recipients",
                vec![Self::field("operation_id", operation_id.as_str())?],
            )?
        } else if report.failed_peers > 0 {
            self.diagnostic(
                "host_playback_partial_delivery",
                vec![
                    Self::field("intended_peers", &report.intended_peers.to_string())?,
                    Self::field("successful_peers", &report.successful_peers.to_string())?,
                    Self::field("failed_peers", &report.failed_peers.to_string())?,
                ],
            )?
        } else {
            ApplyOutcome::changed()
        };
        outcome.changed = true;
        Ok(outcome)
    }

    pub(super) fn complete_transport_operation_failure(
        &mut self,
        operation_id: OperationId,
        mut error: CoreError,
    ) -> Result<ApplyOutcome, CoreError> {
        let pending = self
            .remove_pending_transport(&operation_id)
            .ok_or_else(|| {
                invalid_state(
                    "stale or duplicate transport operation failure",
                    Some(operation_id.clone()),
                )
            })?;
        if let Some(inner_id) = &error.operation_id
            && inner_id != &operation_id
        {
            return Err(invalid_state(
                "transport failure operation ID does not match its wrapper",
                Some(operation_id),
            ));
        }
        error.operation_id = Some(operation_id);
        if pending.is_host_playback() {
            self.snapshot.playback_state = PlaybackState::Error;
            self.snapshot.host_lifecycle = HostLifecycle::Error;
            self.snapshot.recoverable_action = match pending {
                PendingTransportOperation::StartHostPlayback => {
                    Some(RecoverableAction::ReselectAudioSource)
                }
                PendingTransportOperation::PauseHostPlayback
                | PendingTransportOperation::ResumeHostPlayback
                | PendingTransportOperation::StopHostPlayback => {
                    error.retryable.then_some(RecoverableAction::Retry)
                }
                PendingTransportOperation::ApproveJoin(_)
                | PendingTransportOperation::RejectJoin { .. }
                | PendingTransportOperation::DisconnectListener { .. } => None,
            };
        }
        self.snapshot.last_error = Some(error.clone());
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Error(error)],
            changed: true,
            stop_requested: false,
        })
    }

    fn active_host_session(
        &self,
        operation_id: &OperationId,
    ) -> Result<crate::domain::SessionId, CoreError> {
        self.host_session_id.clone().ok_or_else(|| {
            invalid_state(
                "host playback requires an active host session",
                Some(operation_id.clone()),
            )
        })
    }

    fn ensure_host_playback_operation_idle(
        &self,
        operation_id: &OperationId,
    ) -> Result<(), CoreError> {
        if self
            .pending_transport
            .iter()
            .any(|(_, pending)| pending.is_host_playback())
        {
            Err(invalid_state(
                "a host playback operation is already pending",
                Some(operation_id.clone()),
            ))
        } else {
            Ok(())
        }
    }
}
''',
    )

    replace_once(
        "rust/silent-disco-ffi/src/host_control/types.rs",
        '''    DisconnectListener {
        operation_id: String,
        session_id: String,
        listener_id: String,
        reason_code: String,
    },
}
''',
        '''    DisconnectListener {
        operation_id: String,
        session_id: String,
        listener_id: String,
        reason_code: String,
    },
    StartHostPlayback {
        operation_id: String,
        session_id: String,
        source: FfiAudioSource,
    },
    PauseHostPlayback {
        operation_id: String,
        session_id: String,
    },
    ResumeHostPlayback {
        operation_id: String,
        session_id: String,
    },
    StopHostPlayback {
        operation_id: String,
        session_id: String,
    },
}
''',
    )

    replace_once(
        "rust/silent-disco-ffi/src/host_control/conversions.rs",
        '''            TransportEffectRequest::DisconnectListener {
                session_id,
                listener_id,
                reason_code,
            } => Self::DisconnectListener {
                operation_id,
                session_id: session_id.into_string(),
                listener_id: listener_id.into_string(),
                reason_code,
            },
        }
''',
        '''            TransportEffectRequest::DisconnectListener {
                session_id,
                listener_id,
                reason_code,
            } => Self::DisconnectListener {
                operation_id,
                session_id: session_id.into_string(),
                listener_id: listener_id.into_string(),
                reason_code,
            },
            TransportEffectRequest::StartHostPlayback { session_id, source } => {
                Self::StartHostPlayback {
                    operation_id,
                    session_id: session_id.into_string(),
                    source: source.into(),
                }
            }
            TransportEffectRequest::PauseHostPlayback { session_id } => Self::PauseHostPlayback {
                operation_id,
                session_id: session_id.into_string(),
            },
            TransportEffectRequest::ResumeHostPlayback { session_id } => {
                Self::ResumeHostPlayback {
                    operation_id,
                    session_id: session_id.into_string(),
                }
            }
            TransportEffectRequest::StopHostPlayback { session_id } => Self::StopHostPlayback {
                operation_id,
                session_id: session_id.into_string(),
            },
        }
''',
    )

    replace_once(
        "rust/silent-disco-ffi/src/host_control/handle.rs",
        '''    FfiAppRole, FfiBridgeError, FfiCommandReceipt, FfiCoreError, FfiCoreObserver, FfiCoreSnapshot,
    FfiDeliveryReport, FfiHostDraft, FfiJoinRequestInput, FfiListenerSummary,
''',
        '''    FfiAppRole, FfiAudioSource, FfiBridgeError, FfiCommandReceipt, FfiCoreError,
    FfiCoreObserver, FfiCoreSnapshot, FfiDeliveryReport, FfiHostDraft, FfiJoinRequestInput,
    FfiListenerSummary,
''',
    )
    replace_once(
        "rust/silent-disco-ffi/src/host_control/handle.rs",
        '''    pub fn end_host_session(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::EndHostSession)
    }

    pub fn retry_recoverable_failure(
''',
        '''    pub fn end_host_session(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::EndHostSession)
    }

    pub fn start_host_playback(
        &self,
        expected_revision: u64,
        source: FfiAudioSource,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::StartPlayback {
                source: source.try_into()?,
            },
        )
    }

    pub fn pause_host_playback(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::PausePlayback)
    }

    pub fn resume_host_playback(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::ResumePlayback)
    }

    pub fn stop_host_playback(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::StopPlayback)
    }

    pub fn retry_recoverable_failure(
''',
    )
    replace_once(
        "rust/silent-disco-ffi/src/host_control/handle.rs",
        '''    pub fn transport_failed(&self, message: String, retryable: bool) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::Failed(input_transport_error(
                message, retryable,
            )?))?;
        Ok(())
    }

    pub fn settings_saved(&self, operation_id: String) -> Result<(), FfiBridgeError> {
''',
        '''    pub fn transport_failed(&self, message: String, retryable: bool) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::Failed(input_transport_error(
                message, retryable,
            )?))?;
        Ok(())
    }

    pub fn transport_operation_failed(
        &self,
        operation_id: String,
        message: String,
        retryable: bool,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        let operation_id = operation_id_from_string(operation_id)?;
        self.handle
            .submit_transport_event(TransportEvent::OperationFailed {
                operation_id: operation_id.clone(),
                error: input_error(
                    CoreErrorCode::TransportDeliveryFailed,
                    message,
                    retryable,
                    Some(operation_id),
                )?,
            })?;
        Ok(())
    }

    pub fn settings_saved(&self, operation_id: String) -> Result<(), FfiBridgeError> {
''',
    )

    create_new(
        "rust/silent-disco-core/tests/host_block12_playback_authority.rs",
        r'''use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, PlaybackState, TransportState, TrustState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorHandle, CoreActorRuntime,
    CoreCommand, CoreCommandRequest, CoreNotification, CoreSnapshot, DeliveryReport, HostDraftPatch,
    InviteCodePatch, ListenerSummary, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion, SnapshotRevision, TransportEffect, TransportEffectRequest,
    TransportEvent,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn host_playback_commands_are_effect_driven_and_revisioned() {
    let (runtime, handle, receiver, source, ready) = start_ready_host();

    let start_revision = submit_command(
        &handle,
        ready.revision,
        CoreCommand::StartPlayback {
            source: source.clone(),
        },
    );
    let _pending_start = next_snapshot(&receiver, start_revision + 1);
    let start_effect = next_transport_effect(&receiver);
    match &start_effect.request {
        TransportEffectRequest::StartHostPlayback {
            source: effect_source,
            ..
        } => assert_eq!(effect_source, &source),
        other => panic!("unexpected start playback effect: {other:?}"),
    }
    complete_delivery(&handle, start_effect, delivery(1, 1, 0));
    let streaming = next_snapshot(&receiver, start_revision + 2);
    assert_eq!(streaming.host_lifecycle, HostLifecycle::Streaming);
    assert_eq!(streaming.playback_state, PlaybackState::Playing);

    let pause_revision = submit_command(&handle, streaming.revision, CoreCommand::PausePlayback);
    let _pending_pause = next_snapshot(&receiver, pause_revision + 1);
    let pause_effect = next_transport_effect(&receiver);
    assert!(matches!(
        pause_effect.request,
        TransportEffectRequest::PauseHostPlayback { .. }
    ));
    complete_delivery(&handle, pause_effect, delivery(0, 0, 0));
    let paused = next_snapshot(&receiver, pause_revision + 2);
    assert_eq!(paused.host_lifecycle, HostLifecycle::Paused);
    assert_eq!(paused.playback_state, PlaybackState::Paused);
    assert_eq!(
        next_diagnostic(&receiver).name,
        "host_playback_no_listener_recipients"
    );

    let resume_revision = submit_command(&handle, paused.revision, CoreCommand::ResumePlayback);
    let _pending_resume = next_snapshot(&receiver, resume_revision + 1);
    let resume_effect = next_transport_effect(&receiver);
    assert!(matches!(
        resume_effect.request,
        TransportEffectRequest::ResumeHostPlayback { .. }
    ));
    complete_delivery(&handle, resume_effect, delivery(1, 1, 0));
    let resumed = next_snapshot(&receiver, resume_revision + 2);
    assert_eq!(resumed.host_lifecycle, HostLifecycle::Streaming);
    assert_eq!(resumed.playback_state, PlaybackState::Playing);

    let stop_revision = submit_command(&handle, resumed.revision, CoreCommand::StopPlayback);
    let _pending_stop = next_snapshot(&receiver, stop_revision + 1);
    let stop_effect = next_transport_effect(&receiver);
    assert!(matches!(
        stop_effect.request,
        TransportEffectRequest::StopHostPlayback { .. }
    ));
    complete_delivery(&handle, stop_effect, delivery(2, 1, 1));
    let stopped = next_snapshot(&receiver, stop_revision + 2);
    assert_eq!(stopped.host_lifecycle, HostLifecycle::Ready);
    assert_eq!(stopped.playback_state, PlaybackState::Stopped);
    assert_eq!(
        next_diagnostic(&receiver).name,
        "host_playback_partial_delivery"
    );

    submit_command(&handle, stopped.revision, CoreCommand::PausePlayback);
    assert_eq!(
        next_error(&receiver).code,
        CoreErrorCode::InvalidStateTransition
    );

    runtime.shutdown().expect("shutdown playback actor");
}

#[test]
fn correlated_playback_failure_is_visible_and_recoverable() {
    let (runtime, handle, receiver, source, ready) = start_ready_host();
    let accepted = submit_command(
        &handle,
        ready.revision,
        CoreCommand::StartPlayback { source },
    );
    let _pending = next_snapshot(&receiver, accepted + 1);
    let operation_id = next_transport_effect(&receiver).operation_id;
    let error = CoreError::new(
        CoreErrorCode::TransportDeliveryFailed,
        "injected playback adapter failure",
        ErrorSeverity::Error,
        true,
        Some(operation_id.clone()),
    )
    .expect("valid transport error");
    handle
        .submit_transport_event(TransportEvent::OperationFailed {
            operation_id,
            error,
        })
        .expect("submit correlated failure");
    let failed = next_snapshot(&receiver, accepted + 2);
    assert_eq!(failed.host_lifecycle, HostLifecycle::Error);
    assert_eq!(failed.playback_state, PlaybackState::Error);
    assert_eq!(
        failed.last_error.as_ref().expect("visible error").code,
        CoreErrorCode::TransportDeliveryFailed
    );
    runtime.shutdown().expect("shutdown failed playback actor");
}

fn start_ready_host() -> (
    CoreActorRuntime,
    CoreActorHandle,
    Receiver<CoreNotification>,
    AudioSourceDescriptor,
    CoreSnapshot,
) {
    let (sender, receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("playback-host").expect("valid host ID")),
        move |notification| {
            sender.send(notification).expect("receiver remains open");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = runtime.handle();
    let initial = next_snapshot(&receiver, 0);
    let selected = submit_and_snapshot(
        &handle,
        &receiver,
        initial.revision,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    let source = AudioSourceDescriptor::new(
        "source-playback",
        "fixture.wav",
        Some(4_096),
        Some(2_000),
    )
    .expect("valid source");
    let drafted = submit_and_snapshot(
        &handle,
        &receiver,
        selected.revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Playback authority".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source.clone()),
            remember_approved_devices: Some(false),
        }),
    );
    let creating = submit_and_snapshot(
        &handle,
        &receiver,
        drafted.revision,
        CoreCommand::CreateHostSession,
    );
    let advertising = next_platform_effect(&receiver);
    assert!(matches!(
        advertising.request,
        PlatformEffectRequest::StartAdvertising(_)
    ));
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertising.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("complete advertising");
    let waiting = next_snapshot(&receiver, creating.revision.get() + 1);
    let listener = ListenerSummary::new(
        DeviceId::new("listener-playback").expect("valid listener ID"),
        "Playback listener",
        TrustState::SessionOnly,
        TransportState::Connected,
    )
    .expect("valid listener");
    handle
        .submit_transport_event(TransportEvent::ListenerConnected(listener))
        .expect("connect listener");
    let ready = next_snapshot(&receiver, waiting.revision.get() + 1);
    assert_eq!(ready.host_lifecycle, HostLifecycle::Ready);
    (runtime, handle, receiver, source, ready)
}

fn delivery(intended: u32, successful: u32, failed: u32) -> DeliveryReport {
    DeliveryReport::new(intended, successful, failed).expect("valid delivery report")
}

fn submit_and_snapshot(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    revision: SnapshotRevision,
    command: CoreCommand,
) -> CoreSnapshot {
    let accepted = submit_command(handle, revision, command);
    next_snapshot(receiver, accepted + 1)
}

fn submit_command(
    handle: &CoreActorHandle,
    revision: SnapshotRevision,
    command: CoreCommand,
) -> u64 {
    handle
        .submit_command(CoreCommandRequest::new(revision, command).expect("valid command"))
        .expect("queue command")
        .accepted_at_revision
        .get()
}

fn complete_delivery(handle: &CoreActorHandle, effect: TransportEffect, report: DeliveryReport) {
    handle
        .submit_transport_event(TransportEvent::DeliveryCompleted {
            operation_id: effect.operation_id,
            report,
        })
        .expect("complete transport effect");
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum_revision: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot))
                if snapshot.revision.get() >= minimum_revision =>
            {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("timed out waiting for revision {minimum_revision}");
            }
            Err(RecvTimeoutError::Disconnected) => panic!("notification channel disconnected"),
        }
    }
}

fn next_platform_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::PlatformEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for platform effect: {error}"),
        }
    }
}

fn next_transport_effect(receiver: &Receiver<CoreNotification>) -> TransportEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for transport effect: {error}"),
        }
    }
}

fn next_diagnostic(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::CoreDiagnostic {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Diagnostic(diagnostic)) => return diagnostic,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for diagnostic: {error}"),
        }
    }
}

fn next_error(receiver: &Receiver<CoreNotification>) -> CoreError {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Error(error)) => return error,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for error: {error}"),
        }
    }
}
''',
    )

    create_new(
        "rust/silent-disco-ffi/tests/host_playback.rs",
        r'''use silent_disco_ffi::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCoreHandle,
    FfiCoreNotification, FfiCoreObserver, FfiDeliveryReport, FfiHostDraft, FfiHostLifecycle,
    FfiListenerSummary, FfiPlatformCompletion, FfiPlatformEffect, FfiPlaybackState,
    FfiTransportEffect, FfiTransportState, FfiTrustState,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct RecordingObserver {
    notifications: Mutex<Vec<FfiCoreNotification>>,
    available: Condvar,
}

impl RecordingObserver {
    fn wait_for_snapshot(&self, revision: u64) -> silent_disco_ffi::FfiCoreSnapshot {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::Snapshot { snapshot } if snapshot.revision >= revision => {
                Some(snapshot.clone())
            }
            _ => None,
        })
    }

    fn wait_for_platform_effect(&self) -> FfiPlatformEffect {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::PlatformEffect { effect } => Some(effect.clone()),
            _ => None,
        })
    }

    fn wait_for_transport_effect(&self) -> FfiTransportEffect {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::TransportEffect { effect } => Some(effect.clone()),
            _ => None,
        })
    }

    fn wait_for<T>(&self, mut select: impl FnMut(&FfiCoreNotification) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        let mut notifications = self.notifications.lock().expect("notification lock");
        loop {
            if let Some(value) = notifications.iter().find_map(&mut select) {
                return value;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for notification");
            let (next, wait) = self
                .available
                .wait_timeout(notifications, remaining)
                .expect("notification wait");
            notifications = next;
            assert!(!wait.timed_out(), "timed out waiting for notification");
        }
    }
}

impl FfiCoreObserver for RecordingObserver {
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        self.notifications
            .lock()
            .expect("notification lock")
            .push(notification);
        self.available.notify_all();
        Ok(())
    }
}

#[test]
fn uniffi_host_playback_is_command_effect_completion_driven() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-playback-host".to_owned(), observer.clone())
        .expect("open UniFFI core");
    let initial = observer.wait_for_snapshot(0);
    handle
        .select_role(initial.revision, FfiAppRole::Host)
        .expect("select host");
    let selected = observer.wait_for_snapshot(1);
    let source = FfiAudioSource {
        source_id: "source-ffi-playback".to_owned(),
        display_name: "fixture.wav".to_owned(),
        size_bytes: Some(4_096),
        duration_ms: Some(2_000),
    };
    handle
        .update_host_draft(
            selected.revision,
            FfiHostDraft {
                session_name: "FFI playback".to_owned(),
                approval_mode: FfiApprovalMode::Manual,
                invite_code: None,
                audio_source: Some(source.clone()),
                remember_approved_devices: false,
                tuning: selected.host_draft.tuning,
            },
        )
        .expect("update draft");
    let drafted = observer.wait_for_snapshot(2);
    handle
        .create_host_session(drafted.revision)
        .expect("create host");
    let creating = observer.wait_for_snapshot(3);
    let advertising_operation = match observer.wait_for_platform_effect() {
        FfiPlatformEffect::StartAdvertising { operation_id, .. } => operation_id,
        other => panic!("unexpected platform effect: {other:?}"),
    };
    handle
        .platform_operation_succeeded(
            advertising_operation,
            FfiPlatformCompletion::AdvertisingStarted,
        )
        .expect("complete advertising");
    let waiting = observer.wait_for_snapshot(creating.revision + 1);
    handle
        .submit_listener_connected(FfiListenerSummary {
            device_id: "ffi-playback-listener".to_owned(),
            display_name: "FFI Listener".to_owned(),
            trust_state: FfiTrustState::SessionOnly,
            transport_state: FfiTransportState::Connected,
            synchronization: None,
            last_contact_ms: Some(100),
            last_error: None,
        })
        .expect("connect listener");
    let ready = observer.wait_for_snapshot(waiting.revision + 1);
    assert_eq!(ready.host_lifecycle, FfiHostLifecycle::Ready);

    handle
        .start_host_playback(ready.revision, source)
        .expect("start playback command");
    let pending = observer.wait_for_snapshot(ready.revision + 1);
    let operation_id = match observer.wait_for_transport_effect() {
        FfiTransportEffect::StartHostPlayback {
            operation_id,
            session_id,
            ..
        } => {
            assert!(!session_id.is_empty());
            operation_id
        }
        other => panic!("unexpected transport effect: {other:?}"),
    };
    handle
        .transport_delivery_completed(
            operation_id,
            FfiDeliveryReport {
                intended_peers: 1,
                successful_peers: 1,
                failed_peers: 0,
            },
        )
        .expect("complete start playback");
    let streaming = observer.wait_for_snapshot(pending.revision + 1);
    assert_eq!(streaming.host_lifecycle, FfiHostLifecycle::Streaming);
    assert_eq!(streaming.playback_state, FfiPlaybackState::Playing);

    handle
        .pause_host_playback(streaming.revision)
        .expect("pause command");
    let pause_pending = observer.wait_for_snapshot(streaming.revision + 1);
    let pause_operation = match observer.wait_for_transport_effect() {
        FfiTransportEffect::PauseHostPlayback { operation_id, .. } => operation_id,
        other => panic!("unexpected pause effect: {other:?}"),
    };
    handle
        .transport_delivery_completed(
            pause_operation,
            FfiDeliveryReport {
                intended_peers: 0,
                successful_peers: 0,
                failed_peers: 0,
            },
        )
        .expect("complete pause");
    let paused = observer.wait_for_snapshot(pause_pending.revision + 1);
    assert_eq!(paused.host_lifecycle, FfiHostLifecycle::Paused);
    assert_eq!(paused.playback_state, FfiPlaybackState::Paused);

    handle.shutdown().expect("shutdown UniFFI playback core");
}
''',
    )


if __name__ == "__main__":
    main()
