use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, MonotonicMillis, RequestId, TrustState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorHandle, CoreActorRuntime,
    CoreCommand, CoreCommandRequest, CoreNotification, CoreSnapshot, DeliveryReport, HostDraftPatch,
    InviteCodePatch, JoinRequestSummary, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion, SnapshotRevision, StorageEffect, StorageEvent, TransportEffect,
    TransportEffectRequest, TransportEvent,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn manual_admission_is_deduplicated_and_delivery_first() {
    let (runtime, handle, receiver) = start_host(ApprovalMode::Manual, false);
    let request = join_request("request-1", "listener-1", TrustState::SessionOnly, true);
    handle
        .submit_transport_event(TransportEvent::JoinRequested(request.clone()))
        .expect("submit join request");
    let pending = next_snapshot(&receiver, 5);
    assert_eq!(pending.pending_join_requests.len(), 1);

    handle
        .submit_transport_event(TransportEvent::JoinRequested(request.clone()))
        .expect("submit exact duplicate");
    submit_command(
        &handle,
        pending.revision,
        CoreCommand::ApproveJoin {
            request_id: request.request_id.clone(),
        },
    );
    let awaiting_delivery = next_snapshot(&receiver, pending.revision.get() + 1);
    assert_eq!(awaiting_delivery.pending_join_requests.len(), 1);
    let first_effect = next_transport_effect(&receiver);
    assert!(matches!(
        first_effect.request,
        TransportEffectRequest::DeliverJoinApproval {
            trusted_for_future: false,
            ..
        }
    ));

    handle
        .submit_transport_event(TransportEvent::DeliveryCompleted {
            operation_id: first_effect.operation_id,
            report: DeliveryReport::new(0, 0, 0).expect("zero-recipient report"),
        })
        .expect("submit zero-recipient completion");
    let failed_delivery = next_snapshot(&receiver, awaiting_delivery.revision.get() + 1);
    assert_eq!(failed_delivery.pending_join_requests.len(), 1);
    assert_eq!(
        failed_delivery
            .last_error
            .as_ref()
            .expect("visible delivery failure")
            .code,
        CoreErrorCode::TransportDeliveryFailed
    );

    submit_command(
        &handle,
        failed_delivery.revision,
        CoreCommand::ApproveJoin {
            request_id: request.request_id.clone(),
        },
    );
    let retrying = next_snapshot(&receiver, failed_delivery.revision.get() + 1);
    let retry_effect = next_transport_effect(&receiver);
    handle
        .submit_transport_event(TransportEvent::DeliveryCompleted {
            operation_id: retry_effect.operation_id,
            report: DeliveryReport::new(2, 1, 1).expect("partial delivery report"),
        })
        .expect("submit partial completion");
    let committed = next_snapshot(&receiver, retrying.revision.get() + 1);
    assert!(committed.pending_join_requests.is_empty());
    assert_eq!(
        committed
            .last_delivery
            .expect("delivery report")
            .successful_peers,
        1
    );
    assert_eq!(next_diagnostic(&receiver).name, "transport_delivery_partial");

    submit_command(
        &handle,
        committed.revision,
        CoreCommand::RejectJoin {
            request_id: request.request_id,
        },
    );
    let stale = next_error(&receiver);
    assert_eq!(stale.code, CoreErrorCode::InvalidStateTransition);
    assert!(stale.message.contains("stale"));
    runtime.shutdown().expect("shutdown host actor");
}

#[test]
fn trust_write_failure_is_visible_and_approval_becomes_session_only() {
    let (runtime, handle, receiver) = start_host(ApprovalMode::Manual, true);
    let request = join_request("request-2", "listener-2", TrustState::SessionOnly, true);
    handle
        .submit_transport_event(TransportEvent::JoinRequested(request.clone()))
        .expect("submit join request");
    let pending = next_snapshot(&receiver, 5);
    submit_command(
        &handle,
        pending.revision,
        CoreCommand::ApproveJoin {
            request_id: request.request_id,
        },
    );
    let persisting = next_snapshot(&receiver, pending.revision.get() + 1);
    assert_eq!(persisting.pending_join_requests.len(), 1);
    let storage_effect = next_storage_effect(&receiver);

    let storage_error = CoreError::new(
        CoreErrorCode::StorageWriteFailed,
        "injected trusted-device write failure",
        ErrorSeverity::Error,
        false,
        Some(storage_effect.operation_id.clone()),
    )
    .expect("valid storage error");
    handle
        .submit_storage_event(StorageEvent::OperationFailed {
            operation_id: storage_effect.operation_id,
            error: storage_error,
        })
        .expect("submit storage failure");
    let downgraded = next_snapshot(&receiver, persisting.revision.get() + 1);
    assert_eq!(
        downgraded
            .last_error
            .as_ref()
            .expect("storage error remains visible")
            .code,
        CoreErrorCode::StorageWriteFailed
    );
    assert_eq!(downgraded.pending_join_requests.len(), 1);
    let approval = next_transport_effect(&receiver);
    assert!(matches!(
        approval.request,
        TransportEffectRequest::DeliverJoinApproval {
            trusted_for_future: false,
            ..
        }
    ));

    handle
        .submit_transport_event(TransportEvent::DeliveryCompleted {
            operation_id: approval.operation_id,
            report: DeliveryReport::new(1, 1, 0).expect("successful report"),
        })
        .expect("submit approval delivery");
    let approved = next_snapshot(&receiver, downgraded.revision.get() + 1);
    assert!(approved.pending_join_requests.is_empty());
    assert_eq!(
        approved
            .last_error
            .as_ref()
            .expect("persistence failure remains visible")
            .code,
        CoreErrorCode::StorageWriteFailed
    );
    runtime.shutdown().expect("shutdown host actor");
}

#[test]
fn trusted_policy_auto_approves_but_waits_for_delivery() {
    let (runtime, handle, receiver) = start_host(ApprovalMode::TrustedDevices, false);
    let request = join_request("request-3", "listener-3", TrustState::Trusted, true);
    handle
        .submit_transport_event(TransportEvent::JoinRequested(request))
        .expect("submit trusted join request");
    let awaiting_delivery = next_snapshot(&receiver, 5);
    assert_eq!(awaiting_delivery.pending_join_requests.len(), 1);
    let approval = next_transport_effect(&receiver);
    assert!(matches!(
        approval.request,
        TransportEffectRequest::DeliverJoinApproval {
            trusted_for_future: true,
            ..
        }
    ));
    handle
        .submit_transport_event(TransportEvent::DeliveryCompleted {
            operation_id: approval.operation_id,
            report: DeliveryReport::new(1, 1, 0).expect("successful report"),
        })
        .expect("submit auto-approval delivery");
    let approved = next_snapshot(&receiver, awaiting_delivery.revision.get() + 1);
    assert!(approved.pending_join_requests.is_empty());
    runtime.shutdown().expect("shutdown host actor");
}

fn start_host(
    approval_mode: ApprovalMode,
    remember_approved_devices: bool,
) -> (CoreActorRuntime, CoreActorHandle, Receiver<CoreNotification>) {
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
    let selected = submit_and_snapshot(
        &handle,
        &receiver,
        SnapshotRevision::new(0),
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4096), Some(2000))
        .expect("valid audio source");
    let drafted = submit_and_snapshot(
        &handle,
        &receiver,
        selected.revision,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Block 12 host".to_owned()),
            approval_mode: Some(approval_mode),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(remember_approved_devices),
        }),
    );
    let creating = submit_and_snapshot(
        &handle,
        &receiver,
        drafted.revision,
        CoreCommand::CreateHostSession,
    );
    let advertising = next_platform_effect(&receiver);
    assert!(matches!(
        advertising.request,
        PlatformEffectRequest::StartAdvertising(_)
    ));
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertising.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("submit advertising success");
    next_snapshot(&receiver, creating.revision.get() + 1);
    (runtime, handle, receiver)
}

fn join_request(
    request_id: &str,
    listener_id: &str,
    trust_state: TrustState,
    invite_code_valid: bool,
) -> JoinRequestSummary {
    JoinRequestSummary::new(
        RequestId::new(request_id).expect("valid request ID"),
        DeviceId::new(listener_id).expect("valid listener ID"),
        format!("Listener {listener_id}"),
        trust_state,
        invite_code_valid,
        MonotonicMillis::new(100),
    )
    .expect("valid join request")
}

fn submit_and_snapshot(
    handle: &CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    revision: SnapshotRevision,
    command: CoreCommand,
) -> CoreSnapshot {
    submit_command(handle, revision, command);
    next_snapshot(receiver, revision.get() + 1)
}

fn submit_command(handle: &CoreActorHandle, revision: SnapshotRevision, command: CoreCommand) {
    handle
        .submit_command(
            CoreCommandRequest::new(revision, command).expect("valid command request"),
        )
        .expect("queue command");
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

fn next_transport_effect(receiver: &Receiver<CoreNotification>) -> TransportEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for transport effect: {error}"),
        }
    }
}

fn next_storage_effect(receiver: &Receiver<CoreNotification>) -> StorageEffect {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::StorageEffect(effect)) => return effect,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for storage effect: {error}"),
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

fn next_diagnostic(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::CoreDiagnostic {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Diagnostic(diagnostic)) => return diagnostic,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for diagnostic: {error}"),
        }
    }
}
