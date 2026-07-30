mod errors;
mod state;

use self::errors::{
    core_error, invalid_argument, resource_limit, shared_state_error, worker_stopped,
};
use self::state::ActorState;
use super::records::{
    AudioEvent, CommandReceipt, CoreActorInput, CoreCommand, CoreCommandRequest, CoreNotification,
    CoreSnapshot, PlatformEvent, StorageEvent, TransportEvent,
};
use crate::domain::{DeviceId, OperationId};
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
        if !(1..=MAX_ACTOR_QUEUE_CAPACITY).contains(&self.actor_queue_capacity) {
            return Err(core_error(
                CoreErrorCode::InvalidArgument,
                format!("actor queue capacity must be between 1 and {MAX_ACTOR_QUEUE_CAPACITY}"),
                ErrorSeverity::Error,
                false,
                None,
            ));
        }
        if !(1..=MAX_NOTIFICATION_QUEUE_CAPACITY).contains(&self.notification_queue_capacity) {
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

struct ActorMessage(Box<CoreActorInput>);

enum NotificationMessage {
    Notify(Box<CoreNotification>),
    Shutdown,
}

impl CoreActorRuntime {
    /// Starts one serialized actor worker and one notification worker.
    ///
    /// # Errors
    ///
    /// Returns configuration or thread-start failure. A partially started
    /// notification worker is shut down and joined before returning.
    pub fn start<O: CoreObserver>(config: CoreActorConfig, observer: O) -> Result<Self, CoreError> {
        config.validate()?;
        let (notification_sender, notification_receiver) =
            sync_channel(config.notification_queue_capacity);
        let notification_failure = Arc::new(Mutex::new(None));
        let worker_notification_failure = Arc::clone(&notification_failure);
        let notification_join = thread::Builder::new()
            .name("silent-disco-notifications".to_owned())
            .spawn(move || {
                run_notification_worker(
                    &observer,
                    &notification_receiver,
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
        let actor_join = thread::Builder::new()
            .name("silent-disco-actor".to_owned())
            .spawn(move || {
                run_actor_worker(
                    config.local_device_id,
                    &receiver,
                    &actor_shared,
                    &actor_notification_sender,
                )
            });

        let actor_join = match actor_join {
            Ok(join) => join,
            Err(error) => {
                let primary = core_error(
                    CoreErrorCode::WorkerStopped,
                    format!("could not start actor worker: {error}"),
                    ErrorSeverity::Fatal,
                    false,
                    None,
                );
                let send_failed = notification_sender
                    .send(NotificationMessage::Shutdown)
                    .is_err();
                let join_failed = notification_join.join().is_err();
                if send_failed || join_failed {
                    return Err(core_error(
                        CoreErrorCode::ShutdownFailed,
                        format!("{primary}; notification-worker startup cleanup also failed"),
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

    /// Rejects new public submissions, queues shutdown behind admitted work,
    /// joins the actor, and then joins notification delivery.
    ///
    /// # Errors
    ///
    /// Returns a combined structured shutdown failure when any phase fails.
    pub fn shutdown(mut self) -> Result<(), CoreError> {
        let was_accepting = self.handle.shared.accepting.swap(false, Ordering::AcqRel);
        let queue_error = was_accepting.then(|| self.queue_shutdown().err()).flatten();
        let actor_error = join_actor(self.actor_join.take());
        let notification_send_failed = self
            .notification_sender
            .send(NotificationMessage::Shutdown)
            .is_err();
        let notification_join_failed = self
            .notification_join
            .take()
            .is_none_or(|join| join.join().is_err());
        let observer_error = read_failure(
            &self.handle.notification_failure,
            "notification failure mutex was poisoned",
        )
        .err()
        .or_else(|| {
            read_failure(
                &self.handle.notification_failure,
                "notification failure mutex was poisoned",
            )
            .ok()
            .flatten()
        });

        if queue_error.is_none()
            && actor_error.is_none()
            && !notification_send_failed
            && !notification_join_failed
            && observer_error.is_none()
        {
            return Ok(());
        }
        Err(core_error(
            CoreErrorCode::ShutdownFailed,
            format!(
                "controlled shutdown failed (queue={}, actor={}, notification_send={}, notification_join={}, observer={})",
                describe_error(queue_error.as_ref()),
                describe_error(actor_error.as_ref()),
                status_name(notification_send_failed),
                status_name(notification_join_failed),
                describe_error(observer_error.as_ref()),
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
            .send(ActorMessage(Box::new(CoreActorInput::Command {
                operation_id,
                request,
            })))
            .map_err(|_| worker_stopped(None))
    }
}

impl Drop for CoreActorRuntime {
    fn drop(&mut self) {
        let clean = self.actor_join.is_none() && self.notification_join.is_none();
        assert!(
            clean || thread::panicking(),
            "CoreActorRuntime dropped without shutdown"
        );
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
    pub fn submit_command(&self, request: CoreCommandRequest) -> Result<CommandReceipt, CoreError> {
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
        match self.sender.try_send(ActorMessage(Box::new(input))) {
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
        if let Some(error) = read_failure(
            &self.notification_failure,
            "notification failure mutex was poisoned",
        )? {
            return Err(error);
        }
        if let Some(error) = read_failure(
            &self.shared.actor_failure,
            "actor failure mutex was poisoned",
        )? {
            return Err(error);
        }
        Ok(())
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

fn run_actor_worker(
    local_device_id: DeviceId,
    receiver: &Receiver<ActorMessage>,
    shared: &ActorShared,
    notification_sender: &SyncSender<NotificationMessage>,
) -> Result<(), CoreError> {
    let mut state = ActorState::new(local_device_id);
    send_notification(
        notification_sender,
        CoreNotification::Snapshot(state.snapshot.clone()),
    )?;
    while let Ok(ActorMessage(input)) = receiver.recv() {
        let result = state.process(*input);
        write_snapshot(shared, &state.snapshot)?;
        for notification in result.notifications {
            if let Err(error) = send_notification(notification_sender, notification) {
                record_failure(&shared.actor_failure, error.clone());
                shared.accepting.store(false, Ordering::Release);
                return Err(error);
            }
        }
        if result.stop_requested {
            shared.accepting.store(false, Ordering::Release);
            return Ok(());
        }
    }
    let error = worker_stopped(None);
    record_failure(&shared.actor_failure, error.clone());
    shared.accepting.store(false, Ordering::Release);
    Err(error)
}

fn run_notification_worker<O: CoreObserver>(
    observer: &O,
    receiver: &Receiver<NotificationMessage>,
    failure: &Mutex<Option<CoreError>>,
) {
    while let Ok(message) = receiver.recv() {
        match message {
            NotificationMessage::Notify(notification) => {
                let result =
                    catch_unwind(AssertUnwindSafe(|| observer.on_notification(*notification)));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        record_failure(failure, error);
                        return;
                    }
                    Err(_) => {
                        record_failure(
                            failure,
                            core_error(
                                CoreErrorCode::FfiCallbackFailed,
                                "notification observer panicked",
                                ErrorSeverity::Fatal,
                                false,
                                None,
                            ),
                        );
                        return;
                    }
                }
            }
            NotificationMessage::Shutdown => return,
        }
    }
    record_failure(failure, worker_stopped(None));
}

fn send_notification(
    sender: &SyncSender<NotificationMessage>,
    notification: CoreNotification,
) -> Result<(), CoreError> {
    match sender.try_send(NotificationMessage::Notify(Box::new(notification))) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(core_error(
            CoreErrorCode::QueueOverflow,
            "notification queue is full",
            ErrorSeverity::Fatal,
            false,
            None,
        )),
        Err(TrySendError::Disconnected(_)) => Err(worker_stopped(None)),
    }
}

fn write_snapshot(shared: &ActorShared, snapshot: &CoreSnapshot) -> Result<(), CoreError> {
    shared
        .snapshot
        .write()
        .map(|mut current| current.clone_from(snapshot))
        .map_err(|_| shared_state_error("actor snapshot cache was poisoned"))
}

fn record_failure(target: &Mutex<Option<CoreError>>, error: CoreError) {
    if let Ok(mut current) = target.lock()
        && current.is_none()
    {
        *current = Some(error);
    }
}

fn read_failure(
    target: &Mutex<Option<CoreError>>,
    poison_message: &str,
) -> Result<Option<CoreError>, CoreError> {
    target
        .lock()
        .map(|current| current.clone())
        .map_err(|_| shared_state_error(poison_message))
}

fn join_actor(join: Option<JoinHandle<Result<(), CoreError>>>) -> Option<CoreError> {
    let Some(join) = join else {
        return Some(core_error(
            CoreErrorCode::ShutdownFailed,
            "actor worker was already joined",
            ErrorSeverity::Fatal,
            false,
            None,
        ));
    };
    match join.join() {
        Ok(result) => result.err(),
        Err(_) => Some(core_error(
            CoreErrorCode::ShutdownFailed,
            "actor worker panicked during shutdown",
            ErrorSeverity::Fatal,
            false,
            None,
        )),
    }
}

fn describe_error(error: Option<&CoreError>) -> &'static str {
    if error.is_some() { "failed" } else { "ok" }
}

const fn status_name(failed: bool) -> &'static str {
    if failed { "failed" } else { "ok" }
}

#[cfg(test)]
mod tests {
    use super::{CoreActorConfig, CoreActorRuntime};
    use crate::domain::{AppRole, DeviceId};
    use crate::error::CoreErrorCode;
    use crate::runtime::{CoreCommand, CoreCommandRequest, SnapshotRevision};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn rejects_zero_queue_capacity() {
        let mut config = CoreActorConfig::new(DeviceId::new("core-1").expect("valid device ID"));
        config.actor_queue_capacity = 0;
        let error = config.validate().expect_err("zero capacity must fail");
        assert_eq!(error.code, CoreErrorCode::InvalidArgument);
    }

    #[test]
    fn observer_failure_is_visible_and_shutdown_is_explicit() {
        let runtime = CoreActorRuntime::start(
            CoreActorConfig::new(DeviceId::new("core-2").expect("valid device ID")),
            |_| {
                Err(super::core_error(
                    CoreErrorCode::FfiCallbackFailed,
                    "observer rejected notification",
                    crate::error::ErrorSeverity::Fatal,
                    false,
                    None,
                ))
            },
        )
        .expect("runtime starts before observer failure is reported");
        let handle = runtime.handle();
        let shutdown_error = runtime
            .shutdown()
            .expect_err("observer failure must make controlled shutdown fail visibly");
        assert_eq!(shutdown_error.code, CoreErrorCode::ShutdownFailed);
        let observer_error = handle
            .current_snapshot()
            .expect_err("joined observer failure must remain visible through the handle");
        assert_eq!(observer_error.code, CoreErrorCode::FfiCallbackFailed);
    }

    #[test]
    fn notification_queue_overflow_fails_actor_without_blocking() {
        let release_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let observer_gate = Arc::clone(&release_gate);
        let observer_calls = Arc::new(AtomicUsize::new(0));
        let observer_call_count = Arc::clone(&observer_calls);
        let (entered_sender, entered_receiver) = mpsc::channel();

        let mut config = CoreActorConfig::new(DeviceId::new("core-3").expect("valid device ID"));
        config.actor_queue_capacity = 8;
        config.notification_queue_capacity = 1;

        let runtime = CoreActorRuntime::start(config, move |_| {
            if observer_call_count.fetch_add(1, Ordering::SeqCst) == 0 {
                entered_sender
                    .send(())
                    .map_err(|_| super::worker_stopped(None))?;
                let (released, wake) = &*observer_gate;
                let mut released = released
                    .lock()
                    .map_err(|_| super::shared_state_error("test gate was poisoned"))?;
                while !*released {
                    released = wake
                        .wait(released)
                        .map_err(|_| super::shared_state_error("test gate was poisoned"))?;
                }
            }
            Ok(())
        })
        .expect("runtime starts");
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("initial notification entered observer");

        let handle = runtime.handle();
        handle
            .submit_command(
                CoreCommandRequest::new(
                    SnapshotRevision::new(0),
                    CoreCommand::SelectRole {
                        role: AppRole::Host,
                    },
                )
                .expect("valid first command"),
            )
            .expect("first command admitted");

        for _ in 0..1_000 {
            if handle
                .current_snapshot()
                .is_ok_and(|snapshot| snapshot.revision == SnapshotRevision::new(1))
            {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(
            handle
                .current_snapshot()
                .expect("first command snapshot is available")
                .revision,
            SnapshotRevision::new(1)
        );

        handle
            .submit_command(
                CoreCommandRequest::new(
                    SnapshotRevision::new(1),
                    CoreCommand::SelectRole {
                        role: AppRole::Listener,
                    },
                )
                .expect("valid second command"),
            )
            .expect("second command admitted");

        let overflow = (0..1_000).find_map(|_| match handle.current_snapshot() {
            Ok(_) => {
                std::thread::yield_now();
                None
            }
            Err(error) => Some(error),
        });
        let overflow = overflow.expect("notification overflow becomes visible");
        assert_eq!(overflow.code, CoreErrorCode::QueueOverflow);
        assert_eq!(overflow.message, "notification queue is full");

        let (released, wake) = &*release_gate;
        *released.lock().expect("release gate lock") = true;
        wake.notify_all();

        assert!(runtime.shutdown().is_err());
        assert!(observer_calls.load(Ordering::SeqCst) >= 1);
    }
}
