use super::storage_effect_runner::{
    DesktopStorageEffectDispatcher, DesktopStorageEffectRunner, DesktopStorageEventSink,
};
use silent_disco_core::domain::{DeviceId, OperationId, TrustState};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    StorageCompletion, StorageEffect, StorageEffectRequest, StorageEvent,
};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

struct RecordingSink(mpsc::Sender<StorageEvent>);

impl DesktopStorageEventSink for RecordingSink {
    fn submit_storage_event(&self, event: StorageEvent) -> Result<(), CoreError> {
        self.0.send(event).map_err(|_| {
            CoreError::new(
                CoreErrorCode::WorkerStopped,
                "storage test sink closed",
                ErrorSeverity::Error,
                false,
                None,
            )
            .expect("static error")
        })
    }
}

#[test]
fn trusted_device_persistence_completes_before_approval_can_continue() {
    let path = temporary_database_path("success");
    let database =
        DatabaseWorker::start(DatabaseConfig::new(&path).expect("config")).expect("database");
    let client = database.client();
    let (dispatcher, inbox) = DesktopStorageEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let runner = DesktopStorageEffectRunner::start(
        inbox,
        dispatcher.clone(),
        Arc::new(RecordingSink(sender)),
        client.clone(),
    )
    .expect("runner");
    let device_id = DeviceId::new("listener-trusted").expect("device");
    dispatcher
        .dispatch(
            StorageEffect::new(
                OperationId::new("storage-trust-1").expect("operation"),
                StorageEffectRequest::PersistTrustedDevice {
                    device_id: device_id.clone(),
                    display_name: "Trusted Listener".to_owned(),
                },
            )
            .expect("effect"),
        )
        .expect("dispatch");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("completion");
    assert!(matches!(
        event,
        StorageEvent::OperationSucceeded {
            completion: StorageCompletion::TrustedDeviceUpdated { ref device_id },
            ..
        } if device_id.as_str() == "listener-trusted"
    ));
    let stored = client
        .get_trusted_device(&device_id)
        .expect("query")
        .expect("trusted device");
    assert_eq!(stored.display_name, "Trusted Listener");
    assert_eq!(stored.trust_state, TrustState::Trusted);

    runner.shutdown().expect("runner shutdown");
    database.stop_and_join().expect("database shutdown");
    drop(std::fs::remove_file(path));
}

#[test]
fn database_failure_is_returned_as_correlated_storage_event() {
    let path = temporary_database_path("failure");
    let database =
        DatabaseWorker::start(DatabaseConfig::new(&path).expect("config")).expect("database");
    let client = database.client();
    database.stop_and_join().expect("stop database");

    let (dispatcher, inbox) = DesktopStorageEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let runner = DesktopStorageEffectRunner::start(
        inbox,
        dispatcher.clone(),
        Arc::new(RecordingSink(sender)),
        client,
    )
    .expect("runner");
    dispatcher
        .dispatch(
            StorageEffect::new(
                OperationId::new("storage-trust-failure").expect("operation"),
                StorageEffectRequest::PersistTrustedDevice {
                    device_id: DeviceId::new("listener-failure").expect("device"),
                    display_name: "Failure Listener".to_owned(),
                },
            )
            .expect("effect"),
        )
        .expect("dispatch");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("failure event");
    assert!(matches!(
        event,
        StorageEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "storage-trust-failure"
                && error.operation_id.as_ref() == Some(&operation_id)
                && matches!(
                    error.code,
                    CoreErrorCode::WorkerStopped | CoreErrorCode::StorageBusy
                )
    ));
    runner.shutdown().expect("runner shutdown");
    drop(std::fs::remove_file(path));
}

fn temporary_database_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "silent-disco-block23-{label}-{}-{}.sqlite3",
        std::process::id(),
        OperationId::new(format!("{label}-path"))
            .expect("path ID")
            .as_str()
    ))
}
