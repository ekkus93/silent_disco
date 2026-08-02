use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, ListenerLifecycle, SessionId, TransportState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand, CoreCommandRequest,
    CoreNotification, CoreSnapshot, NetworkEndpoint, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, RecoverableAction, SessionAdvertisement,
    SnapshotRevision, TransportEvent, current_protocol_version,
};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn awaiting_approval_then_join_approved_reaches_approved() {
    let (runtime, handle, receiver) = start_actor();
    let (connecting, _endpoint, _session_id) = join_to_connecting(&handle, &receiver);
    assert_eq!(connecting.listener_lifecycle, ListenerLifecycle::Connecting);

    handle
        .submit_transport_event(TransportEvent::AwaitingApproval)
        .expect("submit awaiting approval fact");
    let awaiting = next_snapshot(&receiver, connecting.revision.get() + 1);
    assert_eq!(
        awaiting.listener_lifecycle,
        ListenerLifecycle::AwaitingApproval
    );

    handle
        .submit_transport_event(TransportEvent::JoinApproved {
            trusted_for_future: true,
        })
        .expect("submit join approved fact");
    let approved = next_snapshot(&receiver, awaiting.revision.get() + 1);
    assert_eq!(approved.listener_lifecycle, ListenerLifecycle::Approved);
    assert!(approved.last_error.is_none());
    runtime.shutdown().expect("shutdown listener actor");
}

#[test]
fn join_approved_directly_from_connecting_skips_awaiting_approval() {
    let (runtime, handle, receiver) = start_actor();
    let (connecting, _endpoint, _session_id) = join_to_connecting(&handle, &receiver);

    handle
        .submit_transport_event(TransportEvent::JoinApproved {
            trusted_for_future: false,
        })
        .expect("submit join approved fact");
    let approved = next_snapshot(&receiver, connecting.revision.get() + 1);
    assert_eq!(approved.listener_lifecycle, ListenerLifecycle::Approved);
    runtime.shutdown().expect("shutdown listener actor");
}

#[test]
fn join_rejected_surfaces_reason_and_marks_rescan_recoverable() {
    let (runtime, handle, receiver) = start_actor();
    let (connecting, _endpoint, _session_id) = join_to_connecting(&handle, &receiver);

    handle
        .submit_transport_event(TransportEvent::JoinRejected {
            reason: "session is full".to_owned(),
        })
        .expect("submit join rejected fact");
    let rejected = next_snapshot(&receiver, connecting.revision.get() + 1);
    assert_eq!(rejected.listener_lifecycle, ListenerLifecycle::Error);
    assert_eq!(rejected.selected_session, None);
    assert_eq!(rejected.recoverable_action, Some(RecoverableAction::Rescan));
    let error = rejected.last_error.expect("rejection error is visible");
    assert!(error.message.contains("session is full"));
    runtime.shutdown().expect("shutdown listener actor");
}

#[test]
fn retry_after_join_rejection_rescans_instead_of_reconnecting() {
    let (runtime, handle, receiver) = start_actor();
    let (connecting, _endpoint, _session_id) = join_to_connecting(&handle, &receiver);

    handle
        .submit_transport_event(TransportEvent::JoinRejected {
            reason: "host rejected".to_owned(),
        })
        .expect("submit join rejected fact");
    let rejected = next_snapshot(&receiver, connecting.revision.get() + 1);

    let retrying = command_snapshot(
        &handle,
        &receiver,
        rejected.revision,
        CoreCommand::RetryRecoverableFailure,
    );
    assert!(retrying.last_error.is_none());
    let rescan_effect = next_effect(&receiver);
    assert!(
        matches!(
            rescan_effect.request,
            PlatformEffectRequest::StartDiscovery(_)
        ),
        "retry must re-scan rather than reconnect to the same endpoint"
    );

    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: rescan_effect.operation_id,
            completion: PlatformOperationCompletion::DiscoveryStarted,
        })
        .expect("submit discovery completion");
    let scanning = next_snapshot(&receiver, retrying.revision.get() + 1);
    assert_eq!(scanning.listener_lifecycle, ListenerLifecycle::Scanning);
    assert!(scanning.discovery_active);
    runtime.shutdown().expect("shutdown listener actor");
}

#[test]
fn reselecting_same_session_mid_join_is_a_noop() {
    let (runtime, handle, receiver) = start_actor();
    let (joining, session_id) = select_and_join(&handle, &receiver);
    assert_eq!(joining.transport_state, TransportState::Connecting);

    handle
        .submit_command(command(
            joining.revision,
            CoreCommand::SelectSession { session_id },
        ))
        .expect("queue same-session reselection");
    handle
        .submit_command(command(joining.revision, CoreCommand::CancelJoin))
        .expect("queue cancel immediately after the reselection");

    let cancelled = next_snapshot_expecting_no_errors(&receiver, joining.revision.get() + 1);
    assert_eq!(cancelled.revision.get(), joining.revision.get() + 1);
    let release_effect = next_effect(&receiver);
    assert!(matches!(
        release_effect.request,
        PlatformEffectRequest::ReleaseNetwork
    ));
    runtime.shutdown().expect("shutdown listener actor");
}

#[test]
fn stray_transport_failure_while_idle_is_ignored() {
    let (runtime, handle, receiver) = start_actor();
    let selected = command_snapshot(
        &handle,
        &receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Listener,
        },
    );
    assert_eq!(selected.listener_lifecycle, ListenerLifecycle::Idle);

    handle
        .submit_transport_event(TransportEvent::Failed(injected_error()))
        .expect("submit stray transport failure");
    let failed = next_snapshot(&receiver, selected.revision.get() + 1);
    assert_eq!(failed.listener_lifecycle, ListenerLifecycle::Idle);
    assert_eq!(failed.recoverable_action, None);
    assert!(failed.last_error.is_some());
    runtime.shutdown().expect("shutdown listener actor");
}

#[test]
fn transport_failure_while_scanning_transitions_to_error() {
    let (runtime, handle, receiver) = start_actor();
    let scanning = reach_scanning(&handle, &receiver);

    handle
        .submit_transport_event(TransportEvent::Failed(injected_error()))
        .expect("submit transport failure while active");
    let failed = next_snapshot(&receiver, scanning.revision.get() + 1);
    assert_eq!(failed.listener_lifecycle, ListenerLifecycle::Error);
    assert_eq!(
        failed.recoverable_action,
        Some(RecoverableAction::Reconnect)
    );
    runtime.shutdown().expect("shutdown listener actor");
}

fn injected_error() -> CoreError {
    CoreError::new(
        CoreErrorCode::TransportConnectionLost,
        "injected transport failure",
        ErrorSeverity::Error,
        true,
        None,
    )
    .expect("valid injected error")
}

/// Drives a fresh listener actor from role selection through an established
/// network connection (`ListenerLifecycle::Connecting`), mirroring
/// `actor_integration::run_listener_flow`.
fn join_to_connecting(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
) -> (CoreSnapshot, NetworkEndpoint, SessionId) {
    let (joining, session_id) = select_and_join(handle, receiver);
    let network_effect = next_effect(receiver);
    let endpoint = match network_effect.request {
        PlatformEffectRequest::EstablishNetwork(request) => request
            .endpoint
            .expect("session was discovered with a known endpoint"),
        request => panic!("unexpected join effect: {request:?}"),
    };
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: network_effect.operation_id,
            completion: PlatformOperationCompletion::NetworkEndpointReady(endpoint),
        })
        .expect("submit network completion");
    let connecting = next_snapshot(receiver, joining.revision.get() + 1);
    assert_eq!(connecting.listener_lifecycle, ListenerLifecycle::Connecting);
    assert_eq!(connecting.transport_state, TransportState::Connected);
    (connecting, endpoint, session_id)
}

#[test]
fn submit_join_succeeds_with_unknown_endpoint_and_platform_reports_it_back() {
    let (runtime, handle, receiver) = start_actor();
    let scanning = reach_scanning(&handle, &receiver);

    // A Wi-Fi-Direct-discovered session has no known IP until the platform
    // establishment adapter actually connects -- the advertisement is
    // legitimately endpoint-less at discovery time.
    let session_id = SessionId::new("wifi-direct-session").expect("valid session ID");
    let advertisement = SessionAdvertisement::new(
        session_id.clone(),
        DeviceId::new("wifi-direct-host").expect("valid host ID"),
        "Wi-Fi Direct session",
        ApprovalMode::Manual,
        current_protocol_version(),
        None,
    )
    .expect("valid advertisement");
    handle
        .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement))
        .expect("submit discovered session");
    let discovered = next_snapshot(&receiver, scanning.revision.get() + 1);

    let selected_session = command_snapshot(
        &handle,
        &receiver,
        discovered.revision,
        CoreCommand::SelectSession {
            session_id: session_id.clone(),
        },
    );

    let joining = command_snapshot(
        &handle,
        &receiver,
        selected_session.revision,
        CoreCommand::SubmitJoin { invite_code: None },
    );
    assert_eq!(joining.listener_lifecycle, ListenerLifecycle::JoinRequested);

    let network_effect = next_effect(&receiver);
    match network_effect.request {
        PlatformEffectRequest::EstablishNetwork(request) => {
            assert_eq!(request.session_id, session_id);
            assert_eq!(request.endpoint, None);
        }
        request => panic!("unexpected join effect: {request:?}"),
    }

    // The platform (Wi-Fi Direct) discovers the endpoint only now, as part of
    // establishing the connection, and reports it back on completion.
    let discovered_endpoint =
        NetworkEndpoint::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 41_000, 41_001, 41_002)
            .expect("valid endpoint");
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: network_effect.operation_id,
            completion: PlatformOperationCompletion::NetworkEndpointReady(discovered_endpoint),
        })
        .expect("submit network completion");
    let connecting = next_snapshot(&receiver, joining.revision.get() + 1);
    assert_eq!(connecting.listener_lifecycle, ListenerLifecycle::Connecting);
    assert_eq!(connecting.transport_state, TransportState::Connected);
    runtime.shutdown().expect("shutdown listener actor");
}

/// Drives a fresh listener actor from role selection through `SubmitJoin`
/// (`ListenerLifecycle::JoinRequested`), without waiting for the network
/// effect to complete.
fn select_and_join(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
) -> (CoreSnapshot, SessionId) {
    let scanning = reach_scanning(handle, receiver);

    let endpoint = NetworkEndpoint::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 41_101, 41_102, 41_103)
        .expect("valid endpoint");
    let session_id = SessionId::new("block13-session").expect("valid session ID");
    let advertisement = SessionAdvertisement::new(
        session_id.clone(),
        DeviceId::new("block13-host").expect("valid host ID"),
        "Block 13 session",
        ApprovalMode::Manual,
        current_protocol_version(),
        Some(endpoint),
    )
    .expect("valid advertisement");
    handle
        .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement))
        .expect("submit discovered session");
    let discovered = next_snapshot(receiver, scanning.revision.get() + 1);

    let selected_session = command_snapshot(
        handle,
        receiver,
        discovered.revision,
        CoreCommand::SelectSession {
            session_id: session_id.clone(),
        },
    );
    assert_eq!(
        selected_session.listener_lifecycle,
        ListenerLifecycle::SessionSelected
    );

    let joining = command_snapshot(
        handle,
        receiver,
        selected_session.revision,
        CoreCommand::SubmitJoin { invite_code: None },
    );
    assert_eq!(joining.listener_lifecycle, ListenerLifecycle::JoinRequested);
    (joining, session_id)
}

/// Drives a fresh listener actor from role selection through an active
/// discovery scan (`ListenerLifecycle::Scanning`).
fn reach_scanning(handle: &CoreActorHandle, receiver: &Receiver<CoreNotification>) -> CoreSnapshot {
    assert_eq!(next_snapshot(receiver, 0).revision.get(), 0);

    let selected = command_snapshot(
        handle,
        receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Listener,
        },
    );

    let discovering = command_snapshot(
        handle,
        receiver,
        selected.revision,
        CoreCommand::StartDiscovery,
    );
    let discovery_effect = next_effect(receiver);
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: discovery_effect.operation_id,
            completion: PlatformOperationCompletion::DiscoveryStarted,
        })
        .expect("submit discovery completion");
    let scanning = next_snapshot(receiver, discovering.revision.get() + 1);
    assert_eq!(scanning.listener_lifecycle, ListenerLifecycle::Scanning);
    scanning
}

fn start_actor() -> (
    CoreActorRuntime,
    CoreActorHandle,
    Receiver<CoreNotification>,
) {
    let (sender, receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("listener-block13-core").expect("valid device ID")),
        move |notification| {
            sender
                .send(notification)
                .expect("test receiver remains connected");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = runtime.handle();
    (runtime, handle, receiver)
}

fn command(revision: SnapshotRevision, command: CoreCommand) -> CoreCommandRequest {
    CoreCommandRequest::new(revision, command).expect("valid command request")
}

fn command_snapshot(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    revision: SnapshotRevision,
    core_command: CoreCommand,
) -> CoreSnapshot {
    handle
        .submit_command(command(revision, core_command))
        .expect("queue command");
    next_snapshot(receiver, revision.get() + 1)
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum_revision: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot))
                if snapshot.revision.get() >= minimum_revision =>
            {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("timed out waiting for snapshot revision {minimum_revision}")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("notification receiver disconnected"),
        }
    }
}

/// Like [`next_snapshot`], but fails immediately if an error notification
/// arrives first -- used to prove a command was accepted as a true no-op
/// rather than silently rejected.
fn next_snapshot_expecting_no_errors(
    receiver: &Receiver<CoreNotification>,
    minimum_revision: u64,
) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot))
                if snapshot.revision.get() >= minimum_revision =>
            {
                return snapshot;
            }
            Ok(CoreNotification::Error(error)) => {
                panic!("unexpected error notification: {error:?}")
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                panic!("timed out waiting for snapshot revision {minimum_revision}")
            }
            Err(RecvTimeoutError::Disconnected) => panic!("notification receiver disconnected"),
        }
    }
}

fn next_effect(receiver: &Receiver<CoreNotification>) -> PlatformEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for platform effect: {error}"),
        }
    }
}
