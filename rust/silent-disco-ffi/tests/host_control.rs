use silent_disco_ffi::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCoreHandle,
    FfiCoreNotification, FfiCoreObserver, FfiHostDraft, FfiHostLifecycle, FfiPlatformCompletion,
    FfiPlatformEffect,
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
    fn wait_for_snapshot_revision(&self, revision: u64) -> silent_disco_ffi::FfiCoreSnapshot {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::Snapshot { snapshot } if snapshot.revision >= revision => {
                Some(snapshot.clone())
            }
            _ => None,
        })
    }

    fn wait_for_start_advertising(&self) -> FfiPlatformEffect {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::PlatformEffect {
                effect: effect @ FfiPlatformEffect::StartAdvertising { .. },
            } => Some(effect.clone()),
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
fn uniffi_handle_drives_authoritative_host_flow_and_shutdown() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-host".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let initial = observer.wait_for_snapshot_revision(0);
    assert_eq!(initial.revision, 0);
    assert_eq!(initial.host_lifecycle, FfiHostLifecycle::Idle);

    handle
        .select_role(initial.revision, FfiAppRole::Host)
        .expect("queue role selection");
    let selected = observer.wait_for_snapshot_revision(1);
    assert_eq!(selected.selected_role, Some(FfiAppRole::Host));

    let draft = FfiHostDraft {
        session_name: "UniFFI host".to_owned(),
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
    };
    handle
        .update_host_draft(selected.revision, draft)
        .expect("queue host draft update");
    let drafted = observer.wait_for_snapshot_revision(2);
    assert_eq!(drafted.host_draft.session_name, "UniFFI host");

    handle
        .create_host_session(drafted.revision)
        .expect("queue host session creation");
    let creating = observer.wait_for_snapshot_revision(3);
    assert_eq!(creating.host_lifecycle, FfiHostLifecycle::CreatingSession);
    let effect = observer.wait_for_start_advertising();
    let operation_id = match effect {
        FfiPlatformEffect::StartAdvertising {
            operation_id,
            session_name,
            ..
        } => {
            assert_eq!(session_name, "UniFFI host");
            operation_id
        }
        other => panic!("unexpected platform effect: {other:?}"),
    };

    handle
        .platform_operation_succeeded(operation_id, FfiPlatformCompletion::AdvertisingStarted)
        .expect("submit advertising completion");
    let waiting = observer.wait_for_snapshot_revision(4);
    assert_eq!(
        waiting.host_lifecycle,
        FfiHostLifecycle::WaitingForListeners
    );

    handle.shutdown().expect("explicit UniFFI shutdown");
    assert!(matches!(
        handle.current_snapshot(),
        Err(FfiBridgeError::Closed(_))
    ));
}
