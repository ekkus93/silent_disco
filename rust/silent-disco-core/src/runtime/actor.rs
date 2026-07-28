use super::{
    AudioEvent, CommandReceipt, CoreActorInput, CoreCommand, CoreCommandRequest, CoreDiagnostic,
    CoreNotification, CoreSnapshot, DiagnosticField, DiscoveryRequest, HostDraftPatch,
    NetworkEstablishmentRequest, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion, RecoverableAction, RuntimeContractError, SessionAdvertisement,
    StorageCompletion, StorageEvent, TransportEvent, current_protocol_version,
    MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS, MAX_PENDING_JOIN_REQUESTS,
};
use crate::domain::{
    AppRole, DeviceId, HostLifecycle, ListenerLifecycle, OperationId, PlaybackState, SessionId,
    TransportState,
};
use crate::error::{CoreError, CoreErrorCode, ErrorSeverity};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};

pub const DEFAULT_ACTOR_QUEUE_CAPACITY: usize = 64;
pub const DEFAULT_NOTIFICATION_QUEUE_CAPACITY: usize = 64;
pub const MAX_ACTOR_QUEUE_CAPACITY: usize = 4_096;
pub const MAX_NOTIFICATION_QUEUE_CAPACITY: usize = 4_096;
const MAX_PENDING_PLATFORM_OPERATIONS: usize = 128;

/// Consumer of serialized core notifications.
pub trait CoreObserver: Send + Sync + 'static {
    /// Handles one notification outside actor state ownership.
    ///
    /// # Errors
    ///
    /// Returning an error permanently fails notification delivery and becomes
    /// visible through [`CoreActorHandle::current_snapshot`].
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError>;
}

impl<F> CoreObserver for F
where
    F: Fn(CoreNotification) -> Result<(), CoreError> + Send + Sync + 'static,
{
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        self(notification)
    }
}

/// Configuration for one authoritative actor instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreActorConfig {
    pub local_device_id: DeviceId,
    pub actor_queue_capacity: usize,
    pub notification_queue_capacity: usize,
}

impl CoreActorConfig {
    #[must_use]
    pub fn new(local_device_id: DeviceId) -> Self {
        Self {
            local_device_id,
            actor_queue_capacity: DEFAULT_ACTOR_QUEUE_CAPACITY,
            notification_queue_capacity: DEFAULT_NOTIFICATION_QUEUE_CAPACITY,
        }
    }

    /// Validates bounded nonzero queue capacities.
    ///
    /// # Errors
    ///
    /// Returns a structured invalid-argument error for zero or excessive values.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.actor_queue_capacity == 0
            || self.actor_queue_capacity > MAX_ACTOR_QUEUE_CAPACITY
        {
            return Err(core_error(
                CoreErrorCode::InvalidArgument,
                format!(
                    "actor queue capacity must be between 1 and {MAX_ACTOR_QUEUE_CAPACITY}"
                ),
                ErrorSeverity::Error,
                false,
                None,
            ));
        }
        if self.notification_queue_capacity == 0
            || self.notification_queue_capacity > MAX_NOTIFICATION_QUEUE_CAPACITY
        {
            return Err(core_error(
                CoreErrorCode::InvalidArgument,
                format!(
                    "notification queue capacity must be between 1 and {MAX_NOTIFICATION_QUEUE_CAPACITY}"
                ),
                ErrorSeverity::Error,
                false,
                None,
            ));
        }
        Ok(())
    }
}

/// Cloneable command and event submission surface.
#[derive(Clone)]
pub struct CoreActorHandle {
    sender: SyncSender<ActorMessage>,
    shared: Arc<ActorShared>,
    notification_failure: Arc<Mutex<Option<CoreError>>>,
}

/// Lifecycle owner for the actor and notification workers.
#[must_use = "CoreActorRuntime must be shut down explicitly"]
pub struct CoreActorRuntime {
    handle: CoreActorHandle,
    actor_join: Option<JoinHandle<Result<(), CoreError>>>,
    notification_sender: SyncSender<NotificationMessage>,
    notification_join: Option<JoinHandle<()>>,
}

struct ActorShared {
    accepting: AtomicBool,
    snapshot: RwLock<CoreSnapshot>,
    actor_failure: Mutex<Option<CoreError>>,
    next_command_sequence: AtomicU64,
}

#[derive(Debug)]
enum ActorMessage {
    Input(CoreActorInput),
}

#[derive(Debug)]
enum NotificationMessage {
    Notify(CoreNotification),
    Shutdown,
}

#[derive(Debug, Clone)]
struct ActorState {
    snapshot: CoreSnapshot,
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

impl CoreActorRuntime {
    /// Starts one serialized actor worker and one notification worker.
    ///
    /// # Errors
    ///
    /// Returns configuration or thread-start failure. A partially started
    /// notification worker is shut down and joined before returning.
    pub fn start<O: CoreObserver>(
        config: CoreActorConfig,
        observer: O,
    ) -> Result<Self, CoreError> {
        config.validate()?;

        let (notification_sender, notification_receiver) =
            sync_channel(config.notification_queue_capacity);
        let notification_failure = Arc::new(Mutex::new(None));
        let worker_notification_failure = Arc::clone(&notification_failure);
        let notification_join = thread::Builder::new()
            .name("silent-disco-notifications".to_owned())
            .spawn(move || {
                run_notification_worker(
                    observer,
                    notification_receiver,
                    &worker_notification_failure,
                );
            })
            .map_err(|error| {
                core_error(
                    CoreErrorCode::WorkerStopped,
                    format!("could not start notification worker: {error}"),
                    ErrorSeverity::Fatal,
                    false,
                    None,
                )
            })?;

        let (sender, receiver) = sync_channel(config.actor_queue_capacity);
        let shared = Arc::new(ActorShared {
            accepting: AtomicBool::new(true),
            snapshot: RwLock::new(CoreSnapshot::default()),
            actor_failure: Mutex::new(None),
            next_command_sequence: AtomicU64::new(1),
        });
        let actor_shared = Arc::clone(&shared);
        let actor_notification_sender = notification_sender.clone();
        let actor_notification_failure = Arc::clone(&notification_failure);
        let local_device_id = config.local_device_id;

        let actor_join = match thread::Builder::new()
            .name("silent-disco-actor".to_owned())
            .spawn(move || {
                run_actor_worker(
                    local_device_id,
                    receiver,
                    &actor_shared,
                    &actor_notification_sender,
                    &actor_notification_failure,
                )
            }) {
            Ok(join) => join,
            Err(error) => {
                let primary = core_error(
                    CoreErrorCode::WorkerStopped,
                    format!("could not start actor worker: {error}"),
                    ErrorSeverity::Fatal,
                    false,
                    None,
                );
                let send_result = notification_sender.send(NotificationMessage::Shutdown);
                let join_result = notification_join.join();
                if send_result.is_err() || join_result.is_err() {
                    return Err(core_error(
                        CoreErrorCode::ShutdownFailed,
                        format!(
                            "{primary}; notification-worker cleanup also failed"
                        ),
                        ErrorSeverity::Fatal,
                        false,
                        None,
                    ));
                }
                return Err(primary);
            }
        };

        let handle = CoreActorHandle {
            sender,
            shared,
            notification_failure,
        };
        Ok(Self {
            handle,
            actor_join: Some(actor_join),
            notification_sender,
            notification_join: Some(notification_join),
        })
    }

    #[must_use]
    pub fn handle(&self) -> CoreActorHandle {
        self.handle.clone()
    }

    /// Rejects new public submissions, queues shutdown behind admitted work, joins
    /// the actor, and then drains and joins notification delivery.
    ///
    /// # Errors
    ///
    /// Returns a combined structured shutdown failure when any phase fails.
    pub fn shutdown(mut self) -> Result<(), CoreError> {
        let was_accepting = self.handle.shared.accepting.swap(false, Ordering::AcqRel);
        let queue_error = if was_accepting {
            self.queue_shutdown().err()
        } else {
            None
        };

        let actor_error = match self.actor_join.take() {
            Some(join) => match join.join() {
                Ok(result) => result.err(),
                Err(_) => Some(core_error(
                    CoreErrorCode::ShutdownFailed,
                    "actor worker panicked during shutdown",
                    ErrorSeverity::Fatal,
                    false,
                    None,
                )),
            },
            None => Some(core_error(
                CoreErrorCode::ShutdownFailed,
                "actor worker was already joined",
                ErrorSeverity::Fatal,
                false,
                None,
            )),
        };

        let notification_send_error = self
            .notification_sender
            .send(NotificationMessage::Shutdown)
            .err();
        let notification_join_error = match self.notification_join.take() {
            Some(join) => join.join().err(),
            None => Some(Box::new("notification worker already joined") as Box<dyn std::any::Any + Send>),
        };
        let notification_failure = lock_failure(&self.handle.notification_failure);

        if queue_error.is_none()
            && actor_error.is_none()
            && notification_send_error.is_none()
            && notification_join_error.is_none()
            && notification_failure.is_none()
        {
            return Ok(());
        }

        Err(core_error(
            CoreErrorCode::ShutdownFailed,
            format!(
                "controlled shutdown failed (queue={}, actor={}, notification_send={}, notification_join={}, observer={})",
                describe_error(queue_error.as_ref()),
                describe_error(actor_error.as_ref()),
                if notification_send_error.is_none() { "ok" } else { "failed" },
                if notification_join_error.is_none() { "ok" } else { "failed" },
                describe_error(notification_failure.as_ref()),
            ),
            ErrorSeverity::Fatal,
            false,
            None,
        ))
    }

    fn queue_shutdown(&self) -> Result<(), CoreError> {
        let operation_id = self.handle.next_command_id()?;
        let revision = self.handle.read_snapshot()?.revision;
        let request = CoreCommandRequest::new(revision, CoreCommand::Shutdown)
            .map_err(|error| invalid_argument(error.to_string(), Some(operation_id.clone())))?;
        self.handle
            .sender
            .send(ActorMessage::Input(CoreActorInput::Command {
                operation_id,
                request,
            }))
            .map_err(|_| worker_stopped(None))
    }
}

impl Drop for CoreActorRuntime {
    fn drop(&mut self) {
        if (self.actor_join.is_some() || self.notification_join.is_some())
            && !thread::panicking()
        {
            panic!("CoreActorRuntime dropped without shutdown");
        }
    }
}

impl CoreActorHandle {
    /// Queues one revision-aware command and returns queue-admission evidence.
    ///
    /// The receipt does not prove command completion.
    ///
    /// # Errors
    ///
    /// Returns observer/actor failure, shutdown, operation-ID exhaustion, or
    /// visible queue overflow without blocking.
    pub fn submit_command(
        &self,
        request: CoreCommandRequest,
    ) -> Result<CommandReceipt, CoreError> {
        self.ensure_accepting()?;
        request
            .command
            .validate_shape()
            .map_err(|error| invalid_argument(error.to_string(), None))?;
        let operation_id = self.next_command_id()?;
        let accepted_at_revision = self.current_snapshot()?.revision;
        self.try_send(
            CoreActorInput::Command {
                operation_id: operation_id.clone(),
                request,
            },
            Some(operation_id.clone()),
        )?;
        Ok(CommandReceipt {
            operation_id,
            accepted_at_revision,
        })
    }

    /// Queues a platform completion or spontaneous platform fact.
    ///
    /// # Errors
    ///
    /// Returns observer/actor failure, shutdown, or visible queue overflow.
    pub fn submit_platform_event(&self, event: PlatformEvent) -> Result<(), CoreError> {
        let operation_id = event.operation_id().cloned();
        self.submit_input(CoreActorInput::Platform(event), operation_id)
    }

    /// Queues a transport-runtime fact.
    ///
    /// # Errors
    ///
    /// Returns observer/actor failure, shutdown, or visible queue overflow.
    pub fn submit_transport_event(&self, event: TransportEvent) -> Result<(), CoreError> {
        let operation_id = match &event {
            TransportEvent::DeliveryCompleted { operation_id, .. } => Some(operation_id.clone()),
            _ => None,
        };
        self.submit_input(CoreActorInput::Transport(event), operation_id)
    }

    /// Queues a non-real-time audio fact.
    ///
    /// # Errors
    ///
    /// Returns observer/actor failure, shutdown, or visible queue overflow.
    pub fn submit_audio_event(&self, event: AudioEvent) -> Result<(), CoreError> {
        self.submit_input(CoreActorInput::Audio(event), None)
    }

    /// Queues a validated correlated storage fact.
    ///
    /// # Errors
    ///
    /// Returns payload validation, observer/actor failure, shutdown, or queue
    /// overflow.
    pub fn submit_storage_event(&self, event: StorageEvent) -> Result<(), CoreError> {
        event.validate().map_err(|error| {
            invalid_argument(error.to_string(), Some(event.operation_id().clone()))
        })?;
        let operation_id = Some(event.operation_id().clone());
        self.submit_input(CoreActorInput::Storage(event), operation_id)
    }

    /// Returns the latest immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns recorded observer/actor failure or poisoned shared-state failure.
    pub fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        self.check_failure()?;
        self.read_snapshot()
    }

    fn submit_input(
        &self,
        input: CoreActorInput,
        operation_id: Option<OperationId>,
    ) -> Result<(), CoreError> {
        self.ensure_accepting()?;
        self.try_send(input, operation_id)
    }

    fn try_send(
        &self,
        input: CoreActorInput,
        operation_id: Option<OperationId>,
    ) -> Result<(), CoreError> {
        match self.sender.try_send(ActorMessage::Input(input)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(core_error(
                CoreErrorCode::QueueOverflow,
                "authoritative actor queue is full",
                ErrorSeverity::Error,
                true,
                operation_id,
            )),
            Err(TrySendError::Disconnected(_)) => Err(worker_stopped(operation_id)),
        }
    }

    fn ensure_accepting(&self) -> Result<(), CoreError> {
        self.check_failure()?;
        if self.shared.accepting.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(core_error(
                CoreErrorCode::ShutdownInProgress,
                "authoritative actor shutdown is in progress",
                ErrorSeverity::Warning,
                false,
                None,
            ))
        }
    }

    fn check_failure(&self) -> Result<(), CoreError> {
        if let Some(error) = lock_failure(&self.notification_failure) {
            return Err(error);
        }
        match self.shared.actor_failure.lock() {
            Ok(guard) => guard.clone().map_or(Ok(()), Err),
            Err(_) => Err(shared_state_error("actor failure mutex was poisoned")),
        }
    }

    fn read_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        self.shared
            .snapshot
            .read()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| shared_state_error("actor snapshot cache was poisoned"))
    }

    fn next_command_id(&self) -> Result<OperationId, CoreError> {
        let sequence = self
            .shared
            .next_command_sequence
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| resource_limit("command operation identifier overflow", None))?;
        OperationId::new(format!("command-{sequence}"))
            .map_err(|error| CoreError::from_identifier_validation(&error, None))
    }
}

impl ActorState {
    fn new(local_device_id: DeviceId) -> Self {
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

    fn process(&mut self, input: CoreActorInput) -> ProcessResult {
        let operation_id = input.operation_id().cloned();
        let mut candidate = self.clone();
        match candidate.apply(input) {
            Ok(mut outcome) => {
                if outcome.changed {
                    let revision = match candidate.snapshot.revision.checked_next() {
                        Ok(revision) => revision,
                        Err(_) => {
                            return ProcessResult {
                                notifications: vec![CoreNotification::Error(resource_limit(
                                    "snapshot revision overflow",
                                    operation_id,
                                ))],
                                stop_requested: true,
                            };
                        }
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
            CoreCommand::SelectRole { role } => {
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
            CoreCommand::UpdateHostDraft(patch) => {
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
                    .patched(&patch)
                    .map_err(|error| invalid_argument(error.to_string(), Some(operation_id)))?;
                self.clear_failure();
                Ok(ApplyOutcome::changed())
            }
            CoreCommand::CreateHostSession => {
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
            CoreCommand::EndHostSession => {
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
            CoreCommand::StartDiscovery => {
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
            CoreCommand::StopDiscovery => {
                self.require_role(AppRole::Listener, &operation_id)?;
                if !self.snapshot.discovery_active {
                    return Err(invalid_state(
                        "discovery is not active",
                        Some(operation_id),
                    ));
                }
                let effect = self.start_platform_operation(
                    PlatformEffectRequest::StopDiscovery,
                    PendingPlatformOperation::StopDiscovery,
                )?;
                Ok(ApplyOutcome::effect(effect))
            }
            CoreCommand::SelectSession { session_id } => {
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
            CoreCommand::SubmitJoin { .. } => {
                self.require_role(AppRole::Listener, &operation_id)?;
                if self.snapshot.listener_lifecycle != ListenerLifecycle::SessionSelected {
                    return Err(invalid_state(
                        "join requires a selected session",
                        Some(operation_id),
                    ));
                }
                let selected_id = self
                    .snapshot
                    .selected_session
                    .as_ref()
                    .ok_or_else(|| invalid_state("join has no selected session", Some(operation_id.clone())))?;
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
            CoreCommand::CancelJoin => {
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
            CoreCommand::ExportDiagnostics => {
                let export_id = self.next_export_id()?;
                let effect = self.start_platform_operation(
                    PlatformEffectRequest::ShareDiagnostics {
                        export_id: export_id.clone(),
                    },
                    PendingPlatformOperation::ShareDiagnostics { export_id },
                )?;
                Ok(ApplyOutcome::effect(effect))
            }
            CoreCommand::RetryRecoverableFailure => {
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

    fn apply_platform(&mut self, event: PlatformEvent) -> Result<ApplyOutcome, CoreError> {
        match event {
            PlatformEvent::OperationSucceeded {
                operation_id,
                completion,
            } => self.apply_platform_success(operation_id, completion),
            PlatformEvent::OperationFailed {
                operation_id,
                mut error,
            } => {
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
                self.apply_pending_failure(&pending, &error);
                self.snapshot.last_error = Some(error.clone());
                Ok(ApplyOutcome {
                    notifications: vec![CoreNotification::Error(error)],
                    changed: true,
                    stop_requested: false,
                })
            }
            PlatformEvent::SessionDiscovered(advertisement) => {
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
                        return Err(resource_limit(
                            "discovered-session capacity reached",
                            None,
                        ));
                    }
                    self.snapshot.discovered_sessions.push(advertisement);
                }
                Ok(ApplyOutcome::changed())
            }
            PlatformEvent::SessionExpired { session_id } => {
                let previous = self.snapshot.discovered_sessions.len();
                self.snapshot
                    .discovered_sessions
                    .retain(|session| session.session_id != session_id);
                if previous == self.snapshot.discovered_sessions.len() {
                    return Ok(ApplyOutcome::default());
                }
                if self.snapshot.selected_session.as_ref() == Some(&session_id) {
                    self.snapshot.selected_session = None;
                    self.snapshot.listener_lifecycle = if self.snapshot.discovery_active {
                        ListenerLifecycle::Scanning
                    } else {
                        ListenerLifecycle::Idle
                    };
                }
                Ok(ApplyOutcome::changed())
            }
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
            ) => {
                self.host_session_id = None;
                self.snapshot.host_lifecycle = HostLifecycle::Idle;
                self.snapshot.transport_state = TransportState::Idle;
                self.snapshot.pending_join_requests.clear();
                self.snapshot.listeners.clear();
                self.snapshot.playback_state = PlaybackState::Stopped;
                self.snapshot.playback_position_ms = 0;
                self.snapshot.last_delivery = None;
            }
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
            ) => {
                self.snapshot.discovery_active = false;
                self.snapshot.transport_state = TransportState::Idle;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Idle;
                self.snapshot.selected_session = None;
                self.snapshot.playback_state = PlaybackState::Stopped;
            }
            (
                PendingPlatformOperation::ShareDiagnostics { export_id: expected },
                PlatformOperationCompletion::DiagnosticsShared { export_id },
            ) if expected == &export_id => {
                outcome = self.diagnostic(
                    "diagnostics_shared",
                    vec![self.field("export_id", &export_id)?],
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
            TransportEvent::JoinRequested(request) => {
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
            TransportEvent::ListenerConnected(listener) => {
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
                        return Err(resource_limit(
                            "connected-listener capacity reached",
                            None,
                        ));
                    }
                    self.snapshot.listeners.push(listener);
                }
                if self.snapshot.host_lifecycle == HostLifecycle::WaitingForListeners {
                    self.snapshot.host_lifecycle = HostLifecycle::Ready;
                }
                Ok(ApplyOutcome::changed())
            }
            TransportEvent::ListenerDisconnected { device_id, error } => {
                let previous = self.snapshot.listeners.len();
                self.snapshot
                    .listeners
                    .retain(|listener| listener.device_id != device_id);
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
            TransportEvent::Failed(error) => {
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
        }
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
                    vec![self.field("missing_frames", &missing_frames.to_string())?],
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
                StorageCompletion::SettingsSaved => self.diagnostic("settings_saved", Vec::new()),
                StorageCompletion::TrustedDevicesLoaded(devices) => self.diagnostic(
                    "trusted_devices_loaded",
                    vec![self.field("count", &devices.len().to_string())?],
                ),
                StorageCompletion::TrustedDeviceUpdated { device_id } => self.diagnostic(
                    "trusted_device_updated",
                    vec![self.field("device_id", device_id.as_str())?],
                ),
                StorageCompletion::DiagnosticsExportReady { export_id } => self.diagnostic(
                    "diagnostics_export_ready",
                    vec![self.field("export_id", &export_id)?],
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

    fn apply_pending_failure(&mut self, pending: &PendingPlatformOperation, error: &CoreError) {
        match pending {
            PendingPlatformOperation::StartAdvertising { .. }
            | PendingPlatformOperation::StopAdvertising => {
                self.snapshot.host_lifecycle = HostLifecycle::Error;
                self.snapshot.transport_state = TransportState::Failed;
                self.snapshot.recoverable_action = error.retryable.then_some(RecoverableAction::Retry);
            }
            PendingPlatformOperation::StartDiscovery
            | PendingPlatformOperation::StopDiscovery => {
                self.snapshot.discovery_active = false;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.transport_state = TransportState::Failed;
                self.snapshot.recoverable_action = error.retryable.then_some(RecoverableAction::Rescan);
            }
            PendingPlatformOperation::EstablishNetwork { .. }
            | PendingPlatformOperation::ReleaseNetwork => {
                self.snapshot.discovery_active = false;
                self.snapshot.listener_lifecycle = ListenerLifecycle::Error;
                self.snapshot.transport_state = TransportState::Failed;
                self.snapshot.recoverable_action = error.retryable.then_some(RecoverableAction::Reconnect);
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
            .ok_or_else(|| resource_limit("session identifier counter overflow", None))?;
        SessionId::new(format!("session-{sequence}"))
            .map_err(|error| CoreError::from_identifier_validation(&error, None))
    }

    fn next_export_id(&mut self) -> Result<String, CoreError> {
        let sequence = self.next_export_sequence;
        self.next_export_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resource_limit("diagnostics export counter overflow", None))?;
        Ok(format!("diagnostics-{sequence}"))
    }

    fn require_role(&self, role: AppRole, operation_id: &OperationId) -> Result<(), CoreError> {
        if self.snapshot.selected_role == Some(role) {
            Ok(())
        } else {
            Err(invalid_state(
                format!("command requires the {} role", role.wire_name()),
                Some(operation_id.clone()),
            ))
        }
    }

    fn require_role_event(&self, role: AppRole) -> Result<(), CoreError> {
        if self.snapshot.selected_role == Some(role) {
            Ok(())
        } else {
            Err(invalid_state(
                format!("event requires the {} role", role.wire_name()),
                None,
            ))
        }
    }

    fn require_discovery_active(&self) -> Result<(), CoreError> {
        self.require_role_event(AppRole::Listener)?;
        if self.snapshot.discovery_active {
            Ok(())
        } else {
            Err(invalid_state(
                "session discovery fact arrived while discovery was inactive",
                None,
            ))
        }
    }

    fn clear_failure(&mut self) {
        self.snapshot.last_error = None;
        self.snapshot.recoverable_action = None;
    }

    fn field(&self, key: &str, value: &str) -> Result<DiagnosticField, CoreError> {
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

struct ProcessResult {
    notifications: Vec<CoreNotification>,
    stop_requested: bool,
}

fn run_actor_worker(
    local_device_id: DeviceId,
    receiver: Receiver<ActorMessage>,
    shared: &Arc<ActorShared>,
    notification_sender: &SyncSender<NotificationMessage>,
    notification_failure: &Arc<Mutex<Option<CoreError>>>,
) -> Result<(), CoreError> {
    let mut state = ActorState::new(local_device_id);
    send_notification(
        notification_sender,
        notification_failure,
        CoreNotification::Snapshot(state.snapshot.clone()),
    )?;

    while let Ok(message) = receiver.recv() {
        let ActorMessage::Input(input) = message;
        let result = state.process(input);
        for notification in result.notifications {
            if let CoreNotification::Snapshot(snapshot) = &notification {
                write_snapshot(shared, snapshot)?;
            }
            send_notification(notification_sender, notification_failure, notification)?;
        }
        if result.stop_requested {
            shared.accepting.store(false, Ordering::Release);
            return Ok(());
        }
    }

    shared.accepting.store(false, Ordering::Release);
    let error = worker_stopped(None);
    store_failure(&shared.actor_failure, error.clone());
    Err(error)
}

fn run_notification_worker<O: CoreObserver>(
    observer: O,
    receiver: Receiver<NotificationMessage>,
    failure: &Arc<Mutex<Option<CoreError>>>,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            NotificationMessage::Notify(notification) => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    observer.on_notification(notification)
                }));
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(observer_error)) => Some(core_error(
                        CoreErrorCode::FfiCallbackFailed,
                        format!(
                            "core observer returned {}: {}",
                            observer_error.code.numeric_code(),
                            observer_error.message
                        ),
                        ErrorSeverity::Fatal,
                        false,
                        None,
                    )),
                    Err(_) => Some(core_error(
                        CoreErrorCode::FfiCallbackFailed,
                        "core observer panicked",
                        ErrorSeverity::Fatal,
                        false,
                        None,
                    )),
                };
                if let Some(error) = error {
                    store_failure(failure, error);
                    break;
                }
            }
            NotificationMessage::Shutdown => break,
        }
    }
}

fn send_notification(
    sender: &SyncSender<NotificationMessage>,
    failure: &Arc<Mutex<Option<CoreError>>>,
    notification: CoreNotification,
) -> Result<(), CoreError> {
    if let Some(error) = lock_failure(failure) {
        return Err(error);
    }
    match sender.try_send(NotificationMessage::Notify(notification)) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            let error = core_error(
                CoreErrorCode::QueueOverflow,
                "core notification queue is full",
                ErrorSeverity::Fatal,
                false,
                None,
            );
            store_failure(failure, error.clone());
            Err(error)
        }
        Err(TrySendError::Disconnected(_)) => {
            let error = core_error(
                CoreErrorCode::FfiCallbackFailed,
                "core notification worker is disconnected",
                ErrorSeverity::Fatal,
                false,
                None,
            );
            store_failure(failure, error.clone());
            Err(error)
        }
    }
}

fn write_snapshot(shared: &ActorShared, snapshot: &CoreSnapshot) -> Result<(), CoreError> {
    let mut guard = shared
        .snapshot
        .write()
        .map_err(|_| shared_state_error("actor snapshot cache was poisoned"))?;
    *guard = snapshot.clone();
    Ok(())
}

fn store_failure(target: &Mutex<Option<CoreError>>, error: CoreError) {
    match target.lock() {
        Ok(mut guard) => {
            if guard.is_none() {
                *guard = Some(error);
            }
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = Some(shared_state_error("runtime failure mutex was poisoned"));
        }
    }
}

fn lock_failure(target: &Mutex<Option<CoreError>>) -> Option<CoreError> {
    match target.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => Some(shared_state_error("runtime failure mutex was poisoned")),
    }
}

fn core_error(
    code: CoreErrorCode,
    message: impl Into<String>,
    severity: ErrorSeverity,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    CoreError {
        code,
        message: message.into(),
        subsystem: code.subsystem(),
        severity,
        retryable,
        operation_id,
        context: Vec::new(),
    }
}

fn invalid_state(message: impl Into<String>, operation_id: Option<OperationId>) -> CoreError {
    core_error(
        CoreErrorCode::InvalidStateTransition,
        message,
        ErrorSeverity::Error,
        false,
        operation_id,
    )
}

fn invalid_argument(message: impl Into<String>, operation_id: Option<OperationId>) -> CoreError {
    core_error(
        CoreErrorCode::InvalidArgument,
        message,
        ErrorSeverity::Error,
        false,
        operation_id,
    )
}

fn resource_limit(message: impl Into<String>, operation_id: Option<OperationId>) -> CoreError {
    core_error(
        CoreErrorCode::ResourceLimitExceeded,
        message,
        ErrorSeverity::Error,
        true,
        operation_id,
    )
}

fn worker_stopped(operation_id: Option<OperationId>) -> CoreError {
    core_error(
        CoreErrorCode::WorkerStopped,
        "authoritative actor is not accepting work",
        ErrorSeverity::Error,
        false,
        operation_id,
    )
}

fn shared_state_error(message: impl Into<String>) -> CoreError {
    core_error(
        CoreErrorCode::WorkerStopped,
        message,
        ErrorSeverity::Fatal,
        false,
        None,
    )
}

fn describe_error(error: Option<&CoreError>) -> String {
    error.map_or_else(|| "ok".to_owned(), ToString::to_string)
}
