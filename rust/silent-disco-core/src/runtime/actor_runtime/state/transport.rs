use super::{
    ActorState, AppRole, ApplyOutcome, CoreError, CoreNotification, DeviceId, HostLifecycle,
    ListenerLifecycle, MAX_CONNECTED_LISTENERS, MAX_PENDING_JOIN_REQUESTS, PlaybackState,
    RecoverableAction, TransportEvent, TransportState, invalid_state, join_rejected,
    resource_limit,
};

/// Listener lifecycle states in which the listener is actively engaged with
/// a host (mid-join or connected), as opposed to merely idle or already
/// terminal. Mirrors the Kotlin `classifyTransportSnapshotRole` distinction
/// between a genuine listener failure and a stray transport fact that
/// arrived while the listener wasn't doing anything.
const LISTENER_ACTIVE_LIFECYCLES: &[ListenerLifecycle] = &[
    ListenerLifecycle::Scanning,
    ListenerLifecycle::SessionSelected,
    ListenerLifecycle::JoinRequested,
    ListenerLifecycle::AwaitingApproval,
    ListenerLifecycle::Approved,
    ListenerLifecycle::Connecting,
    ListenerLifecycle::SyncingClock,
    ListenerLifecycle::Buffering,
    ListenerLifecycle::Playing,
    ListenerLifecycle::Reconnecting,
    ListenerLifecycle::Desynced,
];

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
            TransportEvent::DeliveryCompleted {
                operation_id,
                report,
            } => {
                self.snapshot.last_delivery = Some(report);
                self.snapshot.last_delivery_operation_id = Some(operation_id);
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
            TransportEvent::AwaitingApproval => self.record_awaiting_approval(),
            TransportEvent::JoinApproved { trusted_for_future } => {
                self.record_join_approved(trusted_for_future)
            }
            TransportEvent::JoinRejected { reason } => self.record_join_rejected(reason),
        }
    }

    pub(super) fn record_awaiting_approval(&mut self) -> Result<ApplyOutcome, CoreError> {
        self.require_role_event(AppRole::Listener)?;
        if self.snapshot.listener_lifecycle != ListenerLifecycle::Connecting {
            return Err(invalid_state(
                "awaiting approval arrived outside an active connection attempt",
                None,
            ));
        }
        self.snapshot.listener_lifecycle = ListenerLifecycle::AwaitingApproval;
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn record_join_approved(
        &mut self,
        // Persisting trust locally for future auto-reconnect is deferred;
        // see the TransportEvent::JoinApproved doc comment.
        _trusted_for_future: bool,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role_event(AppRole::Listener)?;
        if !matches!(
            self.snapshot.listener_lifecycle,
            ListenerLifecycle::Connecting | ListenerLifecycle::AwaitingApproval
        ) {
            return Err(invalid_state(
                "join approval arrived outside an active join attempt",
                None,
            ));
        }
        self.snapshot.listener_lifecycle = ListenerLifecycle::Approved;
        self.busy_join_retries = 0;
        self.clear_failure();
        Ok(ApplyOutcome::changed())
    }

    pub(super) fn record_join_rejected(
        &mut self,
        reason: String,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role_event(AppRole::Listener)?;
        if !matches!(
            self.snapshot.listener_lifecycle,
            ListenerLifecycle::Connecting | ListenerLifecycle::AwaitingApproval
        ) {
            return Err(invalid_state(
                "join rejection arrived outside an active join attempt",
                None,
            ));
        }
        if crate::runtime::is_host_busy_rejection(&reason)
            && let Some(retry) = crate::runtime::next_busy_join_retry(self.busy_join_retries)
            && let Some(outcome) = self.retry_busy_join(retry)?
        {
            return Ok(outcome);
        }
        let reason = if crate::runtime::is_host_busy_rejection(&reason) {
            format!(
                "the host is busy (tried {} more times); try again later",
                self.busy_join_retries
            )
        } else {
            reason
        };
        self.busy_join_retries = 0;
        let error = join_rejected(reason);
        self.snapshot.last_error = Some(error.clone());
        self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
        self.snapshot.selected_session = None;
        self.snapshot.recoverable_action = Some(RecoverableAction::Rescan);
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Error(error)],
            changed: true,
            stop_requested: false,
        })
    }

    /// Connects again after a busy refusal, if the selected session is still
    /// known: the platform waits `retry.delay_ms`, reuses the endpoint it
    /// resolved and resends the join. `None` when there is nothing to retry.
    fn retry_busy_join(
        &mut self,
        retry: crate::runtime::BusyJoinRetry,
    ) -> Result<Option<ApplyOutcome>, CoreError> {
        let Some(selected_id) = self.snapshot.selected_session.clone() else {
            return Ok(None);
        };
        let Some(endpoint) = self
            .snapshot
            .discovered_sessions
            .iter()
            .find(|session| session.session_id == selected_id)
            .map(|session| session.endpoint)
        else {
            return Ok(None);
        };
        let effect = self.start_platform_operation(
            crate::runtime::PlatformEffectRequest::EstablishNetwork(
                crate::runtime::NetworkEstablishmentRequest {
                    session_id: selected_id.clone(),
                    endpoint,
                    busy_retry: Some(retry),
                },
            ),
            super::PendingPlatformOperation::EstablishNetwork {
                session_id: selected_id,
            },
        )?;
        self.busy_join_retries = retry.attempt;
        self.snapshot.listener_lifecycle = ListenerLifecycle::JoinRequested;
        self.snapshot.transport_state = crate::domain::TransportState::Retrying;
        let mut outcome = ApplyOutcome::effect(effect);
        let diagnostic = self.diagnostic(
            "listener.join_busy_retry",
            vec![
                Self::field("attempt", &retry.attempt.to_string())?,
                Self::field("delay_ms", &retry.delay_ms.to_string())?,
            ],
        )?;
        outcome.notifications.extend(diagnostic.notifications);
        Ok(Some(outcome))
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
        // While a busy retry waits to reconnect, the only connection that can
        // fail is the one the host just refused and closed. Its close must not
        // end the retry; the reconnect's own failures arrive after it is ready.
        if self.snapshot.selected_role == Some(AppRole::Listener)
            && self.snapshot.listener_lifecycle == ListenerLifecycle::JoinRequested
            && self.snapshot.transport_state == TransportState::Retrying
            && self.busy_join_retries > 0
        {
            return self.diagnostic(
                "listener.busy_refused_connection_closed",
                vec![Self::field("message", &error.message)?],
            );
        }
        self.snapshot.last_error = Some(error.clone());
        self.snapshot.transport_state = TransportState::Failed;
        // Forcing transport_state to Failed unconditionally means
        // discovery_active must be cleared too, or CoreSnapshot::validate()
        // rejects the whole candidate on its discovery-state-mismatch
        // invariant (discovery_active must agree with
        // transport_state == Discovering).
        self.snapshot.discovery_active = false;
        match self.snapshot.selected_role {
            Some(AppRole::Host) => {
                self.snapshot.host_lifecycle = HostLifecycle::Error;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Retry);
            }
            Some(AppRole::Listener)
                if LISTENER_ACTIVE_LIFECYCLES.contains(&self.snapshot.listener_lifecycle) =>
            {
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.recoverable_action =
                    error.retryable.then_some(RecoverableAction::Reconnect);
            }
            Some(AppRole::Listener) | None => {}
        }
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Error(error)],
            changed: true,
            stop_requested: false,
        })
    }
}
