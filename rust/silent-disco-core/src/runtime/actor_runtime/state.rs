#![allow(
    clippy::too_many_lines,
    reason = "the serialized reducer keeps each source-ordered event family auditable in one place"
)]

use super::errors::{invalid_argument, invalid_state, resource_limit};
use crate::domain::{
    AppRole, DeviceId, HostLifecycle, ListenerLifecycle, OperationId, PlaybackState, SessionId,
    TransportState,
};
use crate::error::CoreError;
use crate::runtime::records::{
    AudioEvent, CoreActorInput, CoreCommand, CoreDiagnostic, CoreNotification, CoreSnapshot,
    DiagnosticField, DiscoveryRequest, NetworkEstablishmentRequest, PlatformEffect,
    PlatformEffectRequest, PlatformEvent, PlatformOperationCompletion, RecoverableAction,
    SessionAdvertisement, StorageCompletion, StorageEvent, TransportEvent,
    current_protocol_version, MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS,
    MAX_PENDING_JOIN_REQUESTS,
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
                    outcome.notifications.insert(
                        0,
                        CoreNotification::Snapshot(candidate.snapshot.clone()),
                    );
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

    fn apply_command(
        &mut self,
        operation_id: OperationId,
        command: CoreCommand,
    ) -> Result<ApplyOutcome, CoreError> {
        match command {
            CoreCommand::SelectRole { role } => self.select_role(operation_id, role),
            CoreCommand::UpdateHostDraft(patch) => {
                self.update_host_draft(operation_id, &patch)
            }
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

    fn select_role(
        &mut self,
        operation_id: OperationId,
        role: AppRole,
    ) -> Result<ApplyOutcome, CoreError> {
        if self.snapshot.host_lifecycle != HostLifecycle::Idle
            || self.snapshot.listener_lifecycle != ListenerLifecycle::Idle
            || !self.pending_platform.is_empty()
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

    fn update_host_draft(
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

    fn create_host_session(
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
            .map_err(|error| {
                invalid_argument(error.to_string(), Some(operation_id.clone()))
            })?;
        let session_id = self.next_session_id()?;
        let advertisement = SessionAdvertisement::new(
            session_id.clone(),
            self.local_device_id.clone(),
            self.snapshot.host_draft.session_name.clone(),
            self.snapshot.host_draft.approval_mode,
            current_protocol_version(),
            None,
        )
        .map_err(|error| invalid_argument(error.to_string(), Some(operation_id)))?;
        let effect = self.start_platform_operation(
            PlatformEffectRequest::StartAdvertising(advertisement),
            PendingPlatformOperation::StartAdvertising {
                session_id: session_id.clone(),
            },
        )?;
        self.host_session_id = Some(session_id);
        self.snapshot.host_lifecycle = HostLifecycle::CreatingSession;
        self.clear_failure();
        Ok(ApplyOutcome::effect(effect))
    }

    fn end_host_session(
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

    fn start_discovery(
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
            ListenerLifecycle::Idle
                | ListenerLifecycle::Disconnected
                | ListenerLifecycle::Error
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

    fn stop_discovery(
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

    fn select_session(
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

    fn submit_join(
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

    fn cancel_join(
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

    fn export_diagnostics(&mut self) -> Result<ApplyOutcome, CoreError> {
        let export_id = self.next_export_id()?;
        let effect = self.start_platform_operation(
            PlatformEffectRequest::ShareDiagnostics {
                export_id: export_id.clone(),
            },
            PendingPlatformOperation::ShareDiagnostics { export_id },
        )?;
        Ok(ApplyOutcome::effect(effect))
    }

    fn retry_recoverable(
        &mut self,
        operation_id: OperationId,
    ) -> Result<ApplyOutcome, CoreError> {
        let error = self.snapshot.last_error.as_ref().ok_or_else(|| {
            invalid_state(
                "there is no recoverable failure to retry",
                Some(operation_id.clone()),
            )
        })?;
        if !error.retryable || self.snapshot.recoverable_action.is_none() {
            return Err(invalid_state(
                "the current failure is not retryable",
                Some(operation_id),
            ));
        }
        self.clear_failure();
        Ok(ApplyOutcome::changed())
    }

    fn apply_platform(&mut self, event: PlatformEvent) -> Result<ApplyOutcome, CoreError> {
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
            PlatformEvent::AppEnteredForeground => {
                self.diagnostic("app_foreground", Vec::new())
            }
            PlatformEvent::AppEnteredBackground => {
                self.diagnostic("app_background", Vec::new())
            }
        }
    }

    fn apply_platform_failure(
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

    fn record_discovered_session(
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

    fn expire_session(&mut self, session_id: &SessionId) -> Result<ApplyOutcome, CoreError> {
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

    fn apply_platform_success(
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

    fn apply_transport(&mut self, event: TransportEvent) -> Result<ApplyOutcome, CoreError> {
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

    fn record_join_request(
        &mut self,
        request: crate::runtime::types::JoinRequestSummary,
    ) -> Result<ApplyOutcome, CoreError> {
        self.require_role_event(AppRole::Host)?;
        if !matches!(
            self.snapshot.host_lifecycle,
            HostLifecycle::WaitingForListeners
                | HostLifecycle::Ready
                | HostLifecycle::Streaming
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

    fn record_listener(
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

    fn record_listener_disconnect(
        &mut self,
        device_id: &DeviceId,
        error: Option<CoreError>,
    ) -> Result<ApplyOutcome, CoreError> {
        let previous = self.snapshot.listeners.len();
        self.snapshot
            .listeners
            .retain(|listener| &listener.device_id != device_id);
        if let Some(error) = error {
            self.snapshot.last_error = Some(error.clone());
            return Ok(ApplyOutcome {
                notifications: vec![CoreNotification::Error(error)],
                changed: true,
                stop_requested: false,
            });
        }
        if previous == self.snapshot.listeners.len() {
            return Ok(ApplyOutcome::default());
        }
        Ok(ApplyOutcome::changed())
    }

    fn record_transport_failure(
        &mut self,
        error: CoreError,
    ) -> Result<ApplyOutcome, CoreError> {
        self.snapshot.last_error = Some(error.clone());
        self.snapshot.transport_state = TransportState::Failed;
        match self.snapshot.selected_role {
            Some(AppRole::Host) => self.snapshot.host_lifecycle = HostLifecycle::Error,
            Some(AppRole::Listener) => {
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.recoverable_action = Some(RecoverableAction::Reconnect);
            }
            None => {}
        }
        Ok(ApplyOutcome {
            notifications: vec![CoreNotification::Error(error)],
            changed: true,
            stop_requested: false,
        })
    }

    fn apply_audio(&mut self, event: AudioEvent) -> Result<ApplyOutcome, CoreError> {
        match event {
            AudioEvent::PlaybackStateChanged(state) => {
                self.snapshot.playback_state = state;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::PositionAdvanced { position_ms, .. } => {
                if !matches!(
                    self.snapshot.playback_state,
                    PlaybackState::Playing | PlaybackState::Paused
                ) {
                    return Err(invalid_state(
                        "playback position advanced while playback was inactive",
                        None,
                    ));
                }
                if position_ms < self.snapshot.playback_position_ms {
                    return Err(invalid_state("playback position moved backward", None));
                }
                self.snapshot.playback_position_ms = position_ms;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::SynchronizationUpdated { device_id, summary } => {
                self.snapshot.synchronization = Some(summary);
                if let Some(listener) = self
                    .snapshot
                    .listeners
                    .iter_mut()
                    .find(|listener| listener.device_id == device_id)
                {
                    listener.synchronization = Some(summary);
                }
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::EndOfStream { .. } => {
                self.snapshot.playback_state = PlaybackState::Stopped;
                Ok(ApplyOutcome::changed())
            }
            AudioEvent::Underrun { missing_frames } => {
                self.snapshot.playback_state = PlaybackState::Underrun;
                let mut outcome = self.diagnostic(
                    "audio_underrun",
                    vec![Self::field(
                        "missing_frames",
                        &missing_frames.to_string(),
                    )?],
                )?;
                outcome.changed = true;
                Ok(outcome)
            }
            AudioEvent::Failed(error) => {
                self.snapshot.playback_state = PlaybackState::Error;
                self.snapshot.last_error = Some(error.clone());
                Ok(ApplyOutcome {
                    notifications: vec![CoreNotification::Error(error)],
                    changed: true,
                    stop_requested: false,
                })
            }
        }
    }

    fn apply_storage(&mut self, event: StorageEvent) -> Result<ApplyOutcome, CoreError> {
        event.validate().map_err(|error| {
            invalid_argument(error.to_string(), Some(event.operation_id().clone()))
        })?;
        match event {
            StorageEvent::OperationSucceeded { completion, .. } => match completion {
                StorageCompletion::SettingsLoaded(Some(settings)) => {
                    self.snapshot.tuning = settings.tuning;
                    Ok(ApplyOutcome::changed())
                }
                StorageCompletion::SettingsLoaded(None) => Ok(ApplyOutcome::default()),
                StorageCompletion::SettingsSaved => {
                    self.diagnostic("settings_saved", Vec::new())
                }
                StorageCompletion::TrustedDevicesLoaded(devices) => self.diagnostic(
                    "trusted_devices_loaded",
                    vec![Self::field("count", &devices.len().to_string())?],
                ),
                StorageCompletion::TrustedDeviceUpdated { device_id } => self.diagnostic(
                    "trusted_device_updated",
                    vec![Self::field("device_id", device_id.as_str())?],
                ),
                StorageCompletion::DiagnosticsExportReady { export_id } => self.diagnostic(
                    "diagnostics_export_ready",
                    vec![Self::field("export_id", &export_id)?],
                ),
            },
            StorageEvent::OperationFailed {
                operation_id,
                mut error,
            } => {
                if let Some(inner_id) = &error.operation_id
                    && inner_id != &operation_id
                {
                    return Err(invalid_state(
                        "storage failure operation ID does not match its wrapper",
                        Some(operation_id),
                    ));
                }
                error.operation_id = Some(operation_id);
                self.snapshot.last_error = Some(error.clone());
                Ok(ApplyOutcome {
                    notifications: vec![CoreNotification::Error(error)],
                    changed: true,
                    stop_requested: false,
                })
            }
        }
    }

    fn start_platform_operation(
        &mut self,
        request: PlatformEffectRequest,
        pending: PendingPlatformOperation,
    ) -> Result<PlatformEffect, CoreError> {
        if self.pending_platform.len() >= MAX_PENDING_PLATFORM_OPERATIONS {
            return Err(resource_limit(
                "pending platform-operation capacity reached",
                None,
            ));
        }
        let operation_id = self.next_effect_id()?;
        let effect = PlatformEffect::new(operation_id.clone(), request)
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        self.pending_platform.push((operation_id, pending));
        Ok(effect)
    }

    fn pending(&self, operation_id: &OperationId) -> Option<&PendingPlatformOperation> {
        self.pending_platform
            .iter()
            .find(|(candidate, _)| candidate == operation_id)
            .map(|(_, pending)| pending)
    }

    fn remove_pending(
        &mut self,
        operation_id: &OperationId,
    ) -> Option<PendingPlatformOperation> {
        let index = self
            .pending_platform
            .iter()
            .position(|(candidate, _)| candidate == operation_id)?;
        Some(self.pending_platform.remove(index).1)
    }

    fn apply_pending_failure(
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

    fn next_effect_id(&mut self) -> Result<OperationId, CoreError> {
        let sequence = self.next_effect_sequence;
        self.next_effect_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("effect operation identifier overflow", None))?;
        OperationId::new(format!("effect-{sequence}"))
            .map_err(|error| CoreError::from_identifier_validation(&error, None))
    }

    fn next_session_id(&mut self) -> Result<SessionId, CoreError> {
        let sequence = self.next_session_sequence;
        self.next_session_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("session identifier overflow", None))?;
        SessionId::new(format!("session-{sequence}"))
            .map_err(|error| CoreError::from_identifier_validation(&error, None))
    }

    fn next_export_id(&mut self) -> Result<String, CoreError> {
        let sequence = self.next_export_sequence;
        self.next_export_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("diagnostics export identifier overflow", None))?;
        Ok(format!("diagnostics-{sequence}"))
    }

    fn require_role(
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

    fn require_role_event(&self, role: AppRole) -> Result<(), CoreError> {
        if self.snapshot.selected_role == Some(role) {
            Ok(())
        } else {
            Err(invalid_state(
                format!("event requires the {role} role"),
                None,
            ))
        }
    }

    fn require_discovery_active(&self) -> Result<(), CoreError> {
        if self.snapshot.discovery_active {
            Ok(())
        } else {
            Err(invalid_state(
                "discovery result arrived while discovery was inactive",
                None,
            ))
        }
    }

    fn reset_host_session(&mut self) {
        self.host_session_id = None;
        self.snapshot.host_lifecycle = HostLifecycle::Idle;
        self.snapshot.transport_state = TransportState::Idle;
        self.snapshot.pending_join_requests.clear();
        self.snapshot.listeners.clear();
        self.snapshot.playback_state = PlaybackState::Stopped;
        self.snapshot.playback_position_ms = 0;
        self.snapshot.last_delivery = None;
    }

    fn reset_listener_connection(&mut self) {
        self.snapshot.discovery_active = false;
        self.snapshot.transport_state = TransportState::Idle;
        self.snapshot.listener_lifecycle = ListenerLifecycle::Idle;
        self.snapshot.selected_session = None;
        self.snapshot.playback_state = PlaybackState::Stopped;
    }

    fn clear_failure(&mut self) {
        self.snapshot.last_error = None;
        self.snapshot.recoverable_action = None;
    }

    fn field(key: &str, value: &str) -> Result<DiagnosticField, CoreError> {
        DiagnosticField::new(key, value)
            .map_err(|error| invalid_argument(error.to_string(), None))
    }

    fn diagnostic(
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
