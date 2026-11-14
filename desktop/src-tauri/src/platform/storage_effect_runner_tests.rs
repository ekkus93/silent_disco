use super::storage_effect_runner::{
    DesktopStorageEffectDispatcher, DesktopStorageEffectRunner, DesktopStorageEventSink,
};
use super::verified_trust::DesktopVerifiedTrust;
use silent_disco_core::domain::{DeviceId, OperationId, TrustState};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    StorageCompletion, StorageEffect, StorageEffectRequest, StorageEvent,
};
use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use silent_disco_core::transport::secure_identity::key_bound_device_id;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

fn host_key() -> Vec<u8> {
    use p256::{ecdsa::SigningKey, pkcs8::EncodePublicKey};
    SigningKey::from_slice(&[3; 32])
        .expect("fixture key")
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec()
}

fn persist(device_id: &DeviceId, operation: &str) -> StorageEffect {
    StorageEffect::new(
        OperationId::new(operation).expect("operation"),
        StorageEffectRequest::PersistTrustedDevice {
            device_id: device_id.clone(),
            display_name: "Trusted Listener".to_owned(),
        },
    )
    .expect("effect")
}

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
fn always_allow_stores_a_verified_record_before_approval_can_continue() {
    let path = temporary_database_path("success");
    let database =
        DatabaseWorker::start(DatabaseConfig::new(&path).expect("config")).expect("database");
    let client = database.client();
    let trust = Arc::new(DesktopVerifiedTrust::load(client.clone(), host_key()).expect("trust"));
    let (dispatcher, inbox) = DesktopStorageEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let runner = DesktopStorageEffectRunner::start(
        inbox,
        dispatcher.clone(),
        Arc::new(RecordingSink(sender)),
        client.clone(),
        Arc::clone(&trust),
    )
    .expect("runner");
    let device_id = key_bound_device_id(&[0xa1; 32]);
    dispatcher
        .dispatch(persist(&device_id, "storage-trust-1"))
        .expect("dispatch");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("completion");
    assert!(matches!(
        event,
        StorageEvent::OperationSucceeded {
            completion: StorageCompletion::TrustedDeviceUpdated { device_id: ref stored },
            ..
        } if stored == &device_id
    ));
    assert_eq!(trust.join_trust_state(&device_id), TrustState::Trusted);
    let listed = trust.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].display_name, "Trusted Listener");
    assert!(
        client
            .get_trusted_device(&device_id)
            .expect("query")
            .is_none(),
        "no legacy claimed-ID row is written"
    );

    runner.shutdown().expect("runner shutdown");
    drop((trust, client));
    database.stop_and_join().expect("database shutdown");
    drop(std::fs::remove_file(path));
}

#[test]
fn a_claimed_id_is_refused_and_nothing_is_stored() {
    let path = temporary_database_path("claimed");
    let database =
        DatabaseWorker::start(DatabaseConfig::new(&path).expect("config")).expect("database");
    let client = database.client();
    let trust = Arc::new(DesktopVerifiedTrust::load(client.clone(), host_key()).expect("trust"));
    let (dispatcher, inbox) = DesktopStorageEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let runner = DesktopStorageEffectRunner::start(
        inbox,
        dispatcher.clone(),
        Arc::new(RecordingSink(sender)),
        client.clone(),
        Arc::clone(&trust),
    )
    .expect("runner");
    let claimed = DeviceId::new("listener-claimed").expect("device");
    dispatcher
        .dispatch(persist(&claimed, "storage-trust-claimed"))
        .expect("dispatch");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("failure event");
    assert!(matches!(
        event,
        StorageEvent::OperationFailed { ref error, .. }
            if error.code == CoreErrorCode::InvalidArgument
                && error.message.contains("proved its key")
    ));
    assert!(trust.list().expect("list").is_empty());
    assert!(
        client
            .get_trusted_device(&claimed)
            .expect("query")
            .is_none()
    );

    runner.shutdown().expect("runner shutdown");
    drop((trust, client));
    database.stop_and_join().expect("database shutdown");
    drop(std::fs::remove_file(path));
}

#[test]
fn database_failure_is_returned_as_correlated_storage_event() {
    let path = temporary_database_path("failure");
    let database =
        DatabaseWorker::start(DatabaseConfig::new(&path).expect("config")).expect("database");
    let client = database.client();
    let trust = Arc::new(DesktopVerifiedTrust::load(client.clone(), host_key()).expect("trust"));
    database.stop_and_join().expect("stop database");

    let (dispatcher, inbox) = DesktopStorageEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let runner = DesktopStorageEffectRunner::start(
        inbox,
        dispatcher.clone(),
        Arc::new(RecordingSink(sender)),
        client,
        trust,
    )
    .expect("runner");
    dispatcher
        .dispatch(persist(
            &key_bound_device_id(&[0xb2; 32]),
            "storage-trust-failure",
        ))
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
