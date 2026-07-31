use super::{
    ActorState, AppRole, ApplyOutcome, CoreError, CoreNotification, DeviceId, HostLifecycle,
    ListenerLifecycle, MAX_CONNECTED_LISTENERS, MAX_PENDING_JOIN_REQUESTS, PlaybackState,
    RecoverableAction, TransportEvent, TransportState, invalid_state, resource_limit,
};

impl ActorState {
    pub(super) fn apply_transport(
        &mut self,
        event: TransportEvent,
    ) -> Result<ApplyOutcome, CoreError> {
        match event {
            TransportEvent::StateChanged(state) => {
                self.snapshot.transport_state = state;
                self.snapshot.discovery_active = state == TransportState::Discovering;
                Ok(ApplyOutcome::changed())
            }
            TransportEvent::JoinRequested(request) => self.record_join_request(request),
            TransportEvent::ListenerConnected(listener) => self.record_listener(listener),
            TransportEvent::ListenerDisconnected { device_id, error } => {
                self.record_listener_disconnect(&device_id, error)
            }
            TransportEvent::DeliveryCompleted { report, .. } => {
                self.snapshot.last_delivery = Some(report);
                Ok(ApplyOutcome::changed())
            }
            TransportEvent::SessionEnded { session_id } => {
                if self.snapshot.selected_session.as_ref() != Some(&session_id) {
                    return Ok(ApplyOutcome::default());
                }
                self.snapshot.listener_lifecycle = ListenerLifecycle::Disconnected;
                self.snapshot.transport_state = TransportState::Disconnected;
                self.snapshot.playback_state = PlaybackState::Stopped;
                self.snapshot.recoverable_action = Some(RecoverableAction::Reconnect);
                Ok(ApplyOutcome::changed())
            }
            TransportEvent::Failed(error) => self.record_transport_failure(error),
        }
    }

    pub(super) fn record_join_request(
        &mut self,
        request: crate::runtime::types::JoinRequestSummary,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role_event(AppRole::Host)?;
        if !matches!(
            self.snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners | HostLifecycle::Ready | HostLifecycle::Streaming
        ) {
            return Err(invalid_state(
                "join request arrived while host admission was unavailable",
                None,
            ));
        }
        if self
            .snapshot
            .pending_join_requests
            .iter()
            .any(|existing| existing.request_id == request.request_id)
        {
            return Err(invalid_state("duplicate join request identifier", None));
        }
        if self.snapshot.pending_join_requests.len() >= MAX_PENDING_JOIN_REQUESTS {
            return Err(resource_limit(
                "pending join-request capacity reached",
                None,
            ));
        }
        self.snapshot.pending_join_requests.push(request);
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn record_listener(
        &mut self,
        listener: crate::runtime::types::ListenerSummary,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role_event(AppRole::Host)?;
        self.snapshot
            .pending_join_requests
            .retain(|request| request.device_id != listener.device_id);
        if let Some(existing) = self
            .snapshot
            .listeners
            .iter_mut()
            .find(|existing| existing.device_id == listener.device_id)
        {
            *existing = listener;
        } else {
            if self.snapshot.listeners.len() >= MAX_CONNECTED_LISTENERS {
                return Err(resource_limit("connected-listener capacity reached", None));
            }
            self.snapshot.listeners.push(listener);
        }
        if self.snapshot.host_lifecycle == HostLifecycle::WaitingForListeners {
            self.snapshot.host_lifecycle = HostLifecycle::Ready;
        }
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn record_listener_disconnect(
        &mut self,
        device_id: &DeviceId,
        error: Option<CoreError>,
    ) -> Result<ApplyOutcome, CoreError> {
        let previous_listeners = self.snapshot.listeners.len();
        let previous_requests = self.snapshot.pending_join_requests.len();
        self.snapshot
            .listeners
            .retain(|listener| &listener.device_id != device_id);
        self.snapshot
            .pending_join_requests
            .retain(|request| &request.device_id != device_id);
        let listener_removed = previous_listeners != self.snapshot.listeners.len();
        let request_removed = previous_requests != self.snapshot.pending_join_requests.len();
        if listener_removed
            && self.snapshot.listeners.is_empty()
            && self.snapshot.host_lifecycle == HostLifecycle::Ready
        {
            self.snapshot.host_lifecycle = HostLifecycle::WaitingForListeners;
        }
        if let Some(error) = error {
            self.snapshot.last_error = Some(error.clone());
            return Ok(ApplyOutcome {
                notifications: vec![CoreNotification::Error(error)],
                changed: true,
                stop_requested: false,
            });
        }
        if !listener_removed && !request_removed {
            return Ok(ApplyOutcome::default());
        }
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn record_transport_failure(
        &mut self,
        error: CoreError,
    ) -> Result<ApplyOutcome, CoreError> {
        self.snapshot.last_error = Some(error.clone());
        self.snapshot.transport_state = TransportState::Failed;
        match self.snapshot.selected_role {
            Some(AppRole::Host) => {
                self.snapshot.host_lifecycle = HostLifecycle::Error;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Retry);
            }
            Some(AppRole::Listener) => {
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Reconnect);
            }
            None => {}
        }
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Error(error)],
            changed: true,
            stop_requested: false,
        })
    }
}
