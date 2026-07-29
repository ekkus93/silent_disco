use super::{
    ActorState, ApplyOutcome, CoreError, CoreNotification, MAX_CONNECTED_LISTENERS,
    MAX_PENDING_JOIN_REQUESTS, PendingStorageOperation, PendingTransportOperation,
    invalid_state, transport_delivery_failed,
};
use crate::domain::{
    AppRole, DeviceId, HostLifecycle, OperationId, PlaybackState, RequestId, TransportState,
    TrustState,
};
use crate::error::CoreErrorCode;
use crate::runtime::{
    ApprovalDelivery, ApprovalPreparation, AudioEvent, DeliveryCommitDisposition, DeliveryReport,
    JoinRejectionReason, JoinRequestDisposition, JoinRequestSummary, ListenerSummary,
    RecoverableAction, StorageCompletion, StorageEffectRequest, StorageEvent,
    TransportEffectRequest, TransportEvent, TrustPersistenceOutcome, approval_after_persistence,
    classify_delivery, classify_join_request, prepare_approval,
};

impl ActorState {
    pub(super) fn handle_join_request(
        &mut self,
        request: JoinRequestSummary,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_host_admission_event()?;
        if let Some(existing) = self
            .snapshot
            .pending_join_requests
            .iter()
            .find(|existing| existing.request_id == request.request_id)
        {
            if existing == &request {
                return Ok(ApplyOutcome::default());
            }
            return Err(invalid_state(
                "join request identifier was reused with different content",
                None,
            ));
        }
        if self
            .snapshot
            .pending_join_requests
            .iter()
            .any(|existing| existing.device_id == request.device_id)
        {
            return Err(invalid_state(
                "listener already has a pending join request",
                None,
            ));
        }
        if self.snapshot.pending_join_requests.len() >= MAX_PENDING_JOIN_REQUESTS {
            return Err(super::resource_limit(
                "pending join-request capacity reached",
                None,
            ));
        }

        let disposition = classify_join_request(&self.snapshot.host_draft, &request);
        self.snapshot.pending_join_requests.push(request.clone());
        match disposition {
            JoinRequestDisposition::PendingManualDecision => Ok(ApplyOutcome::changed()),
            JoinRequestDisposition::AutoApprove { trusted_for_future } => {
                self.start_approval_delivery(ApprovalDelivery {
                    request_id: request.request_id,
                    device_id: request.device_id,
                    trusted_for_future,
                    persistence_failed: false,
                })
            }
            JoinRequestDisposition::AutoReject { reason } => {
                self.start_rejection_delivery(&request, reason)
            }
        }
    }

    pub(super) fn approve_join(
        &mut self,
        operation_id: OperationId,
        request_id: RequestId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_host_admission_command(&operation_id)?;
        let request = self.pending_join_request(&request_id, &operation_id)?.clone();
        self.ensure_request_operation_idle(&request_id, &operation_id)?;
        match prepare_approval(&self.snapshot.host_draft, &request) {
            ApprovalPreparation::PersistTrustFirst(persistence) => {
                let effect = self.start_storage_operation(
                    StorageEffectRequest::PersistTrustedDevice {
                        device_id: persistence.device_id.clone(),
                        display_name: persistence.display_name.clone(),
                    },
                    PendingStorageOperation::PersistApprovalTrust(persistence),
                )?;
                Ok(ApplyOutcome::storage_effect(effect))
            }
            ApprovalPreparation::Deliver(delivery) => self.start_approval_delivery(delivery),
        }
    }

    pub(super) fn reject_join(
        &mut self,
        operation_id: OperationId,
        request_id: RequestId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_host_admission_command(&operation_id)?;
        let request = self.pending_join_request(&request_id, &operation_id)?.clone();
        self.ensure_request_operation_idle(&request_id, &operation_id)?;
        self.start_rejection_delivery(&request, JoinRejectionReason::HostRejected)
    }

    pub(super) fn remove_listener(
        &mut self,
        operation_id: OperationId,
        listener_id: DeviceId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_host_admission_command(&operation_id)?;
        if !self
            .snapshot
            .listeners
            .iter()
            .any(|listener| listener.device_id == listener_id)
        {
            return Err(invalid_state(
                "listener is stale or no longer connected",
                Some(operation_id),
            ));
        }
        if self.pending_transport.iter().any(|(_, pending)| {
            matches!(
                pending,
                PendingTransportOperation::DisconnectListener {
                    listener_id: pending_id,
                } if pending_id == &listener_id
            )
        }) {
            return Err(invalid_state(
                "listener removal is already pending",
                Some(operation_id),
            ));
        }
        let session_id = self.host_session_id.clone().ok_or_else(|| {
            invalid_state("host session is not active", Some(operation_id.clone()))
        })?;
        let effect = self.start_transport_operation(
            TransportEffectRequest::DisconnectListener {
                session_id,
                listener_id: listener_id.clone(),
                reason_code: "host_removed_listener".to_owned(),
            },
            PendingTransportOperation::DisconnectListener { listener_id },
        )?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    pub(super) fn apply_admission_storage(
        &mut self,
        event: StorageEvent,
    ) -> Result<ApplyOutcome, CoreError> {
        event.validate().map_err(|error| {
            super::invalid_argument(error.to_string(), Some(event.operation_id().clone()))
        })?;
        let wrapper_id = event.operation_id().clone();
        let pending = self
            .remove_pending_storage(&wrapper_id)
            .ok_or_else(|| {
                invalid_state(
                    "stale or duplicate admission storage completion",
                    Some(wrapper_id.clone()),
                )
            })?;
        match (pending, event) {
            (
                PendingStorageOperation::PersistApprovalTrust(persistence),
                StorageEvent::OperationSucceeded {
                    completion: StorageCompletion::TrustedDeviceUpdated { device_id },
                    ..
                },
            ) if device_id == persistence.device_id => self.start_approval_delivery(
                approval_after_persistence(&persistence, TrustPersistenceOutcome::Committed),
            ),
            (
                PendingStorageOperation::PersistApprovalTrust(persistence),
                StorageEvent::OperationFailed {
                    operation_id,
                    mut error,
                },
            ) => {
                if let Some(inner_id) = &error.operation_id
                    && inner_id != &operation_id
                {
                    return Err(invalid_state(
                        "storage failure operation ID does not match its wrapper",
                        Some(operation_id),
                    ));
                }
                error.operation_id = Some(operation_id);
                let delivery = approval_after_persistence(
                    &persistence,
                    TrustPersistenceOutcome::Failed,
                );
                let effect = self.create_approval_transport_effect(delivery)?;
                self.snapshot.last_error = Some(error.clone());
                Ok(ApplyOutcome {
                    notifications: vec![
                        CoreNotification::Error(error),
                        CoreNotification::TransportEffect(effect),
                    ],
                    changed: true,
                    stop_requested: false,
                })
            }
            _ => Err(invalid_state(
                "admission storage completion kind does not match the pending operation",
                Some(wrapper_id),
            )),
        }
    }

    pub(super) fn complete_transport_delivery(
        &mut self,
        operation_id: OperationId,
        report: DeliveryReport,
    ) -> Result<ApplyOutcome, CoreError> {
        let pending = self
            .remove_pending_transport(&operation_id)
            .ok_or_else(|| {
                invalid_state(
                    "stale or duplicate transport delivery completion",
                    Some(operation_id.clone()),
                )
            })?;
        self.snapshot.last_delivery = Some(report);
        let disposition = classify_delivery(report);
        if !disposition.commits_domain_decision() {
            let message = match disposition {
                DeliveryCommitDisposition::NoRecipients => {
                    "transport control delivery had no intended recipients"
                }
                DeliveryCommitDisposition::Failed => {
                    "transport control delivery failed for every intended recipient"
                }
                DeliveryCommitDisposition::Delivered
                | DeliveryCommitDisposition::DeliveredWithFailures => {
                    "transport control delivery did not commit"
                }
            };
            let error = transport_delivery_failed(message, operation_id);
            self.snapshot.last_error = Some(error.clone());
            return Ok(ApplyOutcome {
                notifications: vec![CoreNotification::Error(error)],
                changed: true,
                stop_requested: false,
            });
        }

        match pending {
            PendingTransportOperation::ApproveJoin(delivery) => {
                let request =
                    self.remove_pending_request(&delivery.request_id, &delivery.device_id)?;
                self.record_approved_listener(request, &delivery)?;
            }
            PendingTransportOperation::RejectJoin {
                request_id,
                listener_id,
            } => {
                let _ = self.remove_pending_request(&request_id, &listener_id)?;
            }
            PendingTransportOperation::DisconnectListener { listener_id } => {
                let previous = self.snapshot.listeners.len();
                self.snapshot
                    .listeners
                    .retain(|listener| listener.device_id != listener_id);
                if previous == self.snapshot.listeners.len() {
                    return Err(invalid_state(
                        "listener disappeared before removal delivery completed",
                        Some(operation_id),
                    ));
                }
            }
        }

        if self
            .snapshot
            .last_error
            .as_ref()
            .is_some_and(|error| error.code == CoreErrorCode::TransportDeliveryFailed)
        {
            self.snapshot.last_error = None;
        }
        if disposition == DeliveryCommitDisposition::DeliveredWithFailures {
            let mut outcome = self.diagnostic(
                "transport_delivery_partial",
                vec![
                    Self::field("intended_peers", &report.intended_peers.to_string())?,
                    Self::field("successful_peers", &report.successful_peers.to_string())?,
                    Self::field("failed_peers", &report.failed_peers.to_string())?,
                ],
            )?;
            outcome.changed = true;
            Ok(outcome)
        } else {
            Ok(ApplyOutcome::changed())
        }
    }

    pub(super) fn apply_transport_state(
        &mut self,
        state: TransportState,
    ) -> Result<ApplyOutcome, CoreError> {
        let outcome = self.apply_transport(TransportEvent::StateChanged(state))?;
        if self.snapshot.selected_role == Some(AppRole::Host)
            && self.snapshot.host_lifecycle == HostLifecycle::CreatingSession
            && state == TransportState::Advertising
        {
            self.snapshot.host_lifecycle = HostLifecycle::Advertising;
        }
        Ok(outcome)
    }

    pub(super) fn apply_audio_with_host_lifecycle(
        &mut self,
        event: AudioEvent,
    ) -> Result<ApplyOutcome, CoreError> {
        let transition = match &event {
            AudioEvent::PlaybackStateChanged(state) => Some(*state),
            AudioEvent::EndOfStream { .. } => Some(PlaybackState::Stopped),
            AudioEvent::Failed(_) => Some(PlaybackState::Error),
            AudioEvent::PositionAdvanced { .. }
            | AudioEvent::SynchronizationUpdated { .. }
            | AudioEvent::Underrun { .. } => None,
        };
        let outcome = self.apply_audio(event)?;
        if self.snapshot.selected_role != Some(AppRole::Host) || self.host_session_id.is_none() {
            return Ok(outcome);
        }
        match transition {
            Some(PlaybackState::Playing) => {
                if !matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Ready | HostLifecycle::Paused | HostLifecycle::Streaming
                ) {
                    return Err(invalid_state(
                        "host playback entered playing outside an active ready session",
                        None,
                    ));
                }
                self.snapshot.host_lifecycle = HostLifecycle::Streaming;
            }
            Some(PlaybackState::Paused) => {
                if !matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Streaming | HostLifecycle::Paused
                ) {
                    return Err(invalid_state(
                        "host playback entered paused outside a streaming session",
                        None,
                    ));
                }
                self.snapshot.host_lifecycle = HostLifecycle::Paused;
            }
            Some(PlaybackState::Stopped) => {
                if matches!(
                    self.snapshot.host_lifecycle,
                    HostLifecycle::Ready
                        | HostLifecycle::Streaming
                        | HostLifecycle::Paused
                        | HostLifecycle::WaitingForListeners
                ) {
                    self.snapshot.host_lifecycle = if self.snapshot.listeners.is_empty() {
                        HostLifecycle::WaitingForListeners
                    } else {
                        HostLifecycle::Ready
                    };
                }
            }
            Some(PlaybackState::Error) => {
                self.snapshot.host_lifecycle = HostLifecycle::Error;
                self.snapshot.recoverable_action = Some(RecoverableAction::ReselectAudioSource);
            }
            Some(
                PlaybackState::Buffering
                | PlaybackState::Ready
                | PlaybackState::Underrun,
            )
            | None => {}
        }
        Ok(outcome)
    }

    fn require_host_admission_event(&self) -> Result<(), CoreError> {
        self.require_role_event(AppRole::Host)?;
        if !matches!(
            self.snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners
                | HostLifecycle::Ready
                | HostLifecycle::Streaming
                | HostLifecycle::Paused
        ) {
            return Err(invalid_state(
                "join request arrived while host admission was unavailable",
                None,
            ));
        }
        Ok(())
    }

    fn require_host_admission_command(
        &self,
        operation_id: &OperationId,
    ) -> Result<(), CoreError> {
        self.require_role(AppRole::Host, operation_id)?;
        if self.host_session_id.is_none()
            || !matches!(
                self.snapshot.host_lifecycle,
                HostLifecycle::WaitingForListeners
                    | HostLifecycle::Ready
                    | HostLifecycle::Streaming
                    | HostLifecycle::Paused
            )
        {
            return Err(invalid_state(
                "host admission command requires an active host session",
                Some(operation_id.clone()),
            ));
        }
        Ok(())
    }

    fn pending_join_request(
        &self,
        request_id: &RequestId,
        operation_id: &OperationId,
    ) -> Result<&JoinRequestSummary, CoreError> {
        self.snapshot
            .pending_join_requests
            .iter()
            .find(|request| &request.request_id == request_id)
            .ok_or_else(|| {
                invalid_state(
                    "join request is stale or no longer pending",
                    Some(operation_id.clone()),
                )
            })
    }

    fn ensure_request_operation_idle(
        &self,
        request_id: &RequestId,
        operation_id: &OperationId,
    ) -> Result<(), CoreError> {
        let transport_pending = self.pending_transport.iter().any(|(_, pending)| match pending {
            PendingTransportOperation::ApproveJoin(delivery) => {
                &delivery.request_id == request_id
            }
            PendingTransportOperation::RejectJoin {
                request_id: pending_id,
                ..
            } => pending_id == request_id,
            PendingTransportOperation::DisconnectListener { .. } => false,
        });
        let storage_pending = self.pending_storage.iter().any(|(_, pending)| match pending {
            PendingStorageOperation::PersistApprovalTrust(persistence) => {
                &persistence.request_id == request_id
            }
            PendingStorageOperation::PersistTuning(_) => false,
        });
        if transport_pending || storage_pending {
            return Err(invalid_state(
                "join request already has an admission operation in progress",
                Some(operation_id.clone()),
            ));
        }
        Ok(())
    }

    fn start_approval_delivery(
        &mut self,
        delivery: ApprovalDelivery,
    ) -> Result<ApplyOutcome, CoreError> {
        let effect = self.create_approval_transport_effect(delivery)?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    fn create_approval_transport_effect(
        &mut self,
        delivery: ApprovalDelivery,
    ) -> Result<crate::runtime::TransportEffect, CoreError> {
        let session_id = self
            .host_session_id
            .clone()
            .ok_or_else(|| invalid_state("host session is not active", None))?;
        self.start_transport_operation(
            TransportEffectRequest::DeliverJoinApproval {
                request_id: delivery.request_id.clone(),
                session_id,
                listener_id: delivery.device_id.clone(),
                trusted_for_future: delivery.trusted_for_future,
            },
            PendingTransportOperation::ApproveJoin(delivery),
        )
    }

    fn start_rejection_delivery(
        &mut self,
        request: &JoinRequestSummary,
        reason: JoinRejectionReason,
    ) -> Result<ApplyOutcome, CoreError> {
        let session_id = self
            .host_session_id
            .clone()
            .ok_or_else(|| invalid_state("host session is not active", None))?;
        let effect = self.start_transport_operation(
            TransportEffectRequest::DeliverJoinRejection {
                request_id: request.request_id.clone(),
                session_id,
                listener_id: request.device_id.clone(),
                reason_code: reason.stable_name().to_owned(),
            },
            PendingTransportOperation::RejectJoin {
                request_id: request.request_id.clone(),
                listener_id: request.device_id.clone(),
            },
        )?;
        Ok(ApplyOutcome::transport_effect(effect))
    }

    fn record_approved_listener(
        &mut self,
        request: JoinRequestSummary,
        delivery: &ApprovalDelivery,
    ) -> Result<(), CoreError> {
        let trust_state = if delivery.trusted_for_future {
            TrustState::Trusted
        } else {
            TrustState::SessionOnly
        };
        let listener = ListenerSummary::new(
            request.device_id,
            request.display_name,
            trust_state,
            TransportState::Connecting,
        )
        .map_err(|error| super::invalid_argument(error.to_string(), None))?;
        if let Some(existing) = self
            .snapshot
            .listeners
            .iter_mut()
            .find(|existing| existing.device_id == listener.device_id)
        {
            *existing = listener;
            return Ok(());
        }
        if self.snapshot.listeners.len() >= MAX_CONNECTED_LISTENERS {
            return Err(super::resource_limit(
                "connected-listener capacity reached",
                None,
            ));
        }
        self.snapshot.listeners.push(listener);
        Ok(())
    }

    fn remove_pending_request(
        &mut self,
        request_id: &RequestId,
        listener_id: &DeviceId,
    ) -> Result<JoinRequestSummary, CoreError> {
        let index = self
            .snapshot
            .pending_join_requests
            .iter()
            .position(|request| {
                &request.request_id == request_id && &request.device_id == listener_id
            })
            .ok_or_else(|| {
                invalid_state(
                    "join request disappeared before delivery completed",
                    None,
                )
            })?;
        Ok(self.snapshot.pending_join_requests.remove(index))
    }
}
