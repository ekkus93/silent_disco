//! Remediation F1 through the real Rust actor, as the Android host drives it:
//! approved-devices admission from the host's verified records, and "Always
//! allow" storing the record before the approval is delivered. These run the
//! same FFI calls `HostSessionController` makes, so a refusal at any step
//! (as `submit_listener_connected` once did) fails here.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
use silent_disco_core::transport::secure_identity::key_bound_device_id;

use super::handle::FfiCoreHandle;
use super::handle_trust::{enroll_verified_listener, load_host_trust};
use super::types::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCoreNotification,
    FfiCoreObserver, FfiCoreSnapshot, FfiDeliveryReport, FfiHostDraft, FfiJoinRequestInput,
    FfiListenerSummary, FfiPlatformCompletion, FfiPlatformEffect, FfiStorageEffect,
    FfiTransportEffect, FfiTransportState, FfiTrustState,
};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Recorder {
    notifications: Mutex<Vec<FfiCoreNotification>>,
    available: Condvar,
}

impl Recorder {
    fn wait_for<T>(&self, mut select: impl FnMut(&FfiCoreNotification) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        let mut notifications = self.notifications.lock().expect("recording lock");
        loop {
            if let Some(value) = notifications.iter().find_map(&mut select) {
                return value;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for notification");
            notifications = self
                .available
                .wait_timeout(notifications, remaining)
                .expect("notification wait")
                .0;
        }
    }

    fn snapshot_where(&self, test: impl Fn(&FfiCoreSnapshot) -> bool) -> FfiCoreSnapshot {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::Snapshot { snapshot } if test(snapshot) => Some(snapshot.clone()),
            _ => None,
        })
    }

    /// The transport effect for `request_id`: `Ok((operation, trusted_for_future))`
    /// for an approval, `Err(reason_code)` for a rejection.
    fn decision(&self, request_id: &str) -> Result<(String, bool), String> {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::TransportEffect {
                effect:
                    FfiTransportEffect::DeliverJoinApproval {
                        operation_id,
                        request_id: id,
                        trusted_for_future,
                        ..
                    },
            } if id == request_id => Some(Ok((operation_id.clone(), *trusted_for_future))),
            FfiCoreNotification::TransportEffect {
                effect:
                    FfiTransportEffect::DeliverJoinRejection {
                        request_id: id,
                        reason_code,
                        ..
                    },
            } if id == request_id => Some(Err(reason_code.clone())),
            _ => None,
        })
    }

    fn remember_effect(&self, device_id: &str) -> String {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::StorageEffect {
                effect:
                    FfiStorageEffect::PersistTrustedDevice {
                        operation_id,
                        device_id: id,
                        ..
                    },
            } if id == device_id => Some(operation_id.clone()),
            _ => None,
        })
    }
}

impl FfiCoreObserver for Recorder {
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        self.notifications
            .lock()
            .expect("recording lock")
            .push(notification);
        self.available.notify_all();
        Ok(())
    }
}

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

/// Opens a core handle hosting in `mode`, with advertising completed.
fn hosting(name: &str, mode: FfiApprovalMode) -> (Arc<Recorder>, Arc<FfiCoreHandle>) {
    let recorder = Arc::new(Recorder::default());
    let handle = FfiCoreHandle::open(name.to_owned(), recorder.clone()).expect("open core");
    let initial = recorder.snapshot_where(|_| true);
    handle
        .select_role(initial.revision, FfiAppRole::Host)
        .expect("host role");
    let selected = recorder.snapshot_where(|snapshot| snapshot.revision > initial.revision);
    handle
        .update_host_draft(
            selected.revision,
            FfiHostDraft {
                session_name: "Trust host".to_owned(),
                approval_mode: mode,
                invite_code: None,
                audio_source: Some(FfiAudioSource {
                    source_id: "source-1".to_owned(),
                    display_name: "fixture.wav".to_owned(),
                    size_bytes: Some(4_096),
                    duration_ms: Some(2_000),
                }),
                remember_approved_devices: false,
                tuning: selected.host_draft.tuning,
            },
        )
        .expect("draft");
    let drafted = recorder.snapshot_where(|snapshot| snapshot.revision > selected.revision);
    handle
        .create_host_session(drafted.revision)
        .expect("create session");
    let operation_id = recorder.wait_for(|notification| match notification {
        FfiCoreNotification::PlatformEffect {
            effect: FfiPlatformEffect::StartAdvertising { operation_id, .. },
        } => Some(operation_id.clone()),
        _ => None,
    });
    handle
        .platform_operation_succeeded(operation_id, FfiPlatformCompletion::AdvertisingStarted)
        .expect("advertising");
    (recorder, handle)
}

fn join(handle: &FfiCoreHandle, request_id: &str, device_id: &str) {
    handle
        .submit_join_request(FfiJoinRequestInput {
            request_id: request_id.to_owned(),
            device_id: device_id.to_owned(),
            display_name: "Listener".to_owned(),
            invite_code: None,
            received_at_ms: 100,
        })
        .expect("join reaches the actor");
}

/// What the Android host does once an approval reached the listener.
fn deliver_and_connect(
    recorder: &Recorder,
    handle: &FfiCoreHandle,
    operation_id: String,
    device_id: &str,
    trusted_for_future: bool,
) {
    handle
        .transport_delivery_completed(
            operation_id,
            FfiDeliveryReport {
                intended_peers: 1,
                successful_peers: 1,
                failed_peers: 0,
            },
        )
        .expect("delivery reported");
    handle
        .submit_listener_connected(FfiListenerSummary {
            device_id: device_id.to_owned(),
            display_name: "Listener".to_owned(),
            trust_state: if trusted_for_future {
                FfiTrustState::Trusted
            } else {
                FfiTrustState::SessionOnly
            },
            transport_state: FfiTransportState::Connected,
            synchronization: None,
            last_contact_ms: None,
            last_error: None,
        })
        .expect("the connected listener is accepted");
    recorder.snapshot_where(|snapshot| {
        snapshot
            .listeners
            .iter()
            .any(|listener| listener.device_id == device_id)
    });
}

fn database(tag: &str) -> (std::path::PathBuf, DatabaseWorker) {
    let directory = std::env::temp_dir().join(format!(
        "sd-ffi-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).expect("temp dir");
    let worker = DatabaseWorker::start(
        DatabaseConfig::new(directory.join("trust.sqlite3")).expect("config"),
    )
    .expect("database worker");
    (directory, worker)
}

#[test]
fn approved_devices_admits_only_a_remembered_unrevoked_key() {
    let (directory, worker) = database("approved-only-flow");
    let client = worker.client();
    let host = host_key();
    let remembered = key_bound_device_id(&[0xa1; 32]);
    let stranger = key_bound_device_id(&[0xb2; 32]);
    silent_disco_core::transport::secure_trust_store::enroll_listener(
        &client,
        &host,
        &remembered,
        "Kitchen",
        1,
    )
    .expect("remembered earlier");

    let (recorder, handle) = hosting("approved-only-flow", FfiApprovalMode::TrustedDevices);
    load_host_trust(&handle, &client, &host).expect("host start loads trust");

    join(&handle, "remembered-1", remembered.as_str());
    let (operation, trusted) = recorder
        .decision("remembered-1")
        .expect("a remembered key is admitted without asking");
    assert!(trusted, "admitted as remembered");
    deliver_and_connect(&recorder, &handle, operation, remembered.as_str(), true);
    let snapshot = recorder.snapshot_where(|snapshot| !snapshot.listeners.is_empty());
    assert_eq!(snapshot.listeners[0].trust_state, FfiTrustState::Trusted);

    join(&handle, "stranger-1", stranger.as_str());
    assert!(
        recorder.decision("stranger-1").is_err(),
        "an unknown key is turned away"
    );
    join(&handle, "claimed-1", "claimed-listener");
    assert!(
        recorder.decision("claimed-1").is_err(),
        "a claimed ID is turned away"
    );

    crate::verified_listeners::revoke_verified_listener(&client, &host, remembered.as_str(), 2)
        .expect("revoked in Settings");
    join(&handle, "remembered-2", remembered.as_str());
    assert!(
        recorder.decision("remembered-2").is_err(),
        "a revoked key is turned away, even in this running session"
    );

    handle.shutdown().expect("shutdown");
    drop(client);
    worker.stop_and_join().expect("database closes");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn always_allow_stores_the_record_then_admits_it_after_a_host_restart() {
    let (directory, worker) = database("always-allow-flow");
    let client = worker.client();
    let host = host_key();
    let listener = key_bound_device_id(&[0xc3; 32]);

    let (recorder, handle) = hosting("always-allow-flow", FfiApprovalMode::Manual);
    load_host_trust(&handle, &client, &host).expect("host start loads trust");
    join(&handle, "first-1", listener.as_str());
    let pending = recorder.snapshot_where(|snapshot| !snapshot.pending_join_requests.is_empty());
    handle
        .approve_join(pending.revision, "first-1".to_owned(), true)
        .expect("Always allow");

    // The record is written before the approval is delivered.
    let storage = recorder.remember_effect(listener.as_str());
    enroll_verified_listener(&handle, &client, &host, listener.as_str(), "Listener", 5)
        .expect("the verified record is stored");
    handle
        .trusted_device_updated(storage, listener.as_str().to_owned())
        .expect("storage completion");
    let (operation, trusted) = recorder
        .decision("first-1")
        .expect("approved after the record is stored");
    assert!(trusted, "delivered as remembered");
    deliver_and_connect(&recorder, &handle, operation, listener.as_str(), true);
    handle.shutdown().expect("shutdown");

    // A restarted host in approved-devices mode admits it without asking.
    let (recorder, handle) = hosting("always-allow-restart", FfiApprovalMode::TrustedDevices);
    load_host_trust(&handle, &client, &host).expect("restart loads trust");
    join(&handle, "again-1", listener.as_str());
    let (operation, trusted) = recorder
        .decision("again-1")
        .expect("remembered across a restart");
    assert!(trusted);
    deliver_and_connect(&recorder, &handle, operation, listener.as_str(), true);

    handle.shutdown().expect("shutdown");
    drop(client);
    worker.stop_and_join().expect("database closes");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn a_remembered_listener_reconnects_and_a_new_key_is_a_new_device() {
    let (directory, worker) = database("reconnect-flow");
    let client = worker.client();
    let host = host_key();
    let phone = key_bound_device_id(&[0xd4; 32]);
    // The same phone after losing its key (reinstall, data cleared): a new
    // key, so a new key-bound ID (owner decision: a changed key is a new device).
    let reinstalled = key_bound_device_id(&[0xe5; 32]);
    silent_disco_core::transport::secure_trust_store::enroll_listener(
        &client, &host, &phone, "Phone", 1,
    )
    .expect("remembered earlier");

    let (recorder, handle) = hosting("reconnect-flow", FfiApprovalMode::TrustedDevices);
    load_host_trust(&handle, &client, &host).expect("host start loads trust");
    join(&handle, "first-1", phone.as_str());
    let (operation, _) = recorder.decision("first-1").expect("admitted");
    deliver_and_connect(&recorder, &handle, operation, phone.as_str(), true);

    // The connection drops and the phone joins again with the same key.
    handle
        .submit_listener_disconnected(phone.as_str().to_owned(), None)
        .expect("disconnect reported");
    recorder.snapshot_where(|snapshot| {
        snapshot.listeners.iter().all(|listener| {
            listener.device_id != phone.as_str()
                || listener.transport_state != FfiTransportState::Connected
        })
    });
    join(&handle, "rejoin-1", phone.as_str());
    let (operation, trusted) = recorder
        .decision("rejoin-1")
        .expect("a reconnect is admitted without asking");
    assert!(trusted);
    deliver_and_connect(&recorder, &handle, operation, phone.as_str(), true);

    // After losing its key the phone is unknown until approved again.
    join(&handle, "reinstalled-1", reinstalled.as_str());
    assert!(
        recorder.decision("reinstalled-1").is_err(),
        "a new key is not trusted through the old record"
    );
    handle.shutdown().expect("shutdown");

    // Re-enrolling it in manual mode with Always allow trusts the new key;
    // the old key stays trusted until it is removed in Settings.
    let (recorder, handle) = hosting("reenrol-flow", FfiApprovalMode::Manual);
    load_host_trust(&handle, &client, &host).expect("host start loads trust");
    join(&handle, "reenrol-1", reinstalled.as_str());
    let pending = recorder.snapshot_where(|snapshot| !snapshot.pending_join_requests.is_empty());
    handle
        .approve_join(pending.revision, "reenrol-1".to_owned(), true)
        .expect("Always allow");
    let storage = recorder.remember_effect(reinstalled.as_str());
    enroll_verified_listener(&handle, &client, &host, reinstalled.as_str(), "Phone", 5)
        .expect("stored");
    handle
        .trusted_device_updated(storage, reinstalled.as_str().to_owned())
        .expect("storage completion");
    let (operation, trusted) = recorder.decision("reenrol-1").expect("approved");
    assert!(trusted);
    deliver_and_connect(&recorder, &handle, operation, reinstalled.as_str(), true);
    let remembered: Vec<String> =
        crate::verified_listeners::list_verified_listeners(&client, &host)
            .expect("list")
            .into_iter()
            .map(|listener| listener.device_id)
            .collect();
    assert_eq!(
        remembered,
        vec![phone.as_str().to_owned(), reinstalled.as_str().to_owned()],
        "Settings shows both keys, so the old one can be removed"
    );

    handle.shutdown().expect("shutdown");
    drop(client);
    worker.stop_and_join().expect("database closes");
    let _ = std::fs::remove_dir_all(directory);
}

/// Owner decision 2026-11-18 (remediation TODO §1): revoking a listener that is
/// connected to the running host disconnects it at once, through the same
/// revoke the Android Settings screen calls. A revoke of a listener that is
/// not connected changes nothing in the session.
#[test]
fn revoking_a_connected_listener_disconnects_it_from_the_running_host() {
    let (directory, worker) = database("live-revoke-flow");
    let client = worker.client();
    let host = host_key();
    let remembered = key_bound_device_id(&[0xc3; 32]);
    let absent = key_bound_device_id(&[0xd4; 32]);
    for (device, name) in [(&remembered, "Patio"), (&absent, "Hallway")] {
        silent_disco_core::transport::secure_trust_store::enroll_listener(
            &client, &host, device, name, 1,
        )
        .expect("remembered earlier");
    }

    let (recorder, handle) = hosting("live-revoke-flow", FfiApprovalMode::TrustedDevices);
    load_host_trust(&handle, &client, &host).expect("host start loads trust");
    join(&handle, "patio-1", remembered.as_str());
    let (operation, _) = recorder.decision("patio-1").expect("admitted");
    deliver_and_connect(&recorder, &handle, operation, remembered.as_str(), true);

    // Not connected: nothing to disconnect.
    crate::verified_listeners::revoke_verified_listener(&client, &host, absent.as_str(), 2)
        .expect("revoked an absent listener");
    // Connected: disconnected now, with its own reason.
    crate::verified_listeners::revoke_verified_listener(&client, &host, remembered.as_str(), 3)
        .expect("revoked the connected listener");
    let (listener, reason) = recorder.wait_for(|notification| match notification {
        FfiCoreNotification::TransportEffect {
            effect:
                FfiTransportEffect::DisconnectListener {
                    listener_id,
                    reason_code,
                    ..
                },
        } => Some((listener_id.clone(), reason_code.clone())),
        _ => None,
    });
    assert_eq!(
        listener,
        remembered.as_str(),
        "only the connected, revoked listener"
    );
    assert_eq!(reason, "host_revoked_listener");

    handle.shutdown().expect("shutdown");
    drop(client);
    worker.stop_and_join().expect("database closes");
    let _ = std::fs::remove_dir_all(directory);
}
