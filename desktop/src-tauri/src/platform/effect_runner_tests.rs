use super::effect_runner::{
    DesktopPlatformEffectDispatcher, DesktopPlatformEffectExecutor, DesktopPlatformEffectInbox,
    DesktopPlatformEffectRunner, DesktopPlatformEventSink,
};
use super::failure::{DesktopPlatformFailure, core_error};
use super::paths::DesktopProfilePaths;
use crate::profile::ProfileId;
use silent_disco_core::domain::{DeviceId, OperationId};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    CapabilitySnapshot, CoreActorConfig, CoreActorRuntime, CoreNotification, CoreSnapshot,
    DiscoveryRequest, PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);
static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "silent-disco-block15-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            assert!(
                error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(),
                "failed to remove test directory: {error}"
            );
        }
    }
}

fn test_paths(root: &TestDirectory) -> DesktopProfilePaths {
    let profile_id = ProfileId::parse("main").expect("valid profile ID");
    let paths = DesktopProfilePaths::from_trusted_app_local_data_root(&root.0, &profile_id)
        .expect("valid profile paths");
    paths.prepare_directories().expect("prepare profile paths");
    paths
}

struct RecordingSink {
    snapshot: CoreSnapshot,
    sender: mpsc::Sender<PlatformEvent>,
}

impl DesktopPlatformEventSink for RecordingSink {
    fn submit_platform_event(&self, event: PlatformEvent) -> Result<(), CoreError> {
        self.sender.send(event).map_err(|_| {
            test_core_error(
                CoreErrorCode::WorkerStopped,
                "test platform event receiver was dropped",
                None,
            )
        })
    }

    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(self.snapshot.clone())
    }
}

struct FixedExecutor {
    result: Result<PlatformOperationCompletion, DesktopPlatformFailure>,
    calls: Arc<AtomicUsize>,
}

impl DesktopPlatformEffectExecutor for FixedExecutor {
    fn execute(
        &self,
        _effect: &PlatformEffect,
        _snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.result.clone()
    }
}

struct PanicExecutor;

impl DesktopPlatformEffectExecutor for PanicExecutor {
    fn execute(
        &self,
        _effect: &PlatformEffect,
        _snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        panic!("injected adapter panic")
    }
}

struct DropAwareExecutor(Arc<AtomicBool>);

impl DesktopPlatformEffectExecutor for DropAwareExecutor {
    fn execute(
        &self,
        _effect: &PlatformEffect,
        _snapshot: Option<&CoreSnapshot>,
    ) -> Result<PlatformOperationCompletion, DesktopPlatformFailure> {
        Ok(PlatformOperationCompletion::CapabilitiesResolved(
            CapabilitySnapshot::default(),
        ))
    }
}

impl Drop for DropAwareExecutor {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn recording_components() -> (
    DesktopPlatformEffectDispatcher,
    DesktopPlatformEffectInbox,
    Arc<RecordingSink>,
    Receiver<PlatformEvent>,
) {
    let (dispatcher, inbox) = DesktopPlatformEffectDispatcher::channel();
    let (sender, receiver) = mpsc::channel();
    let sink = Arc::new(RecordingSink {
        snapshot: CoreSnapshot::default(),
        sender,
    });
    (dispatcher, inbox, sink, receiver)
}

fn start_test_runner(
    inbox: DesktopPlatformEffectInbox,
    dispatcher: DesktopPlatformEffectDispatcher,
    sink: Arc<dyn DesktopPlatformEventSink>,
    executor: Arc<dyn DesktopPlatformEffectExecutor>,
) -> DesktopPlatformEffectRunner {
    DesktopPlatformEffectRunner::start_with_components(inbox, dispatcher, sink, executor)
        .expect("start test runner")
}

fn operation_id(value: &str) -> OperationId {
    OperationId::new(value).expect("valid operation ID")
}

fn capability_effect(value: &str) -> PlatformEffect {
    PlatformEffect::new(
        operation_id(value),
        PlatformEffectRequest::RequestCapabilities(vec![PermissionCapability::SecureStore]),
    )
    .expect("valid capability effect")
}

#[test]
fn preserves_operation_id_and_completion_order() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(FixedExecutor {
            result: Ok(PlatformOperationCompletion::CapabilitiesResolved(
                CapabilitySnapshot::default(),
            )),
            calls: Arc::clone(&calls),
        }),
    );

    dispatcher
        .dispatch(capability_effect("operation-correlation"))
        .expect("dispatch effect");
    let event = receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("completion event");
    assert!(matches!(
        event,
        PlatformEvent::OperationSucceeded {
            operation_id,
            completion: PlatformOperationCompletion::CapabilitiesResolved(_),
        } if operation_id.as_str() == "operation-correlation"
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn unsupported_effect_returns_correlated_visible_failure() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let root = TestDirectory::new();
    let paths = test_paths(&root);
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(super::effect_runner::DesktopPlatformAdapters::new(paths)),
    );
    let effect = PlatformEffect::new(
        operation_id("unsupported-discovery"),
        PlatformEffectRequest::StartDiscovery(DiscoveryRequest {
            scan_window_ms: 1_000,
        }),
    )
    .expect("valid discovery effect");
    dispatcher.dispatch(effect).expect("dispatch effect");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("failure event");
    assert!(matches!(
        event,
        PlatformEvent::OperationFailed {
            operation_id,
            error,
        } if operation_id.as_str() == "unsupported-discovery"
            && error.operation_id.as_ref() == Some(&operation_id)
            && error.code == CoreErrorCode::CapabilityUnavailable
    ));
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn diagnostics_export_writes_real_safe_snapshot_file() {
    let root = TestDirectory::new();
    let paths = test_paths(&root);
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(super::effect_runner::DesktopPlatformAdapters::new(
            paths.clone(),
        )),
    );
    let effect = PlatformEffect::new(
        operation_id("diagnostics-operation"),
        PlatformEffectRequest::ShareDiagnostics {
            export_id: "export/with/untrusted/separators".to_owned(),
        },
    )
    .expect("valid diagnostics effect");
    dispatcher.dispatch(effect).expect("dispatch effect");

    let event = receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("diagnostics completion");
    assert!(matches!(
        event,
        PlatformEvent::OperationSucceeded {
            operation_id,
            completion: PlatformOperationCompletion::DiagnosticsShared { export_id },
        } if operation_id.as_str() == "diagnostics-operation"
            && export_id == "export/with/untrusted/separators"
    ));
    let files = fs::read_dir(paths.diagnostics())
        .expect("read diagnostics directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect diagnostics entries");
    assert_eq!(files.len(), 1);
    let path = files[0].path();
    assert_eq!(path.parent(), Some(paths.diagnostics()));
    assert_eq!(
        path.extension().and_then(|value| value.to_str()),
        Some("json")
    );
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("read export")).expect("parse export JSON");
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["exportId"], "export/with/untrusted/separators");
    assert_eq!(json["snapshot"]["revision"], "0");
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn adapter_error_and_panic_are_contained_as_failed_events() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let runner = start_test_runner(inbox, dispatcher.clone(), sink, Arc::new(PanicExecutor));
    dispatcher
        .dispatch(capability_effect("panic-contained"))
        .expect("dispatch panic effect");
    let panic_event = receiver.recv_timeout(TEST_TIMEOUT).expect("panic failure");
    assert!(matches!(
        panic_event,
        PlatformEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "panic-contained"
                && error.code == CoreErrorCode::FfiPanicContained
    ));
    runner.shutdown().expect("shutdown panic runner");

    let (dispatcher, inbox, sink, receiver) = recording_components();
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(FixedExecutor {
            result: Err(DesktopPlatformFailure::new(
                CoreErrorCode::PlatformOperationFailed,
                "injected adapter failure",
                ErrorSeverity::Error,
                true,
            )),
            calls,
        }),
    );
    dispatcher
        .dispatch(capability_effect("error-contained"))
        .expect("dispatch error effect");
    let error_event = receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("adapter failure");
    assert!(matches!(
        error_event,
        PlatformEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "error-contained"
                && error.code == CoreErrorCode::PlatformOperationFailed
                && error.operation_id.as_ref() == Some(&operation_id)
    ));
    runner.shutdown().expect("shutdown error runner");
}

#[test]
fn cancellation_prevents_success_and_skips_adapter_execution() {
    let (dispatcher, inbox, sink, receiver) = recording_components();
    let calls = Arc::new(AtomicUsize::new(0));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(FixedExecutor {
            result: Ok(PlatformOperationCompletion::CapabilitiesResolved(
                CapabilitySnapshot::default(),
            )),
            calls: Arc::clone(&calls),
        }),
    );
    let operation = operation_id("cancel-before-run");
    dispatcher
        .cancel(operation.clone())
        .expect("mark cancelled");
    dispatcher
        .dispatch(
            PlatformEffect::new(
                operation,
                PlatformEffectRequest::RequestCapabilities(vec![PermissionCapability::SecureStore]),
            )
            .expect("valid cancelled effect"),
        )
        .expect("dispatch cancelled effect");

    let event = receiver.recv_timeout(TEST_TIMEOUT).expect("cancel event");
    assert!(matches!(
        event,
        PlatformEvent::OperationFailed { operation_id, error }
            if operation_id.as_str() == "cancel-before-run"
                && error.code == CoreErrorCode::ShutdownInProgress
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    runner.shutdown().expect("shutdown runner");
}

#[test]
fn shutdown_joins_worker_and_rejects_later_dispatch() {
    let (dispatcher, inbox, sink, _receiver) = recording_components();
    let dropped = Arc::new(AtomicBool::new(false));
    let runner = start_test_runner(
        inbox,
        dispatcher.clone(),
        sink,
        Arc::new(DropAwareExecutor(Arc::clone(&dropped))),
    );
    runner.shutdown().expect("shutdown runner");
    assert!(dropped.load(Ordering::Acquire));
    let error = dispatcher
        .dispatch(capability_effect("after-shutdown"))
        .expect_err("dispatch after shutdown must fail");
    assert_eq!(error.code, CoreErrorCode::ShutdownInProgress);
}

#[test]
fn stale_completion_is_rejected_by_authoritative_core() {
    let (notification_sender, notification_receiver) = mpsc::channel();
    let observer = move |notification: CoreNotification| {
        notification_sender.send(notification).map_err(|_| {
            test_core_error(
                CoreErrorCode::WorkerStopped,
                "test core notification receiver was dropped",
                None,
            )
        })
    };
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-test-device").expect("valid device ID")),
        observer,
    )
    .expect("start actor");
    let handle = actor.handle();
    assert!(matches!(
        notification_receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("initial snapshot"),
        CoreNotification::Snapshot(_)
    ));

    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: operation_id("stale-completion"),
            completion: PlatformOperationCompletion::CapabilitiesResolved(
                CapabilitySnapshot::default(),
            ),
        })
        .expect("queue stale completion");

    let error = loop {
        match notification_receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("stale completion result")
        {
            CoreNotification::Error(error) => break error,
            CoreNotification::Snapshot(_)
            | CoreNotification::Effect(_)
            | CoreNotification::TransportEffect(_)
            | CoreNotification::StorageEffect(_)
            | CoreNotification::Diagnostic(_) => {}
        }
    };
    assert_eq!(error.code, CoreErrorCode::InvalidStateTransition);
    assert_eq!(
        error.operation_id.as_ref().map(OperationId::as_str),
        Some("stale-completion")
    );
    actor.shutdown().expect("shutdown actor");
}

fn test_core_error(
    code: CoreErrorCode,
    message: &'static str,
    operation_id: Option<OperationId>,
) -> CoreError {
    core_error(code, message, ErrorSeverity::Error, false, operation_id)
}
