use super::{
    ActorState, ApplyOutcome, CoreError, CoreNotification, HostLifecycle, ListenerLifecycle,
    MAX_DISCOVERED_SESSIONS, OperationId, PendingPlatformOperation, PlatformEvent,
    PlatformOperationCompletion, SessionAdvertisement, SessionId, TransportState, invalid_state,
    resource_limit,
};

/// Listener states in which a join is in progress or established, so the
/// selected session is owned by the connection rather than by discovery.
const LISTENER_JOINED_LIFECYCLES: &[ListenerLifecycle] = &[
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
        // Once joined, the connection -- not mDNS presence -- owns the
        // session (Block 52.3). An advertisement dropping out of a scan says
        // nothing about a live connection, and acting on it reset connected
        // listeners to Idle. A host that really left surfaces as a transport
        // failure instead. The entry stays listed because the snapshot
        // requires the selected session to be a discovered one.
        if self.snapshot.selected_session.as_ref() == Some(session_id)
            && LISTENER_JOINED_LIFECYCLES.contains(&self.snapshot.listener_lifecycle)
        {
            return self.diagnostic(
                "session_expiry_ignored_while_joined",
                vec![
                    Self::field("session_id", session_id.as_str())?,
                    Self::field(
                        "listener_lifecycle",
                        &format!("{:?}", self.snapshot.listener_lifecycle),
                    )?,
                ],
            );
        }
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

#[cfg(test)]
mod session_expiry_tests {
    use super::{ActorState, ListenerLifecycle, SessionAdvertisement, SessionId};
    use crate::domain::{AppRole, ApprovalMode, DeviceId};
    use crate::runtime::CoreNotification;

    fn advertisement(id: &str) -> SessionAdvertisement {
        SessionAdvertisement::new(
            SessionId::new(id).expect("valid session ID"),
            DeviceId::new("desktop-host").expect("valid device ID"),
            "Patio Mix",
            ApprovalMode::Manual,
            2,
            None,
        )
        .expect("valid advertisement")
    }

    fn listener_with(selected: &str, lifecycle: ListenerLifecycle) -> ActorState {
        let mut state = ActorState::new(DeviceId::new("listener").expect("valid device ID"));
        state.snapshot.selected_role = Some(AppRole::Listener);
        state.snapshot.discovered_sessions = vec![advertisement(selected), advertisement("other")];
        state.snapshot.selected_session = Some(SessionId::new(selected).expect("valid ID"));
        state.snapshot.listener_lifecycle = lifecycle;
        state
    }

    /// Block 52.3: once a listener has joined, mDNS presence no longer decides
    /// whether the session exists -- the connection does. A host's
    /// advertisement dropping out of a scan (Android keeps reconciling after
    /// discovery stops) used to clear the selection and reset a connected
    /// listener to Idle, leaving it on "No session selected -- Unknown host".
    #[test]
    fn expiry_of_the_joined_session_is_ignored_in_every_joined_state() {
        for lifecycle in [
            ListenerLifecycle::JoinRequested,
            ListenerLifecycle::Connecting,
            ListenerLifecycle::AwaitingApproval,
            ListenerLifecycle::Approved,
            ListenerLifecycle::SyncingClock,
            ListenerLifecycle::Buffering,
            ListenerLifecycle::Playing,
            ListenerLifecycle::Reconnecting,
            ListenerLifecycle::Desynced,
        ] {
            let mut state = listener_with("joined", lifecycle);
            let outcome = state
                .expire_session(&SessionId::new("joined").expect("valid ID"))
                .expect("expiry of a joined session is not an error");

            assert_eq!(
                state.snapshot.listener_lifecycle, lifecycle,
                "{lifecycle:?}"
            );
            assert_eq!(
                state.snapshot.selected_session,
                Some(SessionId::new("joined").expect("valid ID")),
                "{lifecycle:?}"
            );
            // Kept in the list: the snapshot requires the selected session to
            // be a discovered one, and the UI resolves its name from there.
            assert!(
                state
                    .snapshot
                    .discovered_sessions
                    .iter()
                    .any(|session| session.session_id.as_str() == "joined"),
                "{lifecycle:?}"
            );
            assert!(!outcome.changed, "{lifecycle:?}");
            assert!(
                outcome
                    .notifications
                    .iter()
                    .any(|notification| matches!(notification, CoreNotification::Diagnostic(_))),
                "an ignored expiry must stay observable ({lifecycle:?})"
            );
        }
    }

    /// Before a join, a vanished advertisement really does make the selection
    /// stale, so it is still cleared.
    #[test]
    fn expiry_of_a_selected_but_unjoined_session_clears_the_selection() {
        let mut state = listener_with("picked", ListenerLifecycle::SessionSelected);
        state
            .expire_session(&SessionId::new("picked").expect("valid ID"))
            .expect("expiry succeeds");

        assert_eq!(state.snapshot.selected_session, None);
        assert_eq!(state.snapshot.listener_lifecycle, ListenerLifecycle::Idle);
        assert!(
            !state
                .snapshot
                .discovered_sessions
                .iter()
                .any(|session| session.session_id.as_str() == "picked")
        );
    }

    #[test]
    fn expiry_of_another_session_still_removes_it_while_joined() {
        let mut state = listener_with("joined", ListenerLifecycle::Playing);
        state
            .expire_session(&SessionId::new("other").expect("valid ID"))
            .expect("expiry succeeds");

        assert_eq!(state.snapshot.discovered_sessions.len(), 1);
        assert_eq!(
            state.snapshot.listener_lifecycle,
            ListenerLifecycle::Playing
        );
    }
}
