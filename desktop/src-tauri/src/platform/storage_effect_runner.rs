use super::failure::core_error;
use silent_disco_core::domain::{OperationId, TrustState};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CoreActorHandle, StorageCompletion, StorageEffect, StorageEffectRequest, StorageEvent,
};
use silent_disco_core::storage::{DatabaseClient, StorageError, StoredSettings, TrustedDevice};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STORAGE_EFFECT_QUEUE_CAPACITY: usize = 16;
const STORAGE_EFFECT_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) trait DesktopStorageEventSink: Send + Sync + 'static {
    fn submit_storage_event(&self, event: StorageEvent) -> Result<(), CoreError>;
}

impl DesktopStorageEventSink for CoreActorHandle {
    fn submit_storage_event(&self, event: StorageEvent) -> Result<(), CoreError> {
        CoreActorHandle::submit_storage_event(self, event)
    }
}

#[derive(Clone)]
pub(crate) struct DesktopStorageEffectDispatcher {
    sender: SyncSender<StorageEffect>,
    accepting: Arc<AtomicBool>,
}

pub(crate) struct DesktopStorageEffectInbox {
    receiver: Receiver<StorageEffect>,
}

impl DesktopStorageEffectDispatcher {
    #[must_use]
    pub(crate) fn channel() -> (Self, DesktopStorageEffectInbox) {
        let (sender, receiver) = sync_channel(STORAGE_EFFECT_QUEUE_CAPACITY);
        let accepting = Arc::new(AtomicBool::new(true));
        (
            Self {
                sender,
                accepting: Arc::clone(&accepting),
            },
            DesktopStorageEffectInbox { receiver },
        )
    }

    pub(crate) fn dispatch(&self, effect: StorageEffect) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        if !self.accepting.load(Ordering::Acquire) {
            return Err(core_error(
                CoreErrorCode::ShutdownInProgress,
                "desktop storage-effect runner is shutting down",
                ErrorSeverity::Error,
                false,
                Some(operation_id),
            ));
        }
        match self.sender.try_send(effect) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(core_error(
                CoreErrorCode::QueueOverflow,
                "desktop storage-effect queue is full",
                ErrorSeverity::Error,
                true,
                Some(operation_id),
            )),
            Err(TrySendError::Disconnected(_)) => Err(core_error(
                CoreErrorCode::WorkerStopped,
                "desktop storage-effect worker is unavailable",
                ErrorSeverity::Fatal,
                false,
                Some(operation_id),
            )),
        }
    }
}

pub(crate) struct DesktopStorageEffectRunner {
    dispatcher: DesktopStorageEffectDispatcher,
    join: JoinHandle<Result<(), CoreError>>,
}

impl DesktopStorageEffectRunner {
    pub(crate) fn start(
        inbox: DesktopStorageEffectInbox,
        dispatcher: DesktopStorageEffectDispatcher,
        sink: Arc<dyn DesktopStorageEventSink>,
        database: DatabaseClient,
    ) -> Result<Self, CoreError> {
        let accepting = Arc::clone(&dispatcher.accepting);
        let join = thread::Builder::new()
            .name("silent-disco-desktop-storage-effects".to_owned())
            .spawn(move || run_worker(&inbox.receiver, &sink, &database, &accepting))
            .map_err(|error| {
                core_error(
                    CoreErrorCode::WorkerStopped,
                    format!("failed to start desktop storage-effect worker: {error}"),
                    ErrorSeverity::Fatal,
                    false,
                    None,
                )
            })?;
        Ok(Self { dispatcher, join })
    }

    pub(crate) fn shutdown(self) -> Result<(), CoreError> {
        self.dispatcher.accepting.store(false, Ordering::Release);
        drop(self.dispatcher);
        match self.join.join() {
            Ok(result) => result,
            Err(_) => Err(core_error(
                CoreErrorCode::ShutdownFailed,
                "desktop storage-effect worker panicked during shutdown",
                ErrorSeverity::Fatal,
                false,
                None,
            )),
        }
    }
}

fn run_worker(
    receiver: &Receiver<StorageEffect>,
    sink: &Arc<dyn DesktopStorageEventSink>,
    database: &DatabaseClient,
    accepting: &AtomicBool,
) -> Result<(), CoreError> {
    loop {
        match receiver.recv_timeout(STORAGE_EFFECT_POLL_INTERVAL) {
            Ok(effect) => {
                let event = execute_effect(database, effect);
                sink.submit_storage_event(event)?;
            }
            Err(RecvTimeoutError::Timeout) if accepting.load(Ordering::Acquire) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn execute_effect(database: &DatabaseClient, effect: StorageEffect) -> StorageEvent {
    let operation_id = effect.operation_id;
    let result = catch_unwind(AssertUnwindSafe(|| match effect.request {
        StorageEffectRequest::PersistSettings { settings } => persist_settings(database, settings),
        StorageEffectRequest::PersistTrustedDevice {
            device_id,
            display_name,
        } => persist_trusted_device(database, device_id, display_name),
    }));
    match result {
        Ok(Ok(completion)) => StorageEvent::OperationSucceeded {
            operation_id,
            completion,
        },
        Ok(Err(error)) => StorageEvent::OperationFailed {
            operation_id: operation_id.clone(),
            error: storage_core_error(error, operation_id),
        },
        Err(_) => StorageEvent::OperationFailed {
            operation_id: operation_id.clone(),
            error: core_error(
                CoreErrorCode::FfiPanicContained,
                "desktop storage adapter panicked",
                ErrorSeverity::Error,
                false,
                Some(operation_id),
            ),
        },
    }
}

fn persist_settings(
    database: &DatabaseClient,
    settings: silent_disco_core::domain::TuningSettings,
) -> Result<StorageCompletion, StorageError> {
    database.save_settings(&StoredSettings {
        tuning: settings,
        updated_at_ms: unix_time_ms(),
    })?;
    Ok(StorageCompletion::SettingsSaved)
}

fn persist_trusted_device(
    database: &DatabaseClient,
    device_id: silent_disco_core::domain::DeviceId,
    display_name: String,
) -> Result<StorageCompletion, StorageError> {
    let now_ms = unix_time_ms();
    let existing = database.get_trusted_device(&device_id)?;
    let device = match existing {
        Some(mut current) => {
            current.display_name = display_name;
            current.trust_state = TrustState::Trusted;
            current.last_seen_ms = now_ms.max(current.last_seen_ms);
            current.updated_at_ms = now_ms.max(current.last_seen_ms);
            current
        }
        None => TrustedDevice {
            device_id: device_id.clone(),
            display_name,
            public_key: None,
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            updated_at_ms: now_ms,
        },
    };
    database.upsert_trusted_device(&device)?;
    Ok(StorageCompletion::TrustedDeviceUpdated { device_id })
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| {
            u64::try_from(duration.as_millis())
                .unwrap_or(i64::MAX as u64)
                .min(i64::MAX as u64)
                .max(1)
        })
}

fn storage_core_error(error: StorageError, operation_id: OperationId) -> CoreError {
    core_error(
        error.kind.core_error_code(),
        error.message,
        if error.core_remains_usable {
            ErrorSeverity::Error
        } else {
            ErrorSeverity::Fatal
        },
        error.retryable,
        Some(operation_id),
    )
}
