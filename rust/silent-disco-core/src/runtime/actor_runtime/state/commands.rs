use super::{
    ActorState, AppRole, ApplyOutcome, CoreCommand, CoreError, DiscoveryRequest, HostLifecycle,
    ListenerLifecycle, NetworkEstablishmentRequest, OperationId, PendingPlatformOperation,
    PlatformEffectRequest, RecoverableAction, SessionAdvertisement, SessionId, TransportState,
    current_protocol_version, invalid_argument, invalid_state,
};

impl ActorState {
    pub(super) fn apply_command(
        &mut self,
        operation_id: OperationId,
        command: CoreCommand,
    ) -> Result<ApplyOutcome, CoreError> {
        match command {
            CoreCommand::SelectRole { role } => self.select_role(operation_id, role),
            CoreCommand::UpdateHostDraft(patch) => self.update_host_draft(operation_id, &patch),
            CoreCommand::CreateHostSession => self.create_host_session(operation_id),
            CoreCommand::EndHostSession => self.end_host_session(operation_id),
            CoreCommand::StartDiscovery => self.start_discovery(operation_id),
            CoreCommand::StopDiscovery => self.stop_discovery(operation_id),
            CoreCommand::SelectSession { session_id } => {
                self.select_session(operation_id, session_id)
            }
            CoreCommand::SubmitJoin { .. } => self.submit_join(operation_id),
            CoreCommand::CancelJoin => self.cancel_join(operation_id),
            CoreCommand::ExportDiagnostics => self.export_diagnostics(),
            CoreCommand::RetryRecoverableFailure => self.retry_recoverable(operation_id),
            CoreCommand::Shutdown => {
                self.snapshot.shutting_down = true;
                Ok(ApplyOutcome {
                    notifications: Vec::new(),
                    changed: true,
                    stop_requested: true,
                })
            }
            CoreCommand::ApproveJoin { .. }
            | CoreCommand::RejectJoin { .. }
            | CoreCommand::RemoveListener { .. } => Err(invalid_state(
                "listener admission requires shared migration Block 12",
                Some(operation_id),
            )),
            CoreCommand::StartPlayback { .. }
            | CoreCommand::PausePlayback
            | CoreCommand::ResumePlayback
            | CoreCommand::StopPlayback
            | CoreCommand::SetLocalVolume { .. }
            | CoreCommand::RequestResync => Err(invalid_state(
                "playback requires the shared packetizer and scheduler blocks",
                Some(operation_id),
            )),
            CoreCommand::UpdateTuning(_) => Err(invalid_state(
                "tuning updates require correlated durable storage integration",
                Some(operation_id),
            )),
        }
    }

    pub(super) fn select_role(
        &mut self,
        operation_id: OperationId,
        role: AppRole,
    ) -> Result<ApplyOutcome, CoreError> {
        if self.snapshot.host_lifecycle != HostLifecycle::Idle
            || self.snapshot.listener_lifecycle != ListenerLifecycle::Idle
            || !self.pending_platform.is_empty()
            || !self.pending_transport.is_empty()
            || !self.pending_storage.is_empty()
        {
            return Err(invalid_state(
                "role cannot change while a session or operation is active",
                Some(operation_id),
            ));
        }
        if self.snapshot.selected_role == Some(role) {
            return Ok(ApplyOutcome::default());
        }
        self.snapshot.selected_role = Some(role);
        self.clear_failure();
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn update_host_draft(
        &mut self,
        operation_id: OperationId,
        patch: &crate::runtime::types::HostDraftPatch,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        if !matches!(
            self.snapshot.host_lifecycle,
            HostLifecycle::Idle | HostLifecycle::Error
        ) {
            return Err(invalid_state(
                "host draft cannot change while a session is active",
                Some(operation_id),
            ));
        }
        self.snapshot.host_draft = self
            .snapshot
            .host_draft
            .patched(patch)
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id)))?;
        self.clear_failure();
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn create_host_session(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        if !matches!(
            self.snapshot.host_lifecycle,
            HostLifecycle::Idle | HostLifecycle::Error
        ) {
            return Err(invalid_state(
                "host session creation is not legal now",
                Some(operation_id),
            ));
        }
        self.snapshot
            .host_draft
            .validate_for_creation()
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        let session_id = self.next_session_id()?;
        let effect = self.start_host_advertising(operation_id, session_id.clone())?;
        self.host_session_id = Some(session_id);
        self.snapshot.host_lifecycle = HostLifecycle::CreatingSession;
        self.snapshot.transport_state = TransportState::Idle;
        self.clear_failure();
        Ok(ApplyOutcome::effect(effect))
    }

    pub(super) fn end_host_session(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        if self.host_session_id.is_none()
            || matches!(
                self.snapshot.host_lifecycle,
                HostLifecycle::Idle | HostLifecycle::EndingSession
            )
        {
            return Err(invalid_state(
                "no active host session can be ended",
                Some(operation_id),
            ));
        }
        let effect = self.start_platform_operation(
            PlatformEffectRequest::StopAdvertising,
            PendingPlatformOperation::StopAdvertising,
        )?;
        self.snapshot.host_lifecycle = HostLifecycle::EndingSession;
        Ok(ApplyOutcome::effect(effect))
    }

    pub(super) fn start_discovery(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Listener, &operation_id)?;
        if self.snapshot.discovery_active
            || self
                .pending_platform
                .iter()
                .any(|(_, pending)| pending == &PendingPlatformOperation::StartDiscovery)
        {
            return Err(invalid_state(
                "discovery is already active or starting",
                Some(operation_id),
            ));
        }
        if !matches!(
            self.snapshot.listener_lifecycle,
            ListenerLifecycle::Idle | ListenerLifecycle::Disconnected | ListenerLifecycle::Error
        ) {
            return Err(invalid_state(
                "discovery cannot start in the current lifecycle",
                Some(operation_id),
            ));
        }
        let effect = self.start_platform_operation(
            PlatformEffectRequest::StartDiscovery(DiscoveryRequest::from_tuning(
                &self.snapshot.tuning,
            )),
            PendingPlatformOperation::StartDiscovery,
        )?;
        self.clear_failure();
        Ok(ApplyOutcome::effect(effect))
    }

    pub(super) fn stop_discovery(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Listener, &operation_id)?;
        if !self.snapshot.discovery_active {
            return Err(invalid_state("discovery is not active", Some(operation_id)));
        }
        let effect = self.start_platform_operation(
            PlatformEffectRequest::StopDiscovery,
            PendingPlatformOperation::StopDiscovery,
        )?;
        Ok(ApplyOutcome::effect(effect))
    }

    pub(super) fn select_session(
        &mut self,
        operation_id: OperationId,
        session_id: SessionId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Listener, &operation_id)?;
        if !self
            .snapshot
            .discovered_sessions
            .iter()
            .any(|session| session.session_id == session_id)
        {
            return Err(invalid_argument(
                "selected session is not in discovery results",
                Some(operation_id),
            ));
        }
        if !matches!(
            self.snapshot.listener_lifecycle,
            ListenerLifecycle::Scanning | ListenerLifecycle::SessionSelected
        ) {
            return Err(invalid_state(
                "session selection is not legal now",
                Some(operation_id),
            ));
        }
        self.snapshot.selected_session = Some(session_id);
        self.snapshot.listener_lifecycle = ListenerLifecycle::SessionSelected;
        self.clear_failure();
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn submit_join(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Listener, &operation_id)?;
        if self.snapshot.listener_lifecycle != ListenerLifecycle::SessionSelected {
            return Err(invalid_state(
                "join requires a selected session",
                Some(operation_id),
            ));
        }
        let selected_id = self.snapshot.selected_session.as_ref().ok_or_else(|| {
            invalid_state("join has no selected session", Some(operation_id.clone()))
        })?;
        let advertisement = self
            .snapshot
            .discovered_sessions
            .iter()
            .find(|session| &session.session_id == selected_id)
            .ok_or_else(|| {
                invalid_state(
                    "selected session disappeared before join",
                    Some(operation_id.clone()),
                )
            })?;
        let endpoint = advertisement.endpoint.ok_or_else(|| {
            invalid_state(
                "selected session has no network endpoint",
                Some(operation_id),
            )
        })?;
        let session_id = advertisement.session_id.clone();
        let effect = self.start_platform_operation(
            PlatformEffectRequest::EstablishNetwork(NetworkEstablishmentRequest {
                session_id: session_id.clone(),
                endpoint,
            }),
            PendingPlatformOperation::EstablishNetwork { session_id },
        )?;
        self.snapshot.listener_lifecycle = ListenerLifecycle::JoinRequested;
        self.snapshot.transport_state = TransportState::Connecting;
        self.snapshot.discovery_active = false;
        Ok(ApplyOutcome::effect(effect))
    }

    pub(super) fn cancel_join(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Listener, &operation_id)?;
        if matches!(
            self.snapshot.transport_state,
            TransportState::Connecting | TransportState::Connected | TransportState::Retrying
        ) {
            let effect = self.start_platform_operation(
                PlatformEffectRequest::ReleaseNetwork,
                PendingPlatformOperation::ReleaseNetwork,
            )?;
            return Ok(ApplyOutcome::effect(effect));
        }
        self.snapshot.selected_session = None;
        self.snapshot.listener_lifecycle = ListenerLifecycle::Idle;
        self.snapshot.transport_state = TransportState::Idle;
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn export_diagnostics(&mut self) -> Result<ApplyOutcome, CoreError> {
        let export_id = self.next_export_id()?;
        let effect = self.start_platform_operation(
            PlatformEffectRequest::ShareDiagnostics {
                export_id: export_id.clone(),
            },
            PendingPlatformOperation::ShareDiagnostics { export_id },
        )?;
        Ok(ApplyOutcome::effect(effect))
    }

    pub(super) fn retry_recoverable(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        let error = self.snapshot.last_error.as_ref().ok_or_else(|| {
            invalid_state(
                "there is no recoverable failure to retry",
                Some(operation_id.clone()),
            )
        })?;
        let action = self.snapshot.recoverable_action.ok_or_else(|| {
            invalid_state(
                "the current failure has no recoverable action",
                Some(operation_id.clone()),
            )
        })?;
        if !error.retryable {
            return Err(invalid_state(
                "the current failure is not retryable",
                Some(operation_id),
            ));
        }

        if self.snapshot.selected_role == Some(AppRole::Host)
            && action == RecoverableAction::Retry
        {
            return self.retry_host_session(operation_id);
        }

        self.clear_failure();
        Ok(ApplyOutcome::changed())
    }

    fn retry_host_session(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role(AppRole::Host, &operation_id)?;
        if !self.pending_platform.is_empty()
            || !self.pending_transport.is_empty()
            || !self.pending_storage.is_empty()
        {
            return Err(invalid_state(
                "host retry cannot start while another operation is pending",
                Some(operation_id),
            ));
        }
        self.snapshot
            .host_draft
            .validate_for_creation()
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        let session_id = self.host_session_id.clone().ok_or_else(|| {
            invalid_state(
                "host retry has no active session identity",
                Some(operation_id.clone()),
            )
        })?;
        let effect = self.start_host_advertising(operation_id, session_id)?;
        self.snapshot.host_lifecycle = HostLifecycle::CreatingSession;
        self.snapshot.transport_state = TransportState::Idle;
        self.clear_failure();
        Ok(ApplyOutcome::effect(effect))
    }

    fn start_host_advertising(
        &mut self,
        operation_id: OperationId,
        session_id: SessionId,
    ) -> Result<super::PlatformEffect, CoreError> {
        let advertisement = SessionAdvertisement::new(
            session_id.clone(),
            self.local_device_id.clone(),
            self.snapshot.host_draft.session_name.clone(),
            self.snapshot.host_draft.approval_mode,
            current_protocol_version(),
            None,
        )
        .map_err(|error| invalid_argument(error.to_string(), Some(operation_id)))?;
        self.start_platform_operation(
            PlatformEffectRequest::StartAdvertising(advertisement),
            PendingPlatformOperation::StartAdvertising { session_id },
        )
    }
}
