use silent_disco_core::domain::{DeviceId, TuningSettings};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand, CoreCommandRequest,
    CoreNotification, CoreSnapshot, SnapshotRevision, StorageCompletion, StorageEffect,
    StorageEffectRequest, StorageEvent, TuningPatch,
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn tuning_snapshot_changes_only_after_correlated_storage_success() {
    let (runtime, handle, receiver) = start_actor();
    let defaults = TuningSettings::default();
    let patch = TuningPatch {
        sync_cadence_ms: Some(1_500),
        startup_buffer_ms: Some(500),
        ..TuningPatch::default()
    };
    submit_command(
        &handle,
        SnapshotRevision::new(0),
        CoreCommand::UpdateTuning(patch),
    );
    let pending = next_snapshot(&receiver, 1);
    assert_eq!(pending.tuning, defaults);
    let effect = next_storage_effect(&receiver);
    let expected = match &effect.request {
        StorageEffectRequest::PersistSettings { settings } => settings.clone(),
        request @ StorageEffectRequest::PersistTrustedDevice { .. } => {
            panic!("unexpected storage effect: {request:?}")
        }
    };
    assert_eq!(expected.sync_cadence_ms, 1_500);
    assert_eq!(expected.startup_buffer_ms, 500);

    handle
        .submit_storage_event(StorageEvent::OperationSucceeded {
            operation_id: effect.operation_id,
            completion: StorageCompletion::SettingsSaved,
        })
        .expect("submit settings save completion");
    let committed = next_snapshot(&receiver, 2);
    assert_eq!(committed.tuning, expected);
    assert_eq!(next_diagnostic(&receiver), "settings_saved");
    runtime.shutdown().expect("shutdown actor");
}

#[test]
fn tuning_failure_preserves_previous_settings_and_is_visible() {
    let (runtime, handle, receiver) = start_actor();
    let defaults = TuningSettings::default();
    submit_command(
        &handle,
        SnapshotRevision::new(0),
        CoreCommand::UpdateTuning(TuningPatch {
            scan_window_ms: Some(4_000),
            ..TuningPatch::default()
        }),
    );
    let pending = next_snapshot(&receiver, 1);
    assert_eq!(pending.tuning, defaults);
    let effect = next_storage_effect(&receiver);
    let error = CoreError::new(
        CoreErrorCode::StorageWriteFailed,
        "injected tuning save failure",
        ErrorSeverity::Error,
        true,
        Some(effect.operation_id.clone()),
    )
    .expect("valid storage error");
    handle
        .submit_storage_event(StorageEvent::OperationFailed {
            operation_id: effect.operation_id,
            error,
        })
        .expect("submit settings save failure");
    let failed = next_snapshot(&receiver, 2);
    assert_eq!(failed.tuning, defaults);
    assert_eq!(
        failed.last_error.as_ref().expect("visible error").code,
        CoreErrorCode::StorageWriteFailed
    );
    assert_eq!(
        next_error(&receiver).code,
        CoreErrorCode::StorageWriteFailed
    );

    submit_command(
        &handle,
        failed.revision,
        CoreCommand::UpdateTuning(TuningPatch {
            scan_window_ms: Some(20_000),
            ..TuningPatch::default()
        }),
    );
    let validation = next_error(&receiver);
    assert_eq!(validation.code, CoreErrorCode::InvalidArgument);
    assert_eq!(
        handle
            .current_snapshot()
            .expect("snapshot after rejected tuning")
            .revision,
        failed.revision
    );
    runtime.shutdown().expect("shutdown actor");
}

fn start_actor() -> (
    CoreActorRuntime,
    CoreActorHandle,
    Receiver<CoreNotification>,
) {
    let (sender, receiver) = channel();
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("tuning-core").expect("valid device ID")),
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

fn submit_command(handle: &CoreActorHandle, revision: SnapshotRevision, command: CoreCommand) {
    handle
        .submit_command(CoreCommandRequest::new(revision, command).expect("valid command request"))
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

fn next_diagnostic(receiver: &Receiver<CoreNotification>) -> String {
    loop {
        match receiver.recv_timeout(TIMEOUT) {
            Ok(CoreNotification::Diagnostic(diagnostic)) => return diagnostic.name,
            Ok(_) => {}
            Err(error) => panic!("failed waiting for diagnostic: {error}"),
        }
    }
}
