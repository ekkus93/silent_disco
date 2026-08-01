use super::audio_device;
use super::capabilities::{desktop_capabilities, publish_desktop_capabilities};
use super::diagnostics_export;
use super::discovery;
use super::failure::{DesktopPlatformFailure, core_error};
use super::network::DesktopHostNetworkControl;
use super::paths::DesktopProfilePaths;
use super::storage_effect_runner::DesktopStorageEffectDispatcher;
use crate::notification_buffer::DesktopNotificationBuffer;
use silent_disco_core::domain::OperationId;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity, MAX_ERROR_MESSAGE_BYTES};
use silent_disco_core::runtime::{
    CapabilitySnapshot, CoreActorHandle, CoreNotification, CoreObserver, CoreSnapshot,
    PlatformEffect, PlatformEffectRequest, PlatformEvent, PlatformOperationCompletion,
};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PLATFORM_EFFECT_QUEUE_CAPACITY: usize = 32;
const MAX_CANCELLED_OPERATION_IDS: usize = 128;
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub(crate) struct DesktopPlatformEffectDispatcher {
    sender: SyncSender<RunnerMessage>,
    accepting: Arc<AtomicBool>,
    lifecycle_gate: Arc<Mutex<()>>,
    cancelled: Arc<Mutex<HashSet<OperationId>>>,
}

pub(crate) struct DesktopPlatformEffectInbox {
    receiver: Receiver<RunnerMessage>,
    accepting: Arc<AtomicBool>,
    cancelled: Arc<Mutex<HashSet<OperationId>>>,
}

enum RunnerMessage {
    Effect(PlatformEffect),
    Shutdown,
}

impl DesktopPlatformEffectDispatcher {
    /// Creates the bounded effect queue captured by the core observer and consumed by the
    /// desktop platform worker.
    #[must_use]
    pub(crate) fn channel() -> (Self, DesktopPlatformEffectInbox) {
        Self::channel_with_capacity(PLATFORM_EFFECT_QUEUE_CAPACITY)
    }

    fn channel_with_capacity(capacity: usize) -> (Self, DesktopPlatformEffectInbox) {
        let (sender, receiver) = sync_channel(capacity);
        let accepting = Arc::new(AtomicBool::new(true));
        let cancelled = Arc::new(Mutex::new(HashSet::new()));
        let dispatcher = Self {
            sender,
            accepting: Arc::clone(&accepting),
            lifecycle_gate: Arc::new(Mutex::new(())),
            cancelled: Arc::clone(&cancelled),
        };
        let inbox = DesktopPlatformEffectInbox {
            receiver,
            accepting,
            cancelled,
        };
        (dispatcher, inbox)
    }

    /// Queues one correlated effect without blocking the core notification worker.
    ///
    /// # Errors
    ///
    /// Returns a visible queue, lifecycle, or synchronization failure. No effect is dropped
    /// or converted into a successful no-op.
    pub(crate) fn dispatch(&self, effect: PlatformEffect) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        let _gate = self.lifecycle_gate.lock().map_err(|_| {
            runner_error(
                CoreErrorCode::WorkerStopped,
                "desktop platform runner lifecycle lock was poisoned",
                false,
                Some(operation_id.clone()),
            )
        })?;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(runner_error(
                CoreErrorCode::ShutdownInProgress,
                "desktop platform runner is shutting down",
                true,
                Some(operation_id),
            ));
        }
        match self.sender.try_send(RunnerMessage::Effect(effect)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(runner_error(
                CoreErrorCode::QueueOverflow,
                "desktop platform effect queue is full",
                true,
                Some(operation_id),
            )),
            Err(TrySendError::Disconnected(_)) => Err(runner_error(
                CoreErrorCode::WorkerStopped,
                "desktop platform effect worker is unavailable",
                false,
                Some(operation_id),
            )),
        }
    }

    /// Marks an admitted operation for cancellation. The worker checks before and after
    /// adapter execution so a cancelled operation cannot publish a success completion.
    ///
    /// # Errors
    ///
    /// Returns a visible synchronization or bounded-capacity failure.
    #[allow(
        dead_code,
        reason = "the cancellation surface is required by Block 15 before a later command wires it"
    )]
    pub(crate) fn cancel(&self, operation_id: OperationId) -> Result<(), CoreError> {
        let mut cancelled = self.cancelled.lock().map_err(|_| {
            runner_error(
                CoreErrorCode::WorkerStopped,
                "desktop platform cancellation registry was poisoned",
                false,
                Some(operation_id.clone()),
            )
        })?;
        if cancelled.len() >= MAX_CANCELLED_OPERATION_IDS && !cancelled.contains(&operation_id) {
            return Err(runner_error(
                CoreErrorCode::ResourceLimitExceeded,
                "desktop platform cancellation registry is full",
                true,
                Some(operation_id),
            ));
        }
        cancelled.insert(operation_id);
        Ok(())
    }

    fn stop_accepting(&self) -> Result<(), CoreError> {
        let _gate = self.lifecycle_gate.lock().map_err(|_| {
            runner_error(
                CoreErrorCode::ShutdownFailed,
                "desktop platform runner lifecycle lock was poisoned during shutdown",
                false,
                None,
            )
        })?;
        self.accepting.store(false, Ordering::Release);
        match self.sender.try_send(RunnerMessage::Shutdown) {
            Ok(()) | Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => Ok(()),
        }
    }
}

/// Routes core platform effects away from the frontend notification bridge.
pub(crate) struct DesktopCoreObserver {
    notifications: Arc<DesktopNotificationBuffer>,
    platform_effects: DesktopPlatformEffectDispatcher,
    transport_effects: Arc<DesktopHostNetworkControl>,
    storage_effects: DesktopStorageEffectDispatcher,
}

impl DesktopCoreObserver {
    #[must_use]
    pub(crate) fn new(
        notifications: Arc<DesktopNotificationBuffer>,
        platform_effects: DesktopPlatformEffectDispatcher,
        transport_effects: Arc<DesktopHostNetworkControl>,
        storage_effects: DesktopStorageEffectDispatcher,
    ) -> Self {
        Self {
            notifications,
            platform_effects,
            transport_effects,
            storage_effects,
        }
    }
}

impl CoreObserver for DesktopCoreObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        match notification {
            CoreNotification::Effect(effect) => self.platform_effects.dispatch(effect),
            CoreNotification::TransportEffect(effect) => {
                self.transport_effects.dispatch_transport_effect(effect)
            }
            CoreNotification::StorageEffect(effect) => self.storage_effects.dispatch(effect),
            other => self.notifications.on_notification(other),
        }
    }
}

/// Owns and joins the single bounded desktop platform-effect worker.
#[must_use = "DesktopPlatformEffectRunner must be shut down explicitly"]
pub(crate) struct DesktopPlatformEffectRunner {
    dispatcher: DesktopPlatformEffectDispatcher,
    join: Option<JoinHandle<Result<(), CoreError>>>,
    network: Option<Arc<DesktopHostNetworkControl>>,
}

impl DesktopPlatformEffectRunner {
    /// Starts the production effect worker after the actor handle and profile paths exist.
    ///
    /// # Errors
    ///
    /// Returns a visible worker-start or capability-publication failure. A failed start joins any
    /// worker that was created before returning.
    pub(crate) fn start(
        inbox: DesktopPlatformEffectInbox,
        dispatcher: DesktopPlatformEffectDispatcher,
        handle: CoreActorHandle,
        paths: DesktopProfilePaths,
        network: Arc<DesktopHostNetworkControl>,
    ) -> Result<(Self, CoreSnapshot), CoreError> {
        let capability_handle = handle.clone();
        let mut runner = Self::start_with_components(
            inbox,
            dispatcher,
            Arc::new(handle),
            Arc::new(DesktopPlatformAdapters::new_with_network(
                paths,
                Arc::clone(&network),
                capability_handle.clone(),
            )),
        )?;
        runner.network = Some(network);
        let snapshot = match publish_desktop_capabilities(&capability_handle) {
            Ok(snapshot) => snapshot,
            Err(primary) => {
                return match runner.shutdown() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(append_startup_cleanup(primary, &cleanup)),
                };
            }
        };
        Ok((runner, snapshot))
    }

    pub(super) fn start_with_components(
        inbox: DesktopPlatformEffectInbox,
        dispatcher: DesktopPlatformEffectDispatcher,
        sink: Arc<dyn DesktopPlatformEventSink>,
        executor: Arc<dyn DesktopPlatformEffectExecutor>,
    ) -> Result<Self, CoreError> {
        let join = thread::Builder::new()
            .name("silent-disco-desktop-platform-effects".to_owned())
            .spawn(move || run_worker(&inbox, sink.as_ref(), executor.as_ref()))
            .map_err(|_| {
                runner_error(
                    CoreErrorCode::WorkerStopped,
                    "desktop platform effect worker could not be started",
                    false,
                    None,
                )
            })?;
        Ok(Self {
            dispatcher,
            join: Some(join),
            network: None,
        })
    }

    /// Cancels one correlated platform operation.
    ///
    /// # Errors
    ///
    /// Returns a visible bounded cancellation-registry failure.
    #[allow(
        dead_code,
        reason = "the lifecycle owner exposes cancellation before later commands wire it"
    )]
    pub(crate) fn cancel(&self, operation_id: OperationId) -> Result<(), CoreError> {
        self.dispatcher.cancel(operation_id)
    }

    /// Stops admission, cancels queued effects, and joins the owned worker.
    ///
    /// # Errors
    ///
    /// Returns the first lifecycle, worker, adapter, or core-submission failure. The worker is
    /// always joined before this method returns.
    pub(crate) fn shutdown(mut self) -> Result<(), CoreError> {
        let signal_error = self.dispatcher.stop_accepting().err();
        let worker_error = match self.join.take() {
            Some(join) => match join.join() {
                Ok(result) => result.err(),
                Err(_) => Some(runner_error(
                    CoreErrorCode::WorkerStopped,
                    "desktop platform effect worker panicked outside effect containment",
                    false,
                    None,
                )),
            },
            None => Some(runner_error(
                CoreErrorCode::ShutdownFailed,
                "desktop platform effect worker was already joined",
                false,
                None,
            )),
        };
        let network_error = self
            .network
            .take()
            .and_then(|network| network.shutdown().err());
        combine_shutdown_errors(signal_error.or(worker_error), network_error)
    }
}

impl Drop for DesktopPlatformEffectRunner {
    fn drop(&mut self) {
        assert!(
            self.join.is_none() || thread::panicking(),
            "DesktopPlatformEffectRunner dropped without shutdown"
        );
    }
}

pub(super) trait DesktopPlatformEventSink: Send + Sync + 'static {
    fn submit_platform_event(&self, event: PlatformEvent) -> Result<(), CoreError>;
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError>;
}

impl DesktopPlatformEventSink for CoreActorHandle {
    fn submit_platform_event(&self, event: PlatformEvent) -> Result<(), CoreError> {
        CoreActorHandle::submit_platform_event(self, event)
    }

    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        CoreActorHandle::current_snapshot(self)
    }
}

pub(super) trait DesktopPlatformEffectExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        effect: &PlatformEffect,
        snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure>;
}

pub(super) struct DesktopPlatformAdapters {
    paths: DesktopProfilePaths,
    capabilities: CapabilitySnapshot,
    network: Arc<DesktopHostNetworkControl>,
    transport_events: Option<CoreActorHandle>,
}

impl DesktopPlatformAdapters {
    #[cfg(test)]
    pub(super) fn new(paths: DesktopProfilePaths) -> Self {
        Self {
            paths,
            capabilities: desktop_capabilities(),
            network: Arc::new(DesktopHostNetworkControl::production()),
            transport_events: None,
        }
    }

    pub(super) fn new_with_network(
        paths: DesktopProfilePaths,
        network: Arc<DesktopHostNetworkControl>,
        transport_events: CoreActorHandle,
    ) -> Self {
        Self {
            paths,
            capabilities: desktop_capabilities(),
            network,
            transport_events: Some(transport_events),
        }
    }
}

impl DesktopPlatformEffectExecutor for DesktopPlatformAdapters {
    fn execute(
        &self,
        effect: &PlatformEffect,
        snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        match &effect.request {
            PlatformEffectRequest::RequestCapabilities(_) => Ok(
                PlatformOperationCompletion::CapabilitiesResolved(self.capabilities),
            ),
            PlatformEffectRequest::StartAdvertising(advertisement) => self
                .network
                .start_host(
                    advertisement,
                    self.transport_events.clone().ok_or_else(|| {
                        DesktopPlatformFailure::new(
                            CoreErrorCode::WorkerStopped,
                            "desktop host transport event sink is unavailable",
                            ErrorSeverity::Error,
                            true,
                        )
                    })?,
                )
                .map(|_| PlatformOperationCompletion::AdvertisingStarted),
            PlatformEffectRequest::StopAdvertising => self
                .network
                .stop_host()
                .map(|()| PlatformOperationCompletion::AdvertisingStopped),
            PlatformEffectRequest::StartDiscovery(_)
            | PlatformEffectRequest::StopDiscovery
            | PlatformEffectRequest::EstablishNetwork(_)
            | PlatformEffectRequest::ReleaseNetwork => {
                Err(discovery::unsupported_effect(&effect.request))
            }
            PlatformEffectRequest::PrepareAudioSource(_)
            | PlatformEffectRequest::StartAudioOutput(_)
            | PlatformEffectRequest::StopAudioOutput => {
                Err(audio_device::unsupported_effect(&effect.request))
            }
            PlatformEffectRequest::ShareDiagnostics { export_id } => {
                let snapshot = snapshot.ok_or_else(|| {
                    DesktopPlatformFailure::new(
                        CoreErrorCode::PlatformOperationFailed,
                        "authoritative snapshot is unavailable for diagnostics export",
                        ErrorSeverity::Error,
                        true,
                    )
                })?;
                diagnostics_export::write_export(self.paths.diagnostics(), export_id, snapshot)?;
                Ok(PlatformOperationCompletion::DiagnosticsShared {
                    export_id: export_id.clone(),
                })
            }
        }
    }
}

fn combine_shutdown_errors(
    primary: Option<CoreError>,
    cleanup: Option<CoreError>,
) -> Result<(), CoreError> {
    match (primary, cleanup) {
        (None, None) => Ok(()),
        (Some(error), None) | (None, Some(error)) => Err(error),
        (Some(primary), Some(cleanup)) => {
            let message = format!(
                "{}; network transport cleanup also failed: {}",
                primary.message, cleanup.message
            );
            let fallback = primary.clone();
            Err(CoreError::new(
                primary.code,
                bounded_runner_message(&message),
                primary.severity,
                primary.retryable || cleanup.retryable,
                primary.operation_id.clone(),
            )
            .unwrap_or(fallback))
        }
    }
}

fn bounded_runner_message(message: &str) -> String {
    let mut output = String::new();
    for character in message.chars() {
        if output.len().saturating_add(character.len_utf8()) > MAX_ERROR_MESSAGE_BYTES {
            break;
        }
        output.push(character);
    }
    output
}

fn run_worker(
    inbox: &DesktopPlatformEffectInbox,
    sink: &dyn DesktopPlatformEventSink,
    executor: &dyn DesktopPlatformEffectExecutor,
) -> Result<(), CoreError> {
    loop {
        if !inbox.accepting.load(Ordering::Acquire) {
            return drain_cancelled(inbox, sink);
        }
        match inbox.receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(RunnerMessage::Effect(effect)) => process_effect(inbox, sink, executor, &effect)?,
            Ok(RunnerMessage::Shutdown) => return drain_cancelled(inbox, sink),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(runner_error(
                    CoreErrorCode::WorkerStopped,
                    "desktop platform effect queue disconnected unexpectedly",
                    false,
                    None,
                ));
            }
        }
    }
}

fn process_effect(
    inbox: &DesktopPlatformEffectInbox,
    sink: &dyn DesktopPlatformEventSink,
    executor: &dyn DesktopPlatformEffectExecutor,
    effect: &PlatformEffect,
) -> Result<(), CoreError> {
    let operation_id = effect.operation_id.clone();
    if take_cancellation(inbox, &operation_id)? {
        return sink.submit_platform_event(cancelled_event(operation_id));
    }

    let snapshot = if matches!(
        &effect.request,
        PlatformEffectRequest::ShareDiagnostics { .. }
    ) {
        match sink.current_snapshot() {
            Ok(snapshot) => Some(snapshot),
            Err(mut error) => {
                error.operation_id = Some(operation_id.clone());
                return sink.submit_platform_event(PlatformEvent::OperationFailed {
                    operation_id,
                    error,
                });
            }
        }
    } else {
        None
    };

    let execution = catch_unwind(AssertUnwindSafe(|| {
        executor.execute(effect, snapshot.as_ref())
    }));
    let event = if take_cancellation(inbox, &operation_id)? {
        cancelled_event(operation_id)
    } else {
        match execution {
            Ok(Ok(completion)) => PlatformEvent::OperationSucceeded {
                operation_id,
                completion,
            },
            Ok(Err(failure)) => PlatformEvent::OperationFailed {
                operation_id: operation_id.clone(),
                error: failure.into_core_error(operation_id),
            },
            Err(_) => PlatformEvent::OperationFailed {
                operation_id: operation_id.clone(),
                error: DesktopPlatformFailure::new(
                    CoreErrorCode::FfiPanicContained,
                    "desktop platform effect adapter panicked and was contained",
                    ErrorSeverity::Error,
                    false,
                )
                .into_core_error(operation_id),
            },
        }
    };
    sink.submit_platform_event(event)
}

fn drain_cancelled(
    inbox: &DesktopPlatformEffectInbox,
    sink: &dyn DesktopPlatformEventSink,
) -> Result<(), CoreError> {
    let mut first_error = None;
    loop {
        match inbox.receiver.try_recv() {
            Ok(RunnerMessage::Effect(effect)) => {
                if let Err(error) = sink.submit_platform_event(cancelled_event(effect.operation_id))
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
            Ok(RunnerMessage::Shutdown) => {}
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn take_cancellation(
    inbox: &DesktopPlatformEffectInbox,
    operation_id: &OperationId,
) -> Result<bool, CoreError> {
    inbox
        .cancelled
        .lock()
        .map(|mut cancelled| cancelled.remove(operation_id))
        .map_err(|_| {
            runner_error(
                CoreErrorCode::WorkerStopped,
                "desktop platform cancellation registry was poisoned",
                false,
                Some(operation_id.clone()),
            )
        })
}

fn cancelled_event(operation_id: OperationId) -> PlatformEvent {
    PlatformEvent::OperationFailed {
        operation_id: operation_id.clone(),
        error: DesktopPlatformFailure::new(
            CoreErrorCode::ShutdownInProgress,
            "desktop platform operation was cancelled",
            ErrorSeverity::Error,
            true,
        )
        .into_core_error(operation_id),
    }
}

fn append_startup_cleanup(primary: CoreError, cleanup: &CoreError) -> CoreError {
    let primary_detail = bounded_error_detail(&primary.message, 300);
    let cleanup_detail = bounded_error_detail(&cleanup.message, 120);
    let message = format!(
        "{primary_detail}; platform runner cleanup also failed ({}): {cleanup_detail}",
        cleanup.code
    );
    CoreError::new(
        primary.code,
        message,
        primary.severity,
        primary.retryable,
        primary.operation_id.clone(),
    )
    .unwrap_or(primary)
}

fn bounded_error_detail(value: &str, maximum_bytes: usize) -> String {
    let limit = maximum_bytes.min(MAX_ERROR_MESSAGE_BYTES);
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn runner_error(
    code: CoreErrorCode,
    message: &'static str,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(code, message, ErrorSeverity::Error, retryable, operation_id)
}
