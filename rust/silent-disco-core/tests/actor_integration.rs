use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, ListenerLifecycle, MonotonicMillis,
    RequestId, SessionId, TransportState, TrustState,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorHandle, CoreActorRuntime,
    CoreCommand, CoreCommandRequest, CoreNotification, CoreSnapshot, HostDraftPatch,
    InviteCodePatch, JoinRequestSummary, NetworkEndpoint, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, SessionAdvertisement, SnapshotRevision,
    TransportEvent, current_protocol_version,
};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn public_actor_api_drives_simulated_host_and_listener_flows() {
    run_host_flow();
    run_listener_flow();
}

#[test]
fn observer_can_read_current_snapshot_without_actor_lock_deadlock() {
    let handle_slot = Arc::new(Mutex::new(None::<CoreActorHandle>));
    let observer_handle_slot = Arc::clone(&handle_slot);
    let (observed_sender, observed_receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("observer-core").expect("valid device ID")),
        move |notification| {
            if let CoreNotification::Snapshot(delivered) = notification
                && delivered.revision.get() > 0
            {
                let handle = observer_handle_slot
                    .lock()
                    .expect("observer handle slot lock")
                    .clone()
                    .expect("actor handle installed before changed snapshot");
                let current = handle.current_snapshot()?;
                observed_sender
                    .send((delivered.revision.get(), current.revision.get()))
                    .expect("observer result receiver remains connected");
            }
            Ok(())
        },
    )
    .expect("start actor runtime");
    let handle = runtime.handle();
    *handle_slot.lock().expect("install actor handle") = Some(handle.clone());

    handle
        .submit_command(command(
            0,
            CoreCommand::SelectRole { role: AppRole::Host },
        ))
        .expect("queue role selection");
    assert_eq!(
        observed_receiver
            .recv_timeout(NOTIFICATION_TIMEOUT)
            .expect("observer reads current snapshot"),
        (1, 1)
    );
    runtime.shutdown().expect("shutdown actor runtime");
}

fn run_host_flow() {
    let (runtime, receiver) = start_runtime("host-core");
    let handle = runtime.handle();
    assert_eq!(next_snapshot(&receiver, 0).revision.get(), 0);

    handle
        .submit_command(command(
            0,
            CoreCommand::SelectRole { role: AppRole::Host },
        ))
        .expect("queue host role selection");
    assert_eq!(
        next_snapshot(&receiver, 1).selected_role,
        Some(AppRole::Host)
    );

    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4_096), Some(2_000))
        .expect("valid staged source descriptor");
    handle
        .submit_command(command(
            1,
            CoreCommand::UpdateHostDraft(HostDraftPatch {
                session_name: Some("Integration host".to_owned()),
                approval_mode: Some(ApprovalMode::Manual),
                invite_code: InviteCodePatch::Unchanged,
                audio_source: AudioSourcePatch::Set(source),
                remember_approved_devices: Some(false),
            }),
        ))
        .expect("queue host draft update");
    assert_eq!(next_snapshot(&receiver, 2).revision.get(), 2);

    handle
        .submit_command(command(2, CoreCommand::CreateHostSession))
        .expect("queue host creation");
    assert_eq!(
        next_snapshot(&receiver, 3).host_lifecycle,
        HostLifecycle::CreatingSession
    );
    let advertising_effect = next_effect(&receiver);
    assert!(matches!(
        advertising_effect.request,
        PlatformEffectRequest::StartAdvertising(_)
    ));
    let advertising_operation_id = advertising_effect.operation_id.clone();

    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertising_operation_id.clone(),
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("submit advertising completion");
    let waiting = next_snapshot(&receiver, 4);
    assert_eq!(
        waiting.host_lifecycle,
        HostLifecycle::WaitingForListeners
    );
    assert_eq!(waiting.transport_state, TransportState::Advertising);

    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertising_operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("queue duplicate completion for reducer rejection");
    assert!(next_error(&receiver).message.contains("stale or duplicate"));
    assert_eq!(
        handle
            .current_snapshot()
            .expect("current snapshot after duplicate completion")
            .revision
            .get(),
        4
    );

    let join_request = JoinRequestSummary::new(
        RequestId::new("request-1").expect("valid request ID"),
        DeviceId::new("listener-1").expect("valid listener ID"),
        "Listener one",
        TrustState::SessionOnly,
        true,
        MonotonicMillis::new(100),
    )
    .expect("valid join request");
    handle
        .submit_transport_event(TransportEvent::JoinRequested(join_request))
        .expect("submit join request fact");
    assert_eq!(next_snapshot(&receiver, 5).pending_join_requests.len(), 1);
    runtime.shutdown().expect("shutdown host actor");
}

fn run_listener_flow() {
    let (runtime, receiver) = start_runtime("listener-core");
    let handle = runtime.handle();
    assert_eq!(next_snapshot(&receiver, 0).revision.get(), 0);

    handle
        .submit_command(command(
            0,
            CoreCommand::SelectRole {
                role: AppRole::Listener,
            },
        ))
        .expect("queue listener role selection");
    assert_eq!(
        next_snapshot(&receiver, 1).selected_role,
        Some(AppRole::Listener)
    );

    handle
        .submit_command(command(1, CoreCommand::StartDiscovery))
        .expect("queue discovery start");
    assert!(!next_snapshot(&receiver, 2).discovery_active);
    let discovery_effect = next_effect(&receiver);
    assert!(matches!(
        discovery_effect.request,
        PlatformEffectRequest::StartDiscovery(_)
    ));
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: discovery_effect.operation_id,
            completion: PlatformOperationCompletion::DiscoveryStarted,
        })
        .expect("submit discovery completion");
    let scanning = next_snapshot(&receiver, 3);
    assert!(scanning.discovery_active);
    assert_eq!(scanning.listener_lifecycle, ListenerLifecycle::Scanning);

    let endpoint = NetworkEndpoint::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        41_001,
        41_002,
        41_003,
    )
    .expect("valid endpoint");
    let session_id = SessionId::new("remote-session").expect("valid session ID");
    let advertisement = SessionAdvertisement::new(
        session_id.clone(),
        DeviceId::new("remote-host").expect("valid host ID"),
        "Remote session",
        ApprovalMode::Manual,
        current_protocol_version(),
        Some(endpoint),
    )
    .expect("valid advertisement");
    handle
        .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement))
        .expect("submit discovered session");
    assert_eq!(next_snapshot(&receiver, 4).discovered_sessions.len(), 1);

    handle
        .submit_command(command(
            4,
            CoreCommand::SelectSession {
                session_id: session_id.clone(),
            },
        ))
        .expect("queue session selection");
    assert_eq!(
        next_snapshot(&receiver, 5).listener_lifecycle,
        ListenerLifecycle::SessionSelected
    );
    handle
        .submit_command(command(
            5,
            CoreCommand::SubmitJoin { invite_code: None },
        ))
        .expect("queue listener join");
    let joining = next_snapshot(&receiver, 6);
    assert_eq!(joining.listener_lifecycle, ListenerLifecycle::JoinRequested);
    assert_eq!(joining.transport_state, TransportState::Connecting);
    let network_effect = next_effect(&receiver);
    assert!(matches!(
        network_effect.request,
        PlatformEffectRequest::EstablishNetwork(_)
    ));

    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: network_effect.operation_id,
            completion: PlatformOperationCompletion::NetworkEndpointReady(endpoint),
        })
        .expect("submit network completion");
    let connected = next_snapshot(&receiver, 7);
    assert_eq!(connected.listener_lifecycle, ListenerLifecycle::Connecting);
    assert_eq!(connected.transport_state, TransportState::Connected);
    runtime.shutdown().expect("shutdown listener actor");
}

fn start_runtime(device_id: &str) -> (CoreActorRuntime, Receiver<CoreNotification>) {
    let (sender, receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new(device_id).expect("valid device ID")),
        move |notification| {
            sender
                .send(notification)
                .expect("integration observer receiver remains connected");
            Ok(())
        },
    )
    .expect("start actor runtime");
    (runtime, receiver)
}

fn command(revision: u64, command: CoreCommand) -> CoreCommandRequest {
    CoreCommandRequest::new(SnapshotRevision::new(revision), command)
        .expect("valid command request")
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum_revision: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(NOTIFICATION_TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot))
                if snapshot.revision.get() >= minimum_revision =>
            {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("timed out waiting for snapshot revision {minimum_revision}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("notification observer disconnected while waiting for snapshot")
            }
        }
    }
}

fn next_effect(receiver: &Receiver<CoreNotification>) -> PlatformEffect {
    loop {
        match receiver.recv_timeout(NOTIFICATION_TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for platform effect"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("notification observer disconnected while waiting for effect")
            }
        }
    }
}

fn next_error(receiver: &Receiver<CoreNotification>) -> silent_disco_core::error::CoreError {
    loop {
        match receiver.recv_timeout(NOTIFICATION_TIMEOUT) {
            Ok(CoreNotification::Error(error)) => return error,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for core error"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("notification observer disconnected while waiting for error")
            }
        }
    }
}
