use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, PlaybackState, TransportState, TrustState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorHandle,
    CoreActorRuntime, CoreCommand, CoreCommandRequest, CoreNotification, CoreSnapshot,
    HostDraftPatch, InviteCodePatch, ListenerSummary, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, RecoverableAction, SnapshotRevision,
    TransportEvent,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn host_lifecycle_transitions_are_rust_authoritative() {
    let (runtime, handle, receiver) = start_actor();
    let (creating, start_advertising) = create_host(&handle, &receiver, "Lifecycle host");
    assert_eq!(creating.host_lifecycle, HostLifecycle::CreatingSession);

    handle
        .submit_transport_event(TransportEvent::StateChanged(TransportState::Advertising))
        .expect("submit advertising state");
    let advertising = next_snapshot(&receiver, creating.revision.get() + 1);
    assert_eq!(advertising.host_lifecycle, HostLifecycle::Advertising);
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: start_advertising.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("submit advertising success");
    let waiting = next_snapshot(&receiver, advertising.revision.get() + 1);
    assert_eq!(waiting.host_lifecycle, HostLifecycle::WaitingForListeners);

    let listener = listener_summary();
    handle
        .submit_transport_event(TransportEvent::ListenerConnected(listener.clone()))
        .expect("submit listener connection");
    let ready = next_snapshot(&receiver, waiting.revision.get() + 1);
    assert_eq!(ready.host_lifecycle, HostLifecycle::Ready);

    handle
        .submit_transport_event(TransportEvent::ListenerDisconnected {
            device_id: listener.device_id.clone(),
            error: None,
        })
        .expect("submit listener disconnect");
    let waiting_again = next_snapshot(&receiver, ready.revision.get() + 1);
    assert_eq!(
        waiting_again.host_lifecycle,
        HostLifecycle::WaitingForListeners
    );
    handle
        .submit_transport_event(TransportEvent::ListenerConnected(listener))
        .expect("submit listener reconnect");
    let ready_again = next_snapshot(&receiver, waiting_again.revision.get() + 1);
    assert_eq!(ready_again.host_lifecycle, HostLifecycle::Ready);

    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
        .expect("submit illegal pause");
    let illegal_pause = next_error(&receiver);
    assert_eq!(illegal_pause.code, CoreErrorCode::InvalidStateTransition);
    assert_eq!(
        handle
            .current_snapshot()
            .expect("snapshot after rejected pause")
            .revision,
        ready_again.revision
    );

    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("submit playing state");
    let streaming = next_snapshot(&receiver, ready_again.revision.get() + 1);
    assert_eq!(streaming.host_lifecycle, HostLifecycle::Streaming);
    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
        .expect("submit paused state");
    let paused = next_snapshot(&receiver, streaming.revision.get() + 1);
    assert_eq!(paused.host_lifecycle, HostLifecycle::Paused);
    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
        .expect("submit stopped state");
    let stopped = next_snapshot(&receiver, paused.revision.get() + 1);
    assert_eq!(stopped.host_lifecycle, HostLifecycle::Ready);

    let ending = command_snapshot(
        &handle,
        &receiver,
        stopped.revision,
        CoreCommand::EndHostSession,
    );
    assert_eq!(ending.host_lifecycle, HostLifecycle::EndingSession);
    let stop_advertising = next_platform_effect(&receiver);
    assert!(matches!(
        stop_advertising.request,
        PlatformEffectRequest::StopAdvertising
    ));
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: stop_advertising.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStopped,
        })
        .expect("submit advertising stop");
    let idle = next_snapshot(&receiver, ending.revision.get() + 1);
    assert_eq!(idle.host_lifecycle, HostLifecycle::Idle);
    runtime.shutdown().expect("shutdown actor");
}

#[test]
fn retryable_host_start_failure_reissues_the_real_effect() {
    let (runtime, handle, receiver) = start_actor();
    let (creating, first_effect) = create_host(&handle, &receiver, "Retry host");
    let first_session_id = match &first_effect.request {
        PlatformEffectRequest::StartAdvertising(advertisement) => advertisement.session_id.clone(),
        request => panic!("unexpected host start effect: {request:?}"),
    };
    let failure = CoreError::new(
        CoreErrorCode::PlatformOperationFailed,
        "injected advertising failure",
        ErrorSeverity::Error,
        true,
        Some(first_effect.operation_id.clone()),
    )
    .expect("valid platform error");
    handle
        .submit_platform_event(PlatformEvent::OperationFailed {
            operation_id: first_effect.operation_id,
            error: failure,
        })
        .expect("submit advertising failure");
    let failed = next_snapshot(&receiver, creating.revision.get() + 1);
    assert_eq!(failed.host_lifecycle, HostLifecycle::Error);
    assert_eq!(failed.transport_state, TransportState::Failed);
    assert_eq!(failed.recoverable_action, Some(RecoverableAction::Retry));
    assert!(failed.last_error.is_some());

    let retrying = command_snapshot(
        &handle,
        &receiver,
        failed.revision,
        CoreCommand::RetryRecoverableFailure,
    );
    assert_eq!(retrying.host_lifecycle, HostLifecycle::CreatingSession);
    assert_eq!(retrying.transport_state, TransportState::Idle);
    assert!(retrying.last_error.is_none());
    let retry_effect = next_platform_effect(&receiver);
    let retry_session_id = match retry_effect.request {
        PlatformEffectRequest::StartAdvertising(advertisement) => advertisement.session_id,
        request => panic!("unexpected retry effect: {request:?}"),
    };
    assert_eq!(retry_session_id, first_session_id);
    runtime.shutdown().expect("shutdown actor");
}

fn create_host(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    session_name: &str,
) -> (CoreSnapshot, PlatformEffect) {
    let selected = command_snapshot(
        handle,
        receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4096), Some(2000))
        .expect("valid audio source");
    let drafted = command_snapshot(
        handle,
        receiver,
        selected.revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some(session_name.to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    let creating = command_snapshot(
        handle,
        receiver,
        drafted.revision,
        CoreCommand::CreateHostSession,
    );
    let effect = next_platform_effect(receiver);
    assert!(matches!(
        &effect.request,
        PlatformEffectRequest::StartAdvertising(_)
    ));
    (creating, effect)
}

fn listener_summary() -> ListenerSummary {
    ListenerSummary::new(
        DeviceId::new("listener-1").expect("valid listener ID"),
        "Listener One",
        TrustState::SessionOnly,
        TransportState::Connected,
    )
    .expect("valid listener summary")
}

fn start_actor() -> (
    CoreActorRuntime,
    CoreActorHandle,
    Receiver<CoreNotification>,
) {
    let (sender, receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("host-core").expect("valid host ID")),
        move |notification| {
            sender
                .send(notification)
                .expect("test receiver remains connected");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = runtime.handle();
    assert_eq!(next_snapshot(&receiver, 0).revision.get(), 0);
    (runtime, handle, receiver)
}

fn command_snapshot(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    revision: SnapshotRevision,
    command: CoreCommand,
) -> CoreSnapshot {
    handle
        .submit_command(CoreCommandRequest::new(revision, command).expect("valid command request"))
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

fn next_platform_effect(receiver: &Receiver<CoreNotification>) -> PlatformEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for platform effect: {error}"),
        }
    }
}

fn next_error(receiver: &Receiver<CoreNotification>) -> CoreError {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Error(error)) => return error,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for error: {error}"),
        }
    }
}
