use silent_disco_ffi::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCoreHandle,
    FfiCoreNotification, FfiCoreObserver, FfiDeliveryReport, FfiHostDraft, FfiJoinRequestInput,
    FfiPlatformCompletion, FfiPlatformEffect, FfiTransportEffect, FfiTrustState,
};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct RecordingObserver {
    notifications: Mutex<Vec<FfiCoreNotification>>,
    available: Condvar,
}

impl RecordingObserver {
    fn wait_for_snapshot(&self, revision: u64) -> silent_disco_ffi::FfiCoreSnapshot {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::Snapshot { snapshot } if snapshot.revision >= revision => {
                Some(snapshot.clone())
            }
            _ => None,
        })
    }

    fn wait_for_platform_effect(&self) -> FfiPlatformEffect {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::PlatformEffect { effect } => Some(effect.clone()),
            _ => None,
        })
    }

    fn wait_for_transport_effect(&self) -> FfiTransportEffect {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::TransportEffect { effect } => Some(effect.clone()),
            _ => None,
        })
    }

    fn wait_for<T>(&self, mut select: impl FnMut(&FfiCoreNotification) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        let mut notifications = self.notifications.lock().expect("recording lock");
        loop {
            if let Some(value) = notifications.iter().find_map(&mut select) {
                return value;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for notification");
            let (next, wait) = self
                .available
                .wait_timeout(notifications, remaining)
                .expect("notification wait");
            notifications = next;
            assert!(!wait.timed_out(), "timed out waiting for notification");
        }
    }
}

impl FfiCoreObserver for RecordingObserver {
    fn on_notification(&self, notification: FfiCoreNotification) -> Result<(), FfiBridgeError> {
        self.notifications
            .lock()
            .expect("recording lock")
            .push(notification);
        self.available.notify_all();
        Ok(())
    }
}

#[test]
fn uniffi_admission_reaches_delivery_first_actor_handlers() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-host-admission".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let initial = observer.wait_for_snapshot(0);

    handle
        .select_role(initial.revision, FfiAppRole::Host)
        .expect("select host role");
    let selected = observer.wait_for_snapshot(1);
    handle
        .update_host_draft(
            selected.revision,
            FfiHostDraft {
                session_name: "Admission host".to_owned(),
                approval_mode: FfiApprovalMode::Manual,
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
        .expect("update host draft");
    let drafted = observer.wait_for_snapshot(2);
    handle
        .create_host_session(drafted.revision)
        .expect("create host session");
    let _creating = observer.wait_for_snapshot(3);
    let advertising = observer.wait_for_platform_effect();
    let operation_id = match advertising {
        FfiPlatformEffect::StartAdvertising { operation_id, .. } => operation_id,
        other => panic!("unexpected platform effect: {other:?}"),
    };
    handle
        .platform_operation_succeeded(operation_id, FfiPlatformCompletion::AdvertisingStarted)
        .expect("complete advertising");
    let waiting = observer.wait_for_snapshot(4);

    handle
        .submit_join_request(FfiJoinRequestInput {
            request_id: "request-ffi-1".to_owned(),
            device_id: "listener-ffi-1".to_owned(),
            display_name: "Listener FFI".to_owned(),
            trust_state: FfiTrustState::SessionOnly,
            invite_code: None,
            received_at_ms: 100,
        })
        .expect("submit join request");
    let pending = observer.wait_for_snapshot(waiting.revision + 1);
    assert_eq!(pending.pending_join_requests.len(), 1);

    handle
        .approve_join(pending.revision, "request-ffi-1".to_owned(), false)
        .expect("approve join request");
    let delivering = observer.wait_for_snapshot(pending.revision + 1);
    assert_eq!(delivering.pending_join_requests.len(), 1);
    assert!(delivering.listeners.is_empty());
    let delivery = observer.wait_for_transport_effect();
    let operation_id = match delivery {
        FfiTransportEffect::DeliverJoinApproval {
            operation_id,
            request_id,
            listener_id,
            ..
        } => {
            assert_eq!(request_id, "request-ffi-1");
            assert_eq!(listener_id, "listener-ffi-1");
            operation_id
        }
        other => panic!("unexpected transport effect: {other:?}"),
    };

    handle
        .transport_delivery_completed(
            operation_id,
            FfiDeliveryReport {
                intended_peers: 1,
                successful_peers: 1,
                failed_peers: 0,
            },
        )
        .expect("complete targeted approval delivery");
    let committed = observer.wait_for_snapshot(delivering.revision + 1);
    assert!(committed.pending_join_requests.is_empty());
    assert_eq!(committed.listeners.len(), 1);
    assert_eq!(committed.listeners[0].device_id, "listener-ffi-1");

    handle.shutdown().expect("shutdown UniFFI handle");
}
