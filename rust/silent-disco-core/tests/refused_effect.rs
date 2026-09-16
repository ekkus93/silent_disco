//! Block 52.4 regression lock.
//!
//! A platform that cannot accept an effect (a full queue, a transport that is
//! shutting down, no active session) reports that by returning an error from
//! its observer. The notification worker used to treat that as fatal and exit,
//! so every later effect -- including a join approval -- was silently never
//! delivered, and the actor handle started failing. A refused effect must fail
//! only its own operation, visibly, and notification delivery must continue.

use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, MonotonicMillis, RequestId, TrustState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, CoreSnapshot, HostDraftPatch, InviteCodePatch,
    JoinRequestSummary, PlatformEvent, PlatformOperationCompletion, SnapshotRevision,
    TransportEvent,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn a_refused_effect_fails_its_operation_and_notifications_keep_flowing() {
    let (sender, receiver) = channel();
    let refused_once = Arc::new(AtomicBool::new(false));
    let observer_refused = Arc::clone(&refused_once);
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("refusing-host").expect("valid device ID")),
        move |notification| {
            let refuse = matches!(notification, CoreNotification::Effect(_))
                && !observer_refused.swap(true, Ordering::AcqRel);
            sender
                .send(notification)
                .expect("test receiver remains connected");
            if refuse {
                return Err(CoreError::new(
                    CoreErrorCode::QueueOverflow,
                    "platform effect queue is full",
                    ErrorSeverity::Error,
                    true,
                    None,
                )
                .expect("valid error"));
            }
            Ok(())
        },
    )
    .expect("start actor runtime");
    let handle = runtime.handle();
    next_snapshot(&receiver, 0);

    submit(
        &handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(&receiver, 1);
    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4_096), Some(2_000))
        .expect("valid descriptor");
    submit(
        &handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Refusing host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(&receiver, 2);

    // Creating the session emits the platform effect the observer refuses.
    submit(&handle, 2, CoreCommand::CreateHostSession);
    let failed = wait_for(&receiver, |snapshot| snapshot.last_error.is_some());
    assert!(
        refused_once.load(Ordering::Acquire),
        "the effect was refused"
    );
    let error = failed
        .last_error
        .expect("the refusal is visible on the snapshot");
    assert!(
        error.operation_id.is_some(),
        "the failure is attributed to the refused operation"
    );
    assert_ne!(failed.host_lifecycle, HostLifecycle::CreatingSession);

    // The runtime is still healthy: state is readable and new work still
    // produces notifications.
    let current = handle
        .current_snapshot()
        .expect("a refused effect must not poison the actor handle");
    submit(
        &handle,
        current.revision.get(),
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Still alive".to_owned()),
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Unchanged,
            remember_approved_devices: None,
        }),
    );
    let later = wait_for(&receiver, |snapshot| {
        snapshot.host_draft.session_name == "Still alive"
    });
    assert!(later.revision.get() > current.revision.get());

    runtime
        .shutdown()
        .expect("clean shutdown after a refused effect");
}

/// The case observed on the desktop at 10:00 on 2026-09-16: a join approval
/// the host transport could not accept. The approval must fail visibly and
/// leave the request pending so the operator can approve again, instead of
/// waiting forever for a delivery confirmation that can never arrive.
#[test]
fn a_refused_join_approval_fails_visibly_and_leaves_the_request_retryable() {
    let (sender, receiver) = channel();
    let refused_once = Arc::new(AtomicBool::new(false));
    let observer_refused = Arc::clone(&refused_once);
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("refusing-approver").expect("valid device ID")),
        move |notification| {
            let refuse = matches!(notification, CoreNotification::TransportEffect(_))
                && !observer_refused.swap(true, Ordering::AcqRel);
            sender
                .send(notification)
                .expect("test receiver remains connected");
            if refuse {
                return Err(CoreError::new(
                    CoreErrorCode::QueueOverflow,
                    "desktop host transport effect queue is full",
                    ErrorSeverity::Error,
                    true,
                    None,
                )
                .expect("valid error"));
            }
            Ok(())
        },
    )
    .expect("start actor runtime");
    let handle = runtime.handle();
    let pending = host_with_one_pending_request(&handle, &receiver);

    submit(
        &handle,
        pending.revision.get(),
        CoreCommand::ApproveJoin {
            request_id: RequestId::new("request-1").expect("valid request ID"),
            remember_for_future: false,
        },
    );
    let failed = wait_for(&receiver, |snapshot| {
        snapshot
            .last_error
            .as_ref()
            .is_some_and(|error| error.message.contains("delivery"))
    });
    assert!(
        refused_once.load(Ordering::Acquire),
        "the approval was refused"
    );
    assert_eq!(
        failed.pending_join_requests.len(),
        1,
        "the request stays pending so the operator can approve again"
    );
    assert!(
        failed.listeners.is_empty(),
        "a refused approval admits nobody"
    );

    // A second approval is accepted and dispatched: nothing is wedged.
    submit(
        &handle,
        failed.revision.get(),
        CoreCommand::ApproveJoin {
            request_id: RequestId::new("request-1").expect("valid request ID"),
            remember_for_future: false,
        },
    );
    let retried = next_transport_effect_operation(&receiver);
    handle
        .submit_transport_event(TransportEvent::DeliveryCompleted {
            operation_id: retried,
            report: silent_disco_core::runtime::DeliveryReport::new(1, 1, 0).expect("valid report"),
        })
        .expect("delivery completion");
    let admitted = wait_for(&receiver, |snapshot| snapshot.listeners.len() == 1);
    assert!(admitted.pending_join_requests.is_empty());

    runtime
        .shutdown()
        .expect("clean shutdown after a refused approval");
}

/// Drives a manual-approval host to one pending join request.
fn host_with_one_pending_request(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
) -> CoreSnapshot {
    next_snapshot(receiver, 0);
    submit(
        handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(receiver, 1);
    let source = AudioSourceDescriptor::new("source-1", "fixture.wav", Some(4_096), Some(2_000))
        .expect("valid descriptor");
    submit(
        handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Approving host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(receiver, 2);
    submit(handle, 2, CoreCommand::CreateHostSession);
    let advertising = next_platform_effect(receiver);
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertising,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising completion");
    wait_for(receiver, |snapshot| {
        snapshot.host_lifecycle == HostLifecycle::WaitingForListeners
    });

    handle
        .submit_transport_event(TransportEvent::JoinRequested(
            JoinRequestSummary::new(
                RequestId::new("request-1").expect("valid request ID"),
                DeviceId::new("listener-1").expect("valid listener ID"),
                "Listener one",
                TrustState::SessionOnly,
                true,
                MonotonicMillis::new(100),
            )
            .expect("valid join request"),
        ))
        .expect("join request fact");
    wait_for(receiver, |snapshot| {
        snapshot.pending_join_requests.len() == 1
    })
}

fn next_platform_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::domain::OperationId {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect.operation_id,
            Ok(_) => {}
            Err(error) => panic!("no platform effect: {error:?}"),
        }
    }
}

fn next_transport_effect_operation(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::domain::OperationId {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect.operation_id,
            Ok(_) => {}
            Err(error) => panic!("no transport effect: {error:?}"),
        }
    }
}

fn submit(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    revision: u64,
    command: CoreCommand,
) {
    handle
        .submit_command(
            CoreCommandRequest::new(SnapshotRevision::new(revision), command)
                .expect("valid command request"),
        )
        .expect("queue command");
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum: u64) -> CoreSnapshot {
    wait_for(receiver, |snapshot| snapshot.revision.get() >= minimum)
}

fn wait_for(
    receiver: &Receiver<CoreNotification>,
    predicate: impl Fn(&CoreSnapshot) -> bool,
) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot)) if predicate(&snapshot) => return snapshot,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for a matching snapshot"),
            Err(RecvTimeoutError::Disconnected) => panic!("notification delivery stopped"),
        }
    }
}
