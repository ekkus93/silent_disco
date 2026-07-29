use super::{
    ActorState, AppRole, ApplyOutcome, CoreDiagnostic, CoreError, CoreNotification,
    DiagnosticField, HostLifecycle, ListenerLifecycle, MAX_PENDING_EFFECT_OPERATIONS, OperationId,
    PendingPlatformOperation, PendingStorageOperation, PendingTransportOperation, PlatformEffect,
    PlatformEffectRequest, PlaybackState, RecoverableAction, SessionId, StorageEffect,
    StorageEffectRequest, TransportEffect, TransportEffectRequest, TransportState, invalid_argument,
    invalid_state, resource_limit,
};

impl ActorState {
    pub(super) fn start_platform_operation(
        &mut self,
        request: PlatformEffectRequest,
        pending: PendingPlatformOperation,
    ) -> Result<PlatformEffect, CoreError> {
        self.ensure_effect_capacity()?;
        let operation_id = self.next_effect_id()?;
        let effect = PlatformEffect::new(operation_id.clone(), request)
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        self.pending_platform.push((operation_id, pending));
        Ok(effect)
    }

    pub(super) fn start_transport_operation(
        &mut self,
        request: TransportEffectRequest,
        pending: PendingTransportOperation,
    ) -> Result<TransportEffect, CoreError> {
        self.ensure_effect_capacity()?;
        let operation_id = self.next_effect_id()?;
        let effect = TransportEffect::new(operation_id.clone(), request)
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        self.pending_transport.push((operation_id, pending));
        Ok(effect)
    }

    pub(super) fn start_storage_operation(
        &mut self,
        request: StorageEffectRequest,
        pending: PendingStorageOperation,
    ) -> Result<StorageEffect, CoreError> {
        self.ensure_effect_capacity()?;
        let operation_id = self.next_effect_id()?;
        let effect = StorageEffect::new(operation_id.clone(), request)
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        self.pending_storage.push((operation_id, pending));
        Ok(effect)
    }

    fn ensure_effect_capacity(&self) -> Result<(), CoreError> {
        let pending = self
            .pending_platform
            .len()
            .checked_add(self.pending_transport.len())
            .and_then(|count| count.checked_add(self.pending_storage.len()))
            .ok_or_else(|| resource_limit("pending effect-operation count overflow", None))?;
        if pending >= MAX_PENDING_EFFECT_OPERATIONS {
            return Err(resource_limit(
                "pending effect-operation capacity reached",
                None,
            ));
        }
        Ok(())
    }

    pub(super) fn pending(&self, operation_id: &OperationId) -> Option<&PendingPlatformOperation> {
        self.pending_platform
            .iter()
            .find(|(candidate, _)| candidate == operation_id)
            .map(|(_, pending)| pending)
    }

    pub(super) fn remove_pending(
        &mut self,
        operation_id: &OperationId,
    ) -> Option<PendingPlatformOperation> {
        let index = self
            .pending_platform
            .iter()
            .position(|(candidate, _)| candidate == operation_id)?;
        Some(self.pending_platform.remove(index).1)
    }

    pub(super) fn pending_transport(
        &self,
        operation_id: &OperationId,
    ) -> Option<&PendingTransportOperation> {
        self.pending_transport
            .iter()
            .find(|(candidate, _)| candidate == operation_id)
            .map(|(_, pending)| pending)
    }

    pub(super) fn remove_pending_transport(
        &mut self,
        operation_id: &OperationId,
    ) -> Option<PendingTransportOperation> {
        let index = self
            .pending_transport
            .iter()
            .position(|(candidate, _)| candidate == operation_id)?;
        Some(self.pending_transport.remove(index).1)
    }

    pub(super) fn pending_storage(
        &self,
        operation_id: &OperationId,
    ) -> Option<&PendingStorageOperation> {
        self.pending_storage
            .iter()
            .find(|(candidate, _)| candidate == operation_id)
            .map(|(_, pending)| pending)
    }

    pub(super) fn remove_pending_storage(
        &mut self,
        operation_id: &OperationId,
    ) -> Option<PendingStorageOperation> {
        let index = self
            .pending_storage
            .iter()
            .position(|(candidate, _)| candidate == operation_id)?;
        Some(self.pending_storage.remove(index).1)
    }

    pub(super) fn apply_pending_failure(
        &mut self,
        pending: &PendingPlatformOperation,
        error: &CoreError,
    ) {
        match pending {
            PendingPlatformOperation::StartAdvertising { .. }
            | PendingPlatformOperation::StopAdvertising => {
                self.snapshot.host_lifecycle = HostLifecycle::Error;
                self.snapshot.transport_state = TransportState::Failed;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Retry);
            }
            PendingPlatformOperation::StartDiscovery | PendingPlatformOperation::StopDiscovery => {
                self.snapshot.discovery_active = false;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.transport_state = TransportState::Failed;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Rescan);
            }
            PendingPlatformOperation::EstablishNetwork { .. }
            | PendingPlatformOperation::ReleaseNetwork => {
                self.snapshot.discovery_active = false;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.transport_state = TransportState::Failed;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Reconnect);
            }
            PendingPlatformOperation::ShareDiagnostics { .. } => {}
        }
    }

    pub(super) fn next_effect_id(&mut self) -> Result<OperationId, CoreError> {
        let sequence = self.next_effect_sequence;
        self.next_effect_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("effect operation identifier overflow", None))?;
        OperationId::new(format!("effect-{sequence}"))
            .map_err(|error| CoreError::from_identifier_validation(&error, None))
    }

    pub(super) fn next_session_id(&mut self) -> Result<SessionId, CoreError> {
        let sequence = self.next_session_sequence;
        self.next_session_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("session identifier overflow", None))?;
        SessionId::new(format!("session-{sequence}"))
            .map_err(|error| CoreError::from_identifier_validation(&error, None))
    }

    pub(super) fn next_export_id(&mut self) -> Result<String, CoreError> {
        let sequence = self.next_export_sequence;
        self.next_export_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("diagnostics export identifier overflow", None))?;
        Ok(format!("diagnostics-{sequence}"))
    }

    pub(super) fn require_role(
        &self,
        role: AppRole,
        operation_id: &OperationId,
    ) -> Result<(), CoreError> {
        if self.snapshot.selected_role == Some(role) {
            Ok(())
        } else {
            Err(invalid_state(
                format!("command requires the {role} role"),
                Some(operation_id.clone()),
            ))
        }
    }

    pub(super) fn require_role_event(&self, role: AppRole) -> Result<(), CoreError> {
        if self.snapshot.selected_role == Some(role) {
            Ok(())
        } else {
            Err(invalid_state(
                format!("event requires the {role} role"),
                None,
            ))
        }
    }

    pub(super) fn require_discovery_active(&self) -> Result<(), CoreError> {
        if self.snapshot.discovery_active {
            Ok(())
        } else {
            Err(invalid_state(
                "discovery result arrived while discovery was inactive",
                None,
            ))
        }
    }

    pub(super) fn reset_host_session(&mut self) {
        self.host_session_id = None;
        self.pending_transport.clear();
        self.pending_storage.clear();
        self.snapshot.host_lifecycle = HostLifecycle::Idle;
        self.snapshot.transport_state = TransportState::Idle;
        self.snapshot.pending_join_requests.clear();
        self.snapshot.listeners.clear();
        self.snapshot.playback_state = PlaybackState::Stopped;
        self.snapshot.playback_position_ms = 0;
        self.snapshot.last_delivery = None;
    }

    pub(super) fn reset_listener_connection(&mut self) {
        self.snapshot.discovery_active = false;
        self.snapshot.transport_state = TransportState::Idle;
        self.snapshot.listener_lifecycle = ListenerLifecycle::Idle;
        self.snapshot.selected_session = None;
        self.snapshot.playback_state = PlaybackState::Stopped;
    }

    pub(super) fn clear_failure(&mut self) {
        self.snapshot.last_error = None;
        self.snapshot.recoverable_action = None;
    }

    pub(super) fn field(key: &str, value: &str) -> Result<DiagnosticField, CoreError> {
        DiagnosticField::new(key, value).map_err(|error| invalid_argument(error.to_string(), None))
    }

    pub(super) fn diagnostic(
        &self,
        name: &str,
        fields: Vec<DiagnosticField>,
    ) -> Result<ApplyOutcome, CoreError> {
        let diagnostic = CoreDiagnostic::new(name, fields)
            .map_err(|error| invalid_argument(error.to_string(), None))?;
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Diagnostic(diagnostic)],
            changed: false,
            stop_requested: false,
        })
    }
}
