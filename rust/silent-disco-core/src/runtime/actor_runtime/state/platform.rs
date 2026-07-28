use super::{
    ActorState, ApplyOutcome, CoreError, CoreNotification, HostLifecycle, ListenerLifecycle,
    MAX_DISCOVERED_SESSIONS, OperationId, PendingPlatformOperation, PlatformEvent,
    PlatformOperationCompletion, SessionAdvertisement, SessionId, TransportState, invalid_state,
    resource_limit,
};

impl ActorState {
    pub(super) fn apply_platform(
        &mut self,
        event: PlatformEvent,
    ) -> Result<ApplyOutcome, CoreError> {
        match event {
            PlatformEvent::OperationSucceeded {
                operation_id,
                completion,
            } => self.apply_platform_success(operation_id, completion),
            PlatformEvent::OperationFailed {
                operation_id,
                mut error,
            } => self.apply_platform_failure(operation_id, &mut error),
            PlatformEvent::SessionDiscovered(advertisement) => {
                self.record_discovered_session(advertisement)
            }
            PlatformEvent::SessionExpired { session_id } => self.expire_session(&session_id),
            PlatformEvent::CapabilityStateChanged(capabilities) => {
                if capabilities == self.snapshot.capabilities {
                    return Ok(ApplyOutcome::default());
                }
                self.snapshot.capabilities = capabilities;
                Ok(ApplyOutcome::changed())
            }
            PlatformEvent::AppEnteredForeground => self.diagnostic("app_foreground", Vec::new()),
            PlatformEvent::AppEnteredBackground => self.diagnostic("app_background", Vec::new()),
        }
    }

    pub(super) fn apply_platform_failure(
        &mut self,
        operation_id: OperationId,
        error: &mut CoreError,
    ) -> Result<ApplyOutcome, CoreError> {
        let pending = self.remove_pending(&operation_id).ok_or_else(|| {
            invalid_state(
                "stale or duplicate platform failure completion",
                Some(operation_id.clone()),
            )
        })?;
        if let Some(inner_id) = &error.operation_id
            && inner_id != &operation_id
        {
            return Err(invalid_state(
                "platform failure operation ID does not match its wrapper",
                Some(operation_id),
            ));
        }
        error.operation_id = Some(operation_id);
        self.apply_pending_failure(&pending, error);
        self.snapshot.last_error = Some(error.clone());
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Error(error.clone())],
            changed: true,
            stop_requested: false,
        })
    }

    pub(super) fn record_discovered_session(
        &mut self,
        advertisement: SessionAdvertisement,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_discovery_active()?;
        if let Some(existing) = self
            .snapshot
            .discovered_sessions
            .iter_mut()
            .find(|session| session.session_id == advertisement.session_id)
        {
            *existing = advertisement;
        } else {
            if self.snapshot.discovered_sessions.len() >= MAX_DISCOVERED_SESSIONS {
                return Err(resource_limit("discovered-session capacity reached", None));
            }
            self.snapshot.discovered_sessions.push(advertisement);
        }
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn expire_session(
        &mut self,
        session_id: &SessionId,
    ) -> Result<ApplyOutcome, CoreError> {
        let previous = self.snapshot.discovered_sessions.len();
        self.snapshot
            .discovered_sessions
            .retain(|session| &session.session_id != session_id);
        if previous == self.snapshot.discovered_sessions.len() {
            return Ok(ApplyOutcome::default());
        }
        if self.snapshot.selected_session.as_ref() == Some(session_id) {
            self.snapshot.selected_session = None;
            self.snapshot.listener_lifecycle = if self.snapshot.discovery_active {
                ListenerLifecycle::Scanning
            } else {
                ListenerLifecycle::Idle
            };
        }
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn apply_platform_success(
        &mut self,
        operation_id: OperationId,
        completion: PlatformOperationCompletion,
    ) -> Result<ApplyOutcome, CoreError> {
        let pending = self.pending(&operation_id).cloned().ok_or_else(|| {
            invalid_state(
                "stale or duplicate platform success completion",
                Some(operation_id.clone()),
            )
        })?;
        let mut outcome = ApplyOutcome::changed();
        match (&pending, completion) {
            (
                PendingPlatformOperation::StartAdvertising { session_id },
                PlatformOperationCompletion::AdvertisingStarted,
            ) if self.host_session_id.as_ref() == Some(session_id) => {
                self.snapshot.host_lifecycle = HostLifecycle::WaitingForListeners;
                self.snapshot.transport_state = TransportState::Advertising;
            }
            (
                PendingPlatformOperation::StopAdvertising,
                PlatformOperationCompletion::AdvertisingStopped,
            ) => self.reset_host_session(),
            (
                PendingPlatformOperation::StartDiscovery,
                PlatformOperationCompletion::DiscoveryStarted,
            ) => {
                self.snapshot.discovery_active = true;
                self.snapshot.transport_state = TransportState::Discovering;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Scanning;
            }
            (
                PendingPlatformOperation::StopDiscovery,
                PlatformOperationCompletion::DiscoveryStopped,
            ) => {
                self.snapshot.discovery_active = false;
                self.snapshot.transport_state = TransportState::Idle;
                if self.snapshot.listener_lifecycle == ListenerLifecycle::Scanning {
                    self.snapshot.listener_lifecycle = ListenerLifecycle::Idle;
                }
            }
            (
                PendingPlatformOperation::EstablishNetwork { session_id },
                PlatformOperationCompletion::NetworkEndpointReady(_),
            ) if self.snapshot.selected_session.as_ref() == Some(session_id) => {
                self.snapshot.discovery_active = false;
                self.snapshot.transport_state = TransportState::Connected;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Connecting;
            }
            (
                PendingPlatformOperation::ReleaseNetwork,
                PlatformOperationCompletion::NetworkReleased,
            ) => self.reset_listener_connection(),
            (
                PendingPlatformOperation::ShareDiagnostics {
                    export_id: expected,
                },
                PlatformOperationCompletion::DiagnosticsShared { export_id },
            ) if expected == &export_id => {
                outcome = self.diagnostic(
                    "diagnostics_shared",
                    vec![Self::field("export_id", &export_id)?],
                )?;
                outcome.changed = true;
            }
            _ => {
                return Err(invalid_state(
                    "platform completion kind does not match the pending operation",
                    Some(operation_id),
                ));
            }
        }
        self.remove_pending(&operation_id);
        self.clear_failure();
        Ok(outcome)
    }
}
