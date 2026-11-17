use silent_disco_ffi::{
    FfiAppRole, FfiApprovalMode, FfiBridgeError, FfiCoreHandle, FfiCoreNotification,
    FfiCoreObserver, FfiListenerLifecycle, FfiPlatformCompletion, FfiPlatformEffect,
    FfiSessionAdvertisement,
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

    fn wait_for_effect<T>(&self, mut select: impl FnMut(&FfiPlatformEffect) -> Option<T>) -> T {
        self.wait_for(|notification| match notification {
            FfiCoreNotification::PlatformEffect { effect } => select(effect),
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

fn discovered_session() -> FfiSessionAdvertisement {
    FfiSessionAdvertisement {
        session_id: "ffi-listener-session".to_owned(),
        host_device_id: "ffi-remote-host".to_owned(),
        session_name: "UniFFI listener session".to_owned(),
        approval_mode: FfiApprovalMode::Manual,
        protocol_version: 2,
        address: Some("127.0.0.1".to_owned()),
        control_port: Some(41_201),
        sync_port: Some(41_202),
        audio_port: Some(41_203),
    }
}

fn reach_connecting(
    handle: &FfiCoreHandle,
    observer: &RecordingObserver,
) -> silent_disco_ffi::FfiCoreSnapshot {
    let initial = observer.wait_for_snapshot_revision(0);
    handle
        .select_role(initial.revision, FfiAppRole::Listener)
        .expect("queue listener role selection");
    let selected = observer.wait_for_snapshot_revision(1);

    handle
        .start_discovery(selected.revision)
        .expect("queue discovery start");
    let discovering = observer.wait_for_snapshot_revision(2);
    let discovery_operation_id = observer.wait_for_effect(|effect| match effect {
        FfiPlatformEffect::StartDiscovery { operation_id, .. } => Some(operation_id.clone()),
        _ => None,
    });
    handle
        .platform_operation_succeeded(
            discovery_operation_id,
            FfiPlatformCompletion::DiscoveryStarted,
        )
        .expect("submit discovery completion");
    let scanning = observer.wait_for_snapshot_revision(discovering.revision + 1);
    assert_eq!(scanning.listener_lifecycle, FfiListenerLifecycle::Scanning);

    handle
        .submit_session_discovered(discovered_session())
        .expect("submit discovered session");
    let discovered = observer.wait_for_snapshot_revision(scanning.revision + 1);
    assert_eq!(discovered.discovered_sessions.len(), 1);

    handle
        .select_session(discovered.revision, "ffi-listener-session".to_owned())
        .expect("queue session selection");
    let session_selected = observer.wait_for_snapshot_revision(discovered.revision + 1);
    assert_eq!(
        session_selected.listener_lifecycle,
        FfiListenerLifecycle::SessionSelected
    );

    handle
        .submit_join(session_selected.revision, None)
        .expect("queue join submission");
    let joining = observer.wait_for_snapshot_revision(session_selected.revision + 1);
    assert_eq!(
        joining.listener_lifecycle,
        FfiListenerLifecycle::JoinRequested
    );
    let network_operation_id = observer.wait_for_effect(|effect| match effect {
        FfiPlatformEffect::EstablishNetwork { operation_id, .. } => Some(operation_id.clone()),
        _ => None,
    });

    handle
        .platform_operation_succeeded(
            network_operation_id,
            FfiPlatformCompletion::NetworkEndpointReady {
                address: "127.0.0.1".to_owned(),
                control_port: 41_201,
                sync_port: 41_202,
                audio_port: 41_203,
            },
        )
        .expect("submit network completion");
    let connecting = observer.wait_for_snapshot_revision(joining.revision + 1);
    assert_eq!(
        connecting.listener_lifecycle,
        FfiListenerLifecycle::Connecting
    );
    connecting
}

#[test]
fn uniffi_handle_drives_listener_join_through_approval() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-listener".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let connecting = reach_connecting(&handle, &observer);

    handle
        .submit_awaiting_approval()
        .expect("submit awaiting approval fact");
    let awaiting = observer.wait_for_snapshot_revision(connecting.revision + 1);
    assert_eq!(
        awaiting.listener_lifecycle,
        FfiListenerLifecycle::AwaitingApproval
    );

    // A secure-v4 host's "Always allow" arrives as a remembered approval. The
    // listener stores nothing from it; the transport refuses the flag from a
    // plaintext host (see listener_transport::legacy_trust_tests).
    handle
        .submit_join_approved(true)
        .expect("submit remembered join approved fact");
    let approved = observer.wait_for_snapshot_revision(awaiting.revision + 1);
    assert_eq!(approved.listener_lifecycle, FfiListenerLifecycle::Approved);
    assert!(approved.last_error.is_none());

    handle.shutdown().expect("explicit UniFFI shutdown");
}

#[test]
fn uniffi_handle_joins_a_session_with_no_known_endpoint() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-listener-wifi-direct".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let initial = observer.wait_for_snapshot_revision(0);
    handle
        .select_role(initial.revision, FfiAppRole::Listener)
        .expect("queue listener role selection");
    let selected = observer.wait_for_snapshot_revision(1);

    handle
        .start_discovery(selected.revision)
        .expect("queue discovery start");
    let discovering = observer.wait_for_snapshot_revision(2);
    let discovery_operation_id = observer.wait_for_effect(|effect| match effect {
        FfiPlatformEffect::StartDiscovery { operation_id, .. } => Some(operation_id.clone()),
        _ => None,
    });
    handle
        .platform_operation_succeeded(
            discovery_operation_id,
            FfiPlatformCompletion::DiscoveryStarted,
        )
        .expect("submit discovery completion");
    let scanning = observer.wait_for_snapshot_revision(discovering.revision + 1);

    // A Wi-Fi-Direct-discovered session has no known endpoint until the
    // platform establishment adapter actually connects.
    handle
        .submit_session_discovered(FfiSessionAdvertisement {
            session_id: "wifi-direct-session".to_owned(),
            host_device_id: "wifi-direct-host".to_owned(),
            session_name: "Wi-Fi Direct session".to_owned(),
            approval_mode: FfiApprovalMode::Manual,
            protocol_version: 2,
            address: None,
            control_port: None,
            sync_port: None,
            audio_port: None,
        })
        .expect("submit discovered session");
    let discovered = observer.wait_for_snapshot_revision(scanning.revision + 1);

    handle
        .select_session(discovered.revision, "wifi-direct-session".to_owned())
        .expect("queue session selection");
    let session_selected = observer.wait_for_snapshot_revision(discovered.revision + 1);

    handle
        .submit_join(session_selected.revision, None)
        .expect("queue join submission");
    let joining = observer.wait_for_snapshot_revision(session_selected.revision + 1);
    assert_eq!(
        joining.listener_lifecycle,
        FfiListenerLifecycle::JoinRequested
    );

    let network_operation_id = observer.wait_for_effect(|effect| match effect {
        FfiPlatformEffect::EstablishNetwork {
            operation_id,
            address,
            control_port,
            sync_port,
            audio_port,
            ..
        } => {
            assert_eq!(*address, None);
            assert_eq!(*control_port, None);
            assert_eq!(*sync_port, None);
            assert_eq!(*audio_port, None);
            Some(operation_id.clone())
        }
        _ => None,
    });

    // The platform discovers the endpoint only now, as part of establishing
    // the connection, and reports it back on completion.
    handle
        .platform_operation_succeeded(
            network_operation_id,
            FfiPlatformCompletion::NetworkEndpointReady {
                address: "192.168.49.1".to_owned(),
                control_port: 41_000,
                sync_port: 41_001,
                audio_port: 41_002,
            },
        )
        .expect("submit network completion");
    let connecting = observer.wait_for_snapshot_revision(joining.revision + 1);
    assert_eq!(
        connecting.listener_lifecycle,
        FfiListenerLifecycle::Connecting
    );

    handle.shutdown().expect("explicit UniFFI shutdown");
}

#[test]
fn uniffi_handle_surfaces_listener_join_rejection() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-listener-rejected".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let connecting = reach_connecting(&handle, &observer);

    handle
        .submit_join_rejected("session is full".to_owned())
        .expect("submit join rejected fact");
    let rejected = observer.wait_for_snapshot_revision(connecting.revision + 1);
    assert_eq!(rejected.listener_lifecycle, FfiListenerLifecycle::Error);
    assert_eq!(rejected.selected_session, None);
    let error = rejected.last_error.expect("rejection error is visible");
    assert!(error.message.contains("session is full"));

    handle.shutdown().expect("explicit UniFFI shutdown");
}

/// The `EstablishNetwork` for busy retry `attempt`: its operation and delay.
fn busy_retry_effect(observer: &RecordingObserver, attempt: u8) -> (String, u64) {
    observer.wait_for_effect(|effect| match effect {
        FfiPlatformEffect::EstablishNetwork {
            operation_id,
            busy_retry_attempt: Some(found),
            busy_retry_delay_ms: Some(delay),
            ..
        } if *found == attempt => Some((operation_id.clone(), *delay)),
        _ => None,
    })
}

/// Remediation F3 (owner decision 2026-11-09): a busy refusal is retried at
/// most 3 times, each time reconnecting after a longer delay; the fourth busy
/// refusal is final and says so. Any other refusal is final at once.
#[test]
fn a_busy_host_refusal_is_retried_three_times_then_reported() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-listener-busy".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let mut current = reach_connecting(&handle, &observer);

    for (attempt, expected_delay) in [(1_u8, 1_000_u64), (2, 2_000), (3, 4_000)] {
        handle
            .submit_join_rejected("host_busy".to_owned())
            .expect("busy refusal");
        let retrying = observer.wait_for_snapshot_revision(current.revision + 1);
        assert_eq!(
            retrying.listener_lifecycle,
            FfiListenerLifecycle::JoinRequested,
            "retry {attempt} reconnects instead of failing"
        );
        assert!(retrying.selected_session.is_some());
        // The refused connection closing while the retry waits does not end it.
        handle
            .transport_failed("connection closed by host".to_owned(), true)
            .expect("old connection closes");
        let (operation_id, delay) = busy_retry_effect(&observer, attempt);
        assert_eq!(delay, expected_delay);
        handle
            .platform_operation_succeeded(
                operation_id,
                FfiPlatformCompletion::NetworkEndpointReady {
                    address: "127.0.0.1".to_owned(),
                    control_port: 41_201,
                    sync_port: 41_202,
                    audio_port: 41_203,
                },
            )
            .expect("reconnected");
        current = observer.wait_for_snapshot_revision(retrying.revision + 1);
        assert_eq!(current.listener_lifecycle, FfiListenerLifecycle::Connecting);
    }

    handle
        .submit_join_rejected("host_busy".to_owned())
        .expect("fourth busy refusal");
    let failed = observer.wait_for_snapshot_revision(current.revision + 1);
    assert_eq!(failed.listener_lifecycle, FfiListenerLifecycle::Error);
    let error = failed.last_error.expect("visible failure");
    assert!(
        error.message.contains("busy (tried 3 more times)"),
        "{}",
        error.message
    );
    handle.shutdown().expect("explicit UniFFI shutdown");
}

#[test]
fn a_listener_approved_after_a_busy_retry_shows_no_error() {
    let observer = Arc::new(RecordingObserver::default());
    let handle = FfiCoreHandle::open("ffi-listener-busy-reset".to_owned(), observer.clone())
        .expect("open UniFFI core handle");
    let connecting = reach_connecting(&handle, &observer);
    handle
        .submit_join_rejected("host_busy".to_owned())
        .expect("busy refusal");
    let retrying = observer.wait_for_snapshot_revision(connecting.revision + 1);
    let (operation_id, _) = busy_retry_effect(&observer, 1);
    handle
        .platform_operation_succeeded(
            operation_id,
            FfiPlatformCompletion::NetworkEndpointReady {
                address: "127.0.0.1".to_owned(),
                control_port: 41_201,
                sync_port: 41_202,
                audio_port: 41_203,
            },
        )
        .expect("reconnected");
    let reconnected = observer.wait_for_snapshot_revision(retrying.revision + 1);
    handle.submit_awaiting_approval().expect("awaiting");
    let awaiting = observer.wait_for_snapshot_revision(reconnected.revision + 1);
    handle.submit_join_approved(false).expect("approved");
    let approved = observer.wait_for_snapshot_revision(awaiting.revision + 1);
    assert_eq!(approved.listener_lifecycle, FfiListenerLifecycle::Approved);
    assert!(approved.last_error.is_none());
    handle.shutdown().expect("explicit UniFFI shutdown");
}
