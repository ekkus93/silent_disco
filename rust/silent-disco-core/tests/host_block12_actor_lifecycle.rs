use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, PlaybackState, TransportState, TrustState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorHandle,
    CoreActorRuntime, CoreCommand, CoreCommandRequest, CoreNotification, CoreSnapshot,
    HostDraftPatch, InviteCodePatch, ListenerSummary, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, SnapshotRevision, TransportEvent,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn host_lifecycle_transitions_are_rust_authoritative() {
    let (runtime, handle, receiver) = start_actor();
    let selected = command_snapshot(
        &handle,
        &receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4096), Some(2000))
        .expect("valid audio source");
    let drafted = command_snapshot(
        &handle,
        &receiver,
        selected.revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Lifecycle host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    let creating = command_snapshot(
        &handle,
        &receiver,
        drafted.revision,
        CoreCommand::CreateHostSession,
    );
    assert_eq!(creating.host_lifecycle, HostLifecycle::CreatingSession);
    let start_advertising = next_platform_effect(&receiver);
    assert!(matches!(
        start_advertising.request,
        PlatformEffectRequest::StartAdvertising(_)
    ));

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

    let listener = ListenerSummary::new(
        DeviceId::new("listener-1").expect("valid listener ID"),
        "Listener One",
        TrustState::SessionOnly,
        TransportState::Connected,
    )
    .expect("valid listener summary");
    handle
        .submit_transport_event(TransportEvent::ListenerConnected(listener))
        .expect("submit listener connection");
    let ready = next_snapshot(&receiver, waiting.revision.get() + 1);
    assert_eq!(ready.host_lifecycle, HostLifecycle::Ready);

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
        ready.revision
    );

    handle
        .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
        .expect("submit playing state");
    let streaming = next_snapshot(&receiver, ready.revision.get() + 1);
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
fn host_failure_and_retry_are_visible() {
    let (runtime, handle, receiver) = start_waiting_host();
    let waiting = handle.current_snapshot().expect("waiting snapshot");
    let transport_error = CoreError::new(
        CoreErrorCode::TransportConnectionLost,
        "injected host transport failure",
        ErrorSeverity::Error,
        true,
        None,
    )
    .expect("valid transport error");
    handle
        .submit_transport_event(TransportEvent::Failed(transport_error))
        .expect("submit transport failure");
    let failed = next_snapshot(&receiver, waiting.revision.get() + 1);
    assert_eq!(failed.host_lifecycle, HostLifecycle::Error);
    assert!(failed.last_error.is_some());

    let retried = command_snapshot(
        &handle,
        &receiver,
        failed.revision,
        CoreCommand::RetryRecoverableFailure,
    );
    assert_eq!(retried.host_lifecycle, HostLifecycle::Error);
    assert!(retried.last_error.is_none());
    runtime.shutdown().expect("shutdown actor");
}

fn start_waiting_host() -> (CoreActorRuntime, CoreActorHandle, Receiver<CoreNotification>) {
    let (runtime, handle, receiver) = start_actor();
    let selected = command_snapshot(
        &handle,
        &receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    let source = AudioSourceDescriptor::new("source-2", "fixture.wav", Some(4096), Some(2000))
        .expect("valid audio source");
    let drafted = command_snapshot(
        &handle,
        &receiver,
        selected.revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Failure host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    let creating = command_snapshot(
        &handle,
        &receiver,
        drafted.revision,
        CoreCommand::CreateHostSession,
    );
    let effect = next_platform_effect(&receiver);
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: effect.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("submit advertising success");
    let waiting = next_snapshot(&receiver, creating.revision.get() + 1);
    assert_eq!(waiting.host_lifecycle, HostLifecycle::WaitingForListeners);
    (runtime, handle, receiver)
}

fn start_actor() -> (CoreActorRuntime, CoreActorHandle, Receiver<CoreNotification>) {
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
        .submit_command(
            CoreCommandRequest::new(revision, command).expect("valid command request"),
        )
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
