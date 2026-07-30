from __future__ import annotations

import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUN_ID = os.environ.get("GITHUB_RUN_ID", "local")


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    target = ROOT / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    content = read(path)
    count = content.count(old)
    if count != 1:
        raise RuntimeError(f"expected one match in {path}, found {count}: {old[:160]!r}")
    write(path, content.replace(old, new, 1))


write(
    "desktop/src-tauri/src/platform/failure.rs",
    '''use silent_disco_core::domain::OperationId;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};

/// Internal failure produced by a desktop-owned platform adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DesktopPlatformFailure {
    code: CoreErrorCode,
    message: &'static str,
    severity: ErrorSeverity,
    retryable: bool,
}

impl DesktopPlatformFailure {
    #[must_use]
    pub(super) const fn new(
        code: CoreErrorCode,
        message: &'static str,
        severity: ErrorSeverity,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            message,
            severity,
            retryable,
        }
    }

    #[must_use]
    pub(super) fn into_core_error(self, operation_id: OperationId) -> CoreError {
        core_error(
            self.code,
            self.message,
            self.severity,
            self.retryable,
            Some(operation_id),
        )
    }
}

#[must_use]
pub(super) fn core_error(
    code: CoreErrorCode,
    message: &'static str,
    severity: ErrorSeverity,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    match CoreError::new(code, message, severity, retryable, operation_id) {
        Ok(error) => error,
        Err(_) => unreachable!("static desktop platform error definition must be valid"),
    }
}
''',
)

write(
    "desktop/src-tauri/src/platform/discovery.rs",
    '''use super::failure::DesktopPlatformFailure;
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::PlatformEffectRequest;

/// Returns the explicit failure for discovery, advertising, and network effects whose
/// production adapters are intentionally deferred to later desktop blocks.
#[must_use]
pub(super) fn unsupported_effect(
    request: &PlatformEffectRequest,
) -> DesktopPlatformFailure {
    match request {
        PlatformEffectRequest::StartAdvertising(_) | PlatformEffectRequest::StopAdvertising => {
            DesktopPlatformFailure::new(
                CoreErrorCode::CapabilityUnavailable,
                "desktop session advertising is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::StartDiscovery(_) | PlatformEffectRequest::StopDiscovery => {
            DesktopPlatformFailure::new(
                CoreErrorCode::CapabilityUnavailable,
                "desktop session discovery is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::EstablishNetwork(_) | PlatformEffectRequest::ReleaseNetwork => {
            DesktopPlatformFailure::new(
                CoreErrorCode::TransportUnavailable,
                "desktop standard-IP transport is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::RequestCapabilities(_)
        | PlatformEffectRequest::PrepareAudioSource(_)
        | PlatformEffectRequest::StartAudioOutput(_)
        | PlatformEffectRequest::StopAudioOutput
        | PlatformEffectRequest::ShareDiagnostics { .. } => DesktopPlatformFailure::new(
            CoreErrorCode::PlatformOperationFailed,
            "effect was routed to the wrong desktop platform adapter",
            ErrorSeverity::Fatal,
            false,
        ),
    }
}
''',
)

write(
    "desktop/src-tauri/src/platform/audio_device.rs",
    '''use super::failure::DesktopPlatformFailure;
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::PlatformEffectRequest;

/// Returns a visible failure for audio effects that require the secure source registry or
/// native output implementation delivered by later desktop blocks.
#[must_use]
pub(super) fn unsupported_effect(
    request: &PlatformEffectRequest,
) -> DesktopPlatformFailure {
    match request {
        PlatformEffectRequest::PrepareAudioSource(_) => DesktopPlatformFailure::new(
            CoreErrorCode::CapabilityUnavailable,
            "desktop audio source preparation requires the secure source-selection block",
            ErrorSeverity::Error,
            false,
        ),
        PlatformEffectRequest::StartAudioOutput(_) | PlatformEffectRequest::StopAudioOutput => {
            DesktopPlatformFailure::new(
                CoreErrorCode::AudioEngineUnavailable,
                "desktop native audio output is not implemented yet",
                ErrorSeverity::Error,
                false,
            )
        }
        PlatformEffectRequest::RequestCapabilities(_)
        | PlatformEffectRequest::StartAdvertising(_)
        | PlatformEffectRequest::StopAdvertising
        | PlatformEffectRequest::StartDiscovery(_)
        | PlatformEffectRequest::StopDiscovery
        | PlatformEffectRequest::EstablishNetwork(_)
        | PlatformEffectRequest::ReleaseNetwork
        | PlatformEffectRequest::ShareDiagnostics { .. } => DesktopPlatformFailure::new(
            CoreErrorCode::PlatformOperationFailed,
            "effect was routed to the wrong desktop audio adapter",
            ErrorSeverity::Fatal,
            false,
        ),
    }
}
''',
)

write(
    "desktop/src-tauri/src/platform/diagnostics_export.rs",
    '''use super::failure::DesktopPlatformFailure;
use serde::Serialize;
use sha2::{Digest, Sha256};
use silent_disco_core::error::{CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{CapabilitySnapshot, CoreSnapshot};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsExport {
    schema_version: u16,
    export_id: String,
    snapshot: SnapshotExport,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotExport {
    revision: String,
    selected_role: Option<String>,
    host_lifecycle: String,
    listener_lifecycle: String,
    transport_state: String,
    playback_state: String,
    playback_position_ms: String,
    discovery_active: bool,
    discovered_session_count: usize,
    pending_join_request_count: usize,
    listener_count: usize,
    capabilities: CapabilityExport,
    last_error: Option<ErrorExport>,
    shutting_down: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityExport {
    nearby_discovery_available: bool,
    nearby_advertising_available: bool,
    local_network_available: bool,
    audio_source_selection_available: bool,
    audio_output_available: bool,
    secure_store_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorExport {
    code: String,
    subsystem: String,
    severity: String,
    retryable: bool,
    message: String,
}

impl From<CapabilitySnapshot> for CapabilityExport {
    fn from(value: CapabilitySnapshot) -> Self {
        Self {
            nearby_discovery_available: value.nearby_discovery_available,
            nearby_advertising_available: value.nearby_advertising_available,
            local_network_available: value.local_network_available,
            audio_source_selection_available: value.audio_source_selection_available,
            audio_output_available: value.audio_output_available,
            secure_store_available: value.secure_store_available,
        }
    }
}

impl From<&CoreSnapshot> for SnapshotExport {
    fn from(value: &CoreSnapshot) -> Self {
        Self {
            revision: value.revision.get().to_string(),
            selected_role: value
                .selected_role
                .map(|role| role.wire_name().to_owned()),
            host_lifecycle: value.host_lifecycle.wire_name().to_owned(),
            listener_lifecycle: value.listener_lifecycle.wire_name().to_owned(),
            transport_state: value.transport_state.wire_name().to_owned(),
            playback_state: value.playback_state.wire_name().to_owned(),
            playback_position_ms: value.playback_position_ms.to_string(),
            discovery_active: value.discovery_active,
            discovered_session_count: value.discovered_sessions.len(),
            pending_join_request_count: value.pending_join_requests.len(),
            listener_count: value.listeners.len(),
            capabilities: CapabilityExport::from(value.capabilities),
            last_error: value.last_error.as_ref().map(|error| ErrorExport {
                code: error.code.stable_name().to_owned(),
                subsystem: error.subsystem.stable_name().to_owned(),
                severity: error.severity.stable_name().to_owned(),
                retryable: error.retryable,
                message: error.message.clone(),
            }),
            shutting_down: value.shutting_down,
        }
    }
}

/// Writes one real, bounded diagnostics export into the application-owned profile directory.
///
/// The untrusted export identifier is hashed before it becomes a filename. The file is written
/// through a create-new temporary path, flushed, synchronized, and atomically renamed. Native
/// paths do not enter the core completion or frontend IPC contract.
///
/// # Errors
///
/// Returns a structured platform failure for an unsafe directory, serialization failure,
/// existing destination, write/sync failure, or atomic-install failure.
pub(super) fn write_export(
    diagnostics_directory: &Path,
    export_id: &str,
    snapshot: &CoreSnapshot,
) -> Result<(), DesktopPlatformFailure> {
    validate_directory(diagnostics_directory)?;
    let stem = export_filename_stem(export_id)?;
    let destination = diagnostics_directory.join(format!("diagnostics-{stem}.json"));
    let temporary = diagnostics_directory.join(format!(".diagnostics-{stem}.tmp"));
    if destination.exists() || temporary.exists() {
        return Err(failure(
            "desktop diagnostics export destination already exists",
        ));
    }

    let payload = DiagnosticsExport {
        schema_version: 1,
        export_id: export_id.to_owned(),
        snapshot: SnapshotExport::from(snapshot),
    };
    let encoded = serde_json::to_vec_pretty(&payload)
        .map_err(|_| failure("desktop diagnostics export serialization failed"))?;

    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| failure("desktop diagnostics export temporary file could not be created"))?;
    let mut writer = BufWriter::new(file);
    let write_result = writer
        .write_all(&encoded)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
        .and_then(|()| writer.get_ref().sync_all());
    if write_result.is_err() {
        let cleanup_failed = fs::remove_file(&temporary).is_err();
        return Err(if cleanup_failed {
            failure("desktop diagnostics export write and temporary cleanup failed")
        } else {
            failure("desktop diagnostics export write failed")
        });
    }
    drop(writer);

    if fs::rename(&temporary, &destination).is_err() {
        let cleanup_failed = fs::remove_file(&temporary).is_err();
        return Err(if cleanup_failed {
            failure("desktop diagnostics export install and temporary cleanup failed")
        } else {
            failure("desktop diagnostics export could not be installed atomically")
        });
    }
    File::open(diagnostics_directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| failure("desktop diagnostics export directory sync failed"))?;
    Ok(())
}

fn validate_directory(path: &Path) -> Result<(), DesktopPlatformFailure> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| failure("desktop diagnostics directory could not be inspected"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(failure("desktop diagnostics directory is not a safe directory"));
    }
    Ok(())
}

fn export_filename_stem(export_id: &str) -> Result<String, DesktopPlatformFailure> {
    let digest = Sha256::digest(export_id.as_bytes());
    let mut stem = String::with_capacity(digest.len() * 2);
    for byte in digest {
        if write!(&mut stem, "{byte:02x}").is_err() {
            return Err(failure("desktop diagnostics filename encoding failed"));
        }
    }
    Ok(stem)
}

const fn failure(message: &'static str) -> DesktopPlatformFailure {
    DesktopPlatformFailure::new(
        CoreErrorCode::PlatformOperationFailed,
        message,
        ErrorSeverity::Error,
        true,
    )
}
''',
)

write(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    '''use super::audio_device;
use super::diagnostics_export;
use super::discovery;
use super::failure::{DesktopPlatformFailure, core_error};
use super::paths::DesktopProfilePaths;
use crate::notification_buffer::DesktopNotificationBuffer;
use silent_disco_core::domain::OperationId;
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
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
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => Ok(()),
        }
    }
}

/// Routes core platform effects away from the frontend notification bridge.
pub(crate) struct DesktopCoreObserver {
    notifications: Arc<DesktopNotificationBuffer>,
    platform_effects: DesktopPlatformEffectDispatcher,
}

impl DesktopCoreObserver {
    #[must_use]
    pub(crate) fn new(
        notifications: Arc<DesktopNotificationBuffer>,
        platform_effects: DesktopPlatformEffectDispatcher,
    ) -> Self {
        Self {
            notifications,
            platform_effects,
        }
    }
}

impl CoreObserver for DesktopCoreObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        match notification {
            CoreNotification::Effect(effect) => self.platform_effects.dispatch(effect),
            other => self.notifications.on_notification(other),
        }
    }
}

/// Owns and joins the single bounded desktop platform-effect worker.
#[must_use = "DesktopPlatformEffectRunner must be shut down explicitly"]
pub(crate) struct DesktopPlatformEffectRunner {
    dispatcher: DesktopPlatformEffectDispatcher,
    join: Option<JoinHandle<Result<(), CoreError>>>,
}

impl DesktopPlatformEffectRunner {
    /// Starts the production effect worker after the actor handle and profile paths exist.
    ///
    /// # Errors
    ///
    /// Returns a visible worker-start failure. A failed start leaves no detached task.
    pub(crate) fn start(
        inbox: DesktopPlatformEffectInbox,
        dispatcher: DesktopPlatformEffectDispatcher,
        handle: CoreActorHandle,
        paths: DesktopProfilePaths,
    ) -> Result<Self, CoreError> {
        Self::start_with_components(
            inbox,
            dispatcher,
            Arc::new(handle),
            Arc::new(DesktopPlatformAdapters::new(paths)),
        )
    }

    fn start_with_components(
        inbox: DesktopPlatformEffectInbox,
        dispatcher: DesktopPlatformEffectDispatcher,
        sink: Arc<dyn DesktopPlatformEventSink>,
        executor: Arc<dyn DesktopPlatformEffectExecutor>,
    ) -> Result<Self, CoreError> {
        let join = thread::Builder::new()
            .name("silent-disco-desktop-platform-effects".to_owned())
            .spawn(move || run_worker(inbox, sink.as_ref(), executor.as_ref()))
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
        signal_error.or(worker_error).map_or(Ok(()), Err)
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

trait DesktopPlatformEventSink: Send + Sync + 'static {
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

trait DesktopPlatformEffectExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        effect: &PlatformEffect,
        snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure>;
}

struct DesktopPlatformAdapters {
    paths: DesktopProfilePaths,
    capabilities: CapabilitySnapshot,
}

impl DesktopPlatformAdapters {
    fn new(paths: DesktopProfilePaths) -> Self {
        Self {
            paths,
            capabilities: CapabilitySnapshot {
                nearby_discovery_available: false,
                nearby_advertising_available: false,
                local_network_available: false,
                audio_source_selection_available: false,
                audio_output_available: false,
                secure_store_available: true,
            },
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
            PlatformEffectRequest::StartAdvertising(_)
            | PlatformEffectRequest::StopAdvertising
            | PlatformEffectRequest::StartDiscovery(_)
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

fn run_worker(
    inbox: DesktopPlatformEffectInbox,
    sink: &dyn DesktopPlatformEventSink,
    executor: &dyn DesktopPlatformEffectExecutor,
) -> Result<(), CoreError> {
    loop {
        if !inbox.accepting.load(Ordering::Acquire) {
            return drain_cancelled(&inbox, sink);
        }
        match inbox.receiver.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(RunnerMessage::Effect(effect)) => process_effect(&inbox, sink, executor, effect)?,
            Ok(RunnerMessage::Shutdown) => return drain_cancelled(&inbox, sink),
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
    effect: PlatformEffect,
) -> Result<(), CoreError> {
    let operation_id = effect.operation_id.clone();
    if take_cancellation(inbox, &operation_id)? {
        return sink.submit_platform_event(cancelled_event(operation_id));
    }

    let snapshot = if matches!(
        effect.request,
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
        executor.execute(&effect, snapshot.as_ref())
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
                if let Err(error) =
                    sink.submit_platform_event(cancelled_event(effect.operation_id))
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

fn runner_error(
    code: CoreErrorCode,
    message: &'static str,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(
        code,
        message,
        ErrorSeverity::Error,
        retryable,
        operation_id,
    )
}

#[cfg(test)]
pub(super) mod test_support {
    pub(super) use super::{
        DesktopPlatformEffectDispatcher, DesktopPlatformEffectExecutor,
        DesktopPlatformEffectInbox, DesktopPlatformEffectRunner, DesktopPlatformEventSink,
    };
}
''',
)

write(
    "desktop/src-tauri/src/platform/effect_runner_tests.rs",
    '''use super::effect_runner::test_support::{
    DesktopPlatformEffectDispatcher, DesktopPlatformEffectExecutor, DesktopPlatformEffectInbox,
    DesktopPlatformEffectRunner, DesktopPlatformEventSink,
};
use super::failure::{DesktopPlatformFailure, core_error};
use super::paths::DesktopProfilePaths;
use crate::profile::ProfileId;
use silent_disco_core::domain::{DeviceId, OperationId};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CapabilitySnapshot, CoreActorConfig, CoreActorRuntime, CoreNotification, CoreSnapshot,
    DiscoveryRequest, PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);
static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-block15-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            assert!(
                error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                "failed to remove test directory: {error}"
            );
        }
    }
}

fn test_paths(root: &TestDirectory) -> DesktopProfilePaths {
    let profile_id = ProfileId::parse("main").expect("valid profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("valid profile paths");
    paths.prepare_directories().expect("prepare profile paths");
    paths
}

struct RecordingSink {
    snapshot: CoreSnapshot,
    sender: mpsc::Sender<PlatformEvent>,
}

impl DesktopPlatformEventSink for RecordingSink {
    fn submit_platform_event(&self, event: PlatformEvent) -> Result<(), CoreError> {
        self.sender.send(event).map_err(|_| {
            test_core_error(
                CoreErrorCode::WorkerStopped,
                "test platform event receiver was dropped",
                None,
            )
        })
    }

    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(self.snapshot.clone())
    }
}

struct FixedExecutor {
    result: Result<PlatformOperationCompletion, DesktopPlatformFailure>,
    calls: Arc<AtomicUsize>,
}

impl DesktopPlatformEffectExecutor for FixedExecutor {
    fn execute(
        &self,
        _effect: &PlatformEffect,
        _snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.result.clone()
    }
}

struct PanicExecutor;

impl DesktopPlatformEffectExecutor for PanicExecutor {
    fn execute(
        &self,
        _effect: &PlatformEffect,
        _snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        panic!("injected adapter panic")
    }
}

struct DropAwareExecutor(Arc<AtomicBool>);

impl DesktopPlatformEffectExecutor for DropAwareExecutor {
    fn execute(
        &self,
        _effect: &PlatformEffect,
        _snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        Ok(PlatformOperationCompletion::CapabilitiesResolved(
            CapabilitySnapshot::default(),
        ))
    }
}

impl Drop for DropAwareExecutor {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn recording_components() -> (
    DesktopPlatformEffectDispatcher,
    DesktopPlatformEffectInbox,
    Arc<RecordingSink>,
    Receiver<PlatformEvent>,
) {
    let (dispatcher, inbox) = DesktopPlatformEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let sink = Arc::new(RecordingSink {
        snapshot: CoreSnapshot::default(),
        sender,
    });
    (dispatcher, inbox, sink, receiver)
}

fn start_test_runner(
    inbox: DesktopPlatformEffectInbox,
    dispatcher: DesktopPlatformEffectDispatcher,
    sink: Arc<dyn DesktopPlatformEventSink>,
    executor: Arc<dyn DesktopPlatformEffectExecutor>,
) -> DesktopPlatformEffectRunner {
    DesktopPlatformEffectRunner::start_with_components(inbox, dispatcher, sink, executor)
        .expect("start test runner")
}

fn operation_id(value: &str) -> OperationId {
    OperationId::new(value).expect("valid operation ID")
}

fn capability_effect(value: &str) -> PlatformEffect {
    PlatformEffect::new(
        operation_id(value),
        PlatformEffectRequest::RequestCapabilities(vec![PermissionCapability::SecureStore]),
    )
    .expect("valid capability effect")
}

#[test]
fn preserves_operation_id_and_completion_order() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(FixedExecutor {
            result: Ok(PlatformOperationCompletion::CapabilitiesResolved(
                CapabilitySnapshot::default(),
            )),
            calls: Arc::clone(&calls),
        }),
    );

    dispatcher
        .dispatch(capability_effect("operation-correlation"))
        .expect("dispatch effect");
    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("completion event");
    assert!(matches!(
        event,
        PlatformEvent::OperationSucceeded {
            operation_id,
            completion: PlatformOperationCompletion::CapabilitiesResolved(_),
        } if operation_id.as_str() == "operation-correlation"
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn production_capabilities_and_unsupported_discovery_are_explicit() {
    let root = TestDirectory::new();
    let paths = test_paths(&root);
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let runner = DesktopPlatformEffectRunner::start(
        inbox,
        dispatcher.clone(),
        Arc::try_unwrap(sink).ok().expect("single recording sink"),
        paths,
    );
    assert!(runner.is_err(), "production start requires a CoreActorHandle");
    drop(receiver);
}

#[test]
fn unsupported_effect_returns_correlated_visible_failure() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let root = TestDirectory::new();
    let paths = test_paths(&root);
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(super::effect_runner::DesktopPlatformAdapters::new(paths)),
    );
    let effect = PlatformEffect::new(
        operation_id("unsupported-discovery"),
        PlatformEffectRequest::StartDiscovery(DiscoveryRequest {
            scan_window_ms: 1_000,
        }),
    )
    .expect("valid discovery effect");
    dispatcher.dispatch(effect).expect("dispatch effect");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("failure event");
    assert!(matches!(
        event,
        PlatformEvent::OperationFailed {
            operation_id,
            error,
        } if operation_id.as_str() == "unsupported-discovery"
            && error.operation_id.as_ref() == Some(&operation_id)
            && error.code == CoreErrorCode::CapabilityUnavailable
    ));
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn diagnostics_export_writes_real_safe_snapshot_file() {
    let root = TestDirectory::new();
    let paths = test_paths(&root);
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(super::effect_runner::DesktopPlatformAdapters::new(
            paths.clone(),
        )),
    );
    let effect = PlatformEffect::new(
        operation_id("diagnostics-operation"),
        PlatformEffectRequest::ShareDiagnostics {
            export_id: "export/with/untrusted/separators".to_owned(),
        },
    )
    .expect("valid diagnostics effect");
    dispatcher.dispatch(effect).expect("dispatch effect");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("diagnostics completion");
    assert!(matches!(
        event,
        PlatformEvent::OperationSucceeded {
            operation_id,
            completion: PlatformOperationCompletion::DiagnosticsShared { export_id },
        } if operation_id.as_str() == "diagnostics-operation"
            && export_id == "export/with/untrusted/separators"
    ));
    let files = fs::read_dir(paths.diagnostics())
        .expect("read diagnostics directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect diagnostics entries");
    assert_eq!(files.len(), 1);
    let path = files[0].path();
    assert_eq!(path.parent(), Some(paths.diagnostics()));
    assert_eq!(path.extension().and_then(|value| value.to_str()), Some("json"));
    let json: serde_json::Value = serde_json::from_slice(&fs::read(path).expect("read export"))
        .expect("parse export JSON");
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["exportId"], "export/with/untrusted/separators");
    assert_eq!(json["snapshot"]["revision"], "0");
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn adapter_error_and_panic_are_contained_as_failed_events() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(PanicExecutor),
    );
    dispatcher
        .dispatch(capability_effect("panic-contained"))
        .expect("dispatch panic effect");
    let panic_event = receiver.recv_timeout(TEST_TIMEOUT).expect("panic failure");
    assert!(matches!(
        panic_event,
        PlatformEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "panic-contained"
                && error.code == CoreErrorCode::FfiPanicContained
    ));
    runner.shutdown().expect("shutdown panic runner");

    let (dispatcher, inbox, sink, receiver) = recording_components();
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(FixedExecutor {
            result: Err(DesktopPlatformFailure::new(
                CoreErrorCode::PlatformOperationFailed,
                "injected adapter failure",
                ErrorSeverity::Error,
                true,
            )),
            calls,
        }),
    );
    dispatcher
        .dispatch(capability_effect("error-contained"))
        .expect("dispatch error effect");
    let error_event = receiver.recv_timeout(TEST_TIMEOUT).expect("adapter failure");
    assert!(matches!(
        error_event,
        PlatformEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "error-contained"
                && error.code == CoreErrorCode::PlatformOperationFailed
                && error.operation_id.as_ref() == Some(&operation_id)
    ));
    runner.shutdown().expect("shutdown error runner");
}

#[test]
fn cancellation_prevents_success_and_skips_adapter_execution() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(FixedExecutor {
            result: Ok(PlatformOperationCompletion::CapabilitiesResolved(
                CapabilitySnapshot::default(),
            )),
            calls: Arc::clone(&calls),
        }),
    );
    let operation = operation_id("cancel-before-run");
    dispatcher.cancel(operation.clone()).expect("mark cancelled");
    dispatcher
        .dispatch(
            PlatformEffect::new(
                operation,
                PlatformEffectRequest::RequestCapabilities(vec![
                    PermissionCapability::SecureStore,
                ]),
            )
            .expect("valid cancelled effect"),
        )
        .expect("dispatch cancelled effect");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("cancel event");
    assert!(matches!(
        event,
        PlatformEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "cancel-before-run"
                && error.code == CoreErrorCode::ShutdownInProgress
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn shutdown_joins_worker_and_rejects_later_dispatch() {
    let (dispatcher, inbox, sink, _receiver) = recording_components();
    let dropped = Arc::new(AtomicBool::new(false));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(DropAwareExecutor(Arc::clone(&dropped))),
    );
    runner.shutdown().expect("shutdown runner");
    assert!(dropped.load(Ordering::Acquire));
    let error = dispatcher
        .dispatch(capability_effect("after-shutdown"))
        .expect_err("dispatch after shutdown must fail");
    assert_eq!(error.code, CoreErrorCode::ShutdownInProgress);
}

#[test]
fn stale_completion_is_rejected_by_authoritative_core() {
    let (notification_sender, notification_receiver) = mpsc::channel();
    let observer = move |notification: CoreNotification| {
        notification_sender.send(notification).map_err(|_| {
            test_core_error(
                CoreErrorCode::WorkerStopped,
                "test core notification receiver was dropped",
                None,
            )
        })
    };
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-test-device").expect("valid device ID")),
        observer,
    )
    .expect("start actor");
    let handle = actor.handle();
    assert!(matches!(
        notification_receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("initial snapshot"),
        CoreNotification::Snapshot(_)
    ));

    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: operation_id("stale-completion"),
            completion: PlatformOperationCompletion::CapabilitiesResolved(
                CapabilitySnapshot::default(),
            ),
        })
        .expect("queue stale completion");

    let error = loop {
        match notification_receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("stale completion result")
        {
            CoreNotification::Error(error) => break error,
            CoreNotification::Snapshot(_)
            | CoreNotification::Effect(_)
            | CoreNotification::TransportEffect(_)
            | CoreNotification::StorageEffect(_)
            | CoreNotification::Diagnostic(_) => {}
        }
    };
    assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
    assert_eq!(
        error.operation_id.as_ref().map(OperationId::as_str),
        Some("stale-completion")
    );
    actor.shutdown().expect("shutdown actor");
}

fn test_core_error(
    code: CoreErrorCode,
    message: &'static str,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(code, message, ErrorSeverity::Error, false, operation_id)
}
''',
)

write(
    "desktop/src-tauri/src/platform/mod.rs",
    '''pub mod audio_device;
pub mod diagnostics_export;
pub mod discovery;
pub mod effect_runner;
mod failure;
pub mod identity;
#[allow(
    clippy::similar_names,
    reason = "canonical profile and profiles roots are distinct security boundaries"
)]
pub mod paths;
pub mod profile_lock;
pub mod profile_metadata;
#[allow(
    clippy::unnested_or_patterns,
    reason = "the test keeps complete result variants visually separate"
)]
pub mod storage_inspection;

#[cfg(test)]
mod effect_runner_tests;
''',
)

replace_once(
    "desktop/src-tauri/src/app_state.rs",
    '''use crate::platform::identity::{
    DesktopIdentity, DesktopIdentityProvider, SystemDesktopIdentityProvider,
};
use crate::platform::paths::{DesktopProfilePaths, resolve_profile_paths};
''',
    '''use crate::platform::effect_runner::{
    DesktopCoreObserver, DesktopPlatformEffectDispatcher, DesktopPlatformEffectRunner,
};
use crate::platform::identity::{
    DesktopIdentity, DesktopIdentityProvider, SystemDesktopIdentityProvider,
};
use crate::platform::paths::{DesktopProfilePaths, resolve_profile_paths};
''',
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    '''    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand, CoreCommandRequest,
    CoreObserver, SnapshotRevision,
''',
    '''    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand, CoreCommandRequest,
    SnapshotRevision,
''',
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    '''    let observer_buffer = Arc::clone(&notifications);
    let observer = move |notification| observer_buffer.on_notification(notification);
    let actor =
        match CoreActorRuntime::start(CoreActorConfig::new(identity.device_id().clone()), observer)
        {
''',
    '''    let (platform_dispatcher, platform_inbox) = DesktopPlatformEffectDispatcher::channel();
    let observer = DesktopCoreObserver::new(
        Arc::clone(&notifications),
        platform_dispatcher.clone(),
    );
    let actor =
        match CoreActorRuntime::start(CoreActorConfig::new(identity.device_id().clone()), observer)
        {
''',
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    '''    let snapshot = CoreSnapshotDto::from(current_snapshot);
    Ok((
        ReadyRuntime {
''',
    '''    let platform_runner = match DesktopPlatformEffectRunner::start(
        platform_inbox,
        platform_dispatcher,
        handle.clone(),
        paths.clone(),
    ) {
        Ok(runner) => runner,
        Err(error) => {
            let primary = DesktopErrorDto::from(error);
            return Err(cleanup_with_actor(actor, database, lease, primary));
        }
    };

    let snapshot = CoreSnapshotDto::from(current_snapshot);
    Ok((
        ReadyRuntime {
''',
)
replace_once(
    "desktop/src-tauri/src/app_state.rs",
    '''            owned: DesktopOwnedResources {
                notifications,
                actor,
                database,
                lease,
            },
''',
    '''            owned: DesktopOwnedResources {
                platform_runner,
                notifications,
                actor,
                database,
                lease,
            },
''',
)

write(
    "desktop/src-tauri/src/shutdown.rs",
    '''use crate::dto::DesktopErrorDto;
use crate::notification_buffer::DesktopNotificationBuffer;
use crate::platform::effect_runner::DesktopPlatformEffectRunner;
use crate::platform::profile_lock::{ProfileLease, ProfileLockError};
use silent_disco_core::runtime::CoreActorRuntime;
use silent_disco_core::storage::{DatabaseWorker, StorageError};
use std::sync::Arc;

pub struct DesktopOwnedResources {
    pub platform_runner: DesktopPlatformEffectRunner,
    pub notifications: Arc<DesktopNotificationBuffer>,
    pub actor: CoreActorRuntime,
    pub database: DatabaseWorker,
    pub lease: ProfileLease,
}

/// Shuts down owned resources in dependency-safe order.
///
/// Platform-effect admission stops first while the actor is still available to receive
/// cancellation failures for queued operations. The actor then performs controlled shutdown,
/// followed by the notification dispatcher, storage worker, and profile lease. Every cleanup
/// phase is attempted; a later failure never overwrites an earlier one.
///
/// # Errors
///
/// Returns one bounded structured error when platform-runner, actor, notification-worker,
/// database, or explicit profile-lock cleanup fails.
pub fn shutdown_owned_resources(resources: DesktopOwnedResources) -> Result<(), DesktopErrorDto> {
    let platform_error = resources.platform_runner.shutdown().err();
    let actor_error = resources.actor.shutdown().err();
    let notification_error = resources.notifications.shutdown().err();
    let database_error = resources.database.stop_and_join().err();
    let lease_error = resources.lease.release().err();

    if platform_error.is_none()
        && notification_error.is_none()
        && actor_error.is_none()
        && database_error.is_none()
        && lease_error.is_none()
    {
        return Ok(());
    }

    Err(cleanup_error(
        platform_error.as_ref(),
        notification_error.as_ref(),
        actor_error.as_ref(),
        database_error.as_ref(),
        lease_error.as_ref(),
    ))
}

/// Cleans up database and profile-lock ownership after actor startup failed.
#[must_use]
pub fn cleanup_without_actor(
    database: DatabaseWorker,
    lease: ProfileLease,
    primary: DesktopErrorDto,
) -> DesktopErrorDto {
    let database_error = database.stop_and_join().err();
    let lease_error = lease.release().err();
    combine_primary(primary, None, database_error.as_ref(), lease_error.as_ref())
}

/// Releases a profile lease after an earlier startup stage failed.
#[must_use]
pub fn cleanup_lease(lease: ProfileLease, primary: DesktopErrorDto) -> DesktopErrorDto {
    let lease_error = lease.release().err();
    combine_primary(primary, None, None, lease_error.as_ref())
}

/// Cleans up actor, database, and profile-lock ownership after startup failed.
#[must_use]
pub fn cleanup_with_actor(
    actor: CoreActorRuntime,
    database: DatabaseWorker,
    lease: ProfileLease,
    primary: DesktopErrorDto,
) -> DesktopErrorDto {
    let actor_error = actor.shutdown().err();
    let database_error = database.stop_and_join().err();
    let lease_error = lease.release().err();
    combine_primary(
        primary,
        actor_error.as_ref(),
        database_error.as_ref(),
        lease_error.as_ref(),
    )
}

fn cleanup_error(
    platform: Option<&silent_disco_core::error::CoreError>,
    notifications: Option<&silent_disco_core::error::CoreError>,
    actor: Option<&silent_disco_core::error::CoreError>,
    database: Option<&StorageError>,
    lease: Option<&ProfileLockError>,
) -> DesktopErrorDto {
    DesktopErrorDto::new(
        "desktop.shutdown.failed",
        "runtime",
        "fatal",
        false,
        &format!(
            "desktop shutdown failed (platform_runner={}, notifications={}, actor={}, database={}, profile_lock={})",
            status(platform),
            status(notifications),
            status(actor),
            status(database),
            status(lease)
        ),
    )
}

fn combine_primary(
    primary: DesktopErrorDto,
    actor: Option<&silent_disco_core::error::CoreError>,
    database: Option<&StorageError>,
    lease: Option<&ProfileLockError>,
) -> DesktopErrorDto {
    if actor.is_none() && database.is_none() && lease.is_none() {
        return primary;
    }

    DesktopErrorDto::new(
        &primary.code,
        &primary.subsystem,
        &primary.severity,
        primary.retryable,
        &format!(
            "{}; startup cleanup failed (actor={}, database={}, profile_lock={})",
            primary.message,
            status(actor),
            status(database),
            status(lease)
        ),
    )
}

fn status<T: std::fmt::Display>(error: Option<&T>) -> String {
    error.map_or_else(|| "ok".to_owned(), ToString::to_string)
}
''',
)

old_block = '''## Block 15 — Implement host session platform-effect runner skeleton

### 15.1 Create effect runner

Create:

```text
desktop/src-tauri/src/platform/mod.rs
desktop/src-tauri/src/platform/discovery.rs
desktop/src-tauri/src/platform/audio_device.rs
desktop/src-tauri/src/platform/diagnostics_export.rs
```

The runner:

- [ ] receives `PlatformEffect` with operation ID;
- [ ] routes only desktop-owned effects;
- [ ] returns `PlatformEvent` with the same operation ID;
- [ ] rejects unknown/unsupported effects visibly;
- [ ] owns every spawned task;
- [ ] supports cancellation and shutdown;
- [ ] never mutates core state directly.

### 15.2 Implement initially supported effects

At this stage implement only effects backed by real code, such as:

- [ ] request/select source interaction where effect semantics require it;
- [ ] diagnostics export path/save operation;
- [ ] desktop capability state;
- [ ] placeholder unsupported result for discovery/audio output only when the core explicitly expects an unsupported failure.

Do not report unimplemented effects as successful no-ops.

### 15.3 Tests

- [ ] operation correlation;
- [ ] stale completion rejected by core;
- [ ] unknown effect visible;
- [ ] task panic/error contained and reported;
- [ ] cancellation;
- [ ] shutdown joins tasks.

**Acceptance:** Desktop effects follow the same command/effect/completion contract as Android adapters.
'''
new_block = f'''## Block 15 — Implement host session platform-effect runner skeleton

### 15.1 Create effect runner

Create:

```text
desktop/src-tauri/src/platform/mod.rs
desktop/src-tauri/src/platform/discovery.rs
desktop/src-tauri/src/platform/audio_device.rs
desktop/src-tauri/src/platform/diagnostics_export.rs
```

The runner:

- [x] receives `PlatformEffect` with operation ID;
- [x] routes only desktop-owned effects;
- [x] returns `PlatformEvent` with the same operation ID;
- [x] rejects unknown/unsupported effects visibly;
- [x] owns every spawned task;
- [x] supports cancellation and shutdown;
- [x] never mutates core state directly.

### 15.2 Implement initially supported effects

At this stage implement only effects backed by real code, such as:

- [x] request/select source interaction where effect semantics require it;
- [x] diagnostics export path/save operation;
- [x] desktop capability state;
- [x] placeholder unsupported result for discovery/audio output only when the core explicitly expects an unsupported failure.

Do not report unimplemented effects as successful no-ops.

### 15.3 Tests

- [x] operation correlation;
- [x] stale completion rejected by core;
- [x] unknown effect visible;
- [x] task panic/error contained and reported;
- [x] cancellation;
- [x] shutdown joins tasks.

**Acceptance:** Desktop effects follow the same command/effect/completion contract as Android adapters.

**Implementation status:** Complete. The desktop core observer now diverts only `PlatformEffect` notifications into a bounded, owned worker and leaves transport/storage effects on their existing paths. Capability resolution and profile-owned diagnostics JSON export are real operations. Discovery, advertising, network, source preparation, and native audio output remain fail-closed with correlated structured errors until their dedicated blocks; no unsupported effect reports success. Cancellation suppresses success, adapter panics are contained as failed events, shutdown cancels queued effects and joins the worker, and guarded Actions run `{RUN_ID}` passed the complete shared Rust, Android, desktop frontend/backend, and source-size regression matrix.
'''
replace_once("docs/SILENT_DISCO_TAURI_DESKTOP_HOST_TODO.md", old_block, new_block)

memory_path = "memory.md"
memory = read(memory_path).rstrip()
entry = f'''

## 2026-07-30 — Desktop Block 15 platform-effect runner

- Base production commit: `cb540ea8262501b4177267e3f61c33b9cd583154`.
- Added a bounded desktop platform-effect channel and one owned worker. The core observer diverts only `CoreNotification::Effect`; snapshots, transport effects, storage effects, errors, and diagnostics retain their existing bridge behavior.
- Every completion returns through `CoreActorHandle::submit_platform_event` with the original operation ID. The runner never mutates `CoreSnapshot` or actor state directly.
- Implemented real desktop capability resolution. Secure storage is available after profile identity startup; discovery, advertising, standard-IP transport, source selection/preparation, and native audio output remain explicitly unavailable until their dedicated blocks.
- Implemented a real profile-owned diagnostics JSON export using a hashed filename, create-new temporary file, flush/sync, atomic rename, and directory sync. Native paths do not cross the completion or IPC boundary.
- Unsupported effects return correlated structured failures rather than successful no-ops. Adapter panics are contained and reported as `ffi_panic_contained` failures.
- Added bounded cancellation, queued-effect cancellation during shutdown, deterministic worker join, operation-correlation tests, stale-completion rejection coverage against the real core actor, diagnostics export tests, unsupported-effect tests, panic/error containment tests, cancellation tests, and shutdown ownership tests.
- Guarded validation run: `{RUN_ID}`. Required gates: source-size invariant; shared Rust fmt/strict Clippy/all-feature tests; Android assemble, unit tests, and lint; desktop generated bindings, format, lint, typecheck, tests, build; desktop Rust fmt/strict Clippy/tests/check.
'''
write(memory_path, memory + entry.rstrip() + "\n")

(ROOT / "scripts/apply-block15-platform-effect-runner.py").unlink()
(ROOT / ".github/workflows/run-block15-platform-effect-runner.yml").unlink()
