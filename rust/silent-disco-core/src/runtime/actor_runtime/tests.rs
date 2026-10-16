use super::{CoreActorConfig, CoreActorRuntime};
use crate::domain::{AppRole, DeviceId};
use crate::error::CoreErrorCode;
use crate::runtime::{CoreCommand, CoreCommandRequest, SnapshotRevision};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

#[test]
fn rejects_zero_queue_capacity() {
    let mut config = CoreActorConfig::new(DeviceId::new("core-1").expect("valid device ID"));
    config.actor_queue_capacity = 0;
    let error = config.validate().expect_err("zero capacity must fail");
    assert_eq!(error.code, CoreErrorCode::InvalidArgument);
}

#[test]
fn observer_failure_is_visible_and_shutdown_is_explicit() {
    let runtime = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("core-2").expect("valid device ID")),
        |_| {
            Err(super::core_error(
                CoreErrorCode::FfiCallbackFailed,
                "observer rejected notification",
                crate::error::ErrorSeverity::Fatal,
                false,
                None,
            ))
        },
    )
    .expect("runtime starts before observer failure is reported");
    let handle = runtime.handle();
    let shutdown_error = runtime
        .shutdown()
        .expect_err("observer failure must make controlled shutdown fail visibly");
    assert_eq!(shutdown_error.code, CoreErrorCode::ShutdownFailed);
    let observer_error = handle
        .current_snapshot()
        .expect_err("joined observer failure must remain visible through the handle");
    assert_eq!(observer_error.code, CoreErrorCode::FfiCallbackFailed);
}

#[test]
fn notification_queue_overflow_fails_actor_without_blocking() {
    let release_gate = Arc::new((Mutex::new(false), Condvar::new()));
    let observer_gate = Arc::clone(&release_gate);
    let observer_calls = Arc::new(AtomicUsize::new(0));
    let observer_call_count = Arc::clone(&observer_calls);
    let (entered_sender, entered_receiver) = mpsc::channel();

    let mut config = CoreActorConfig::new(DeviceId::new("core-3").expect("valid device ID"));
    config.actor_queue_capacity = 8;
    config.notification_queue_capacity = 1;

    let runtime = CoreActorRuntime::start(config, move |_| {
        if observer_call_count.fetch_add(1, Ordering::SeqCst) == 0 {
            entered_sender
                .send(())
                .map_err(|_| super::worker_stopped(None))?;
            let (released, wake) = &*observer_gate;
            let mut released = released
                .lock()
                .map_err(|_| super::shared_state_error("test gate was poisoned"))?;
            while !*released {
                released = wake
                    .wait(released)
                    .map_err(|_| super::shared_state_error("test gate was poisoned"))?;
            }
        }
        Ok(())
    })
    .expect("runtime starts");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("initial notification entered observer");

    let handle = runtime.handle();
    handle
        .submit_command(
            CoreCommandRequest::new(
                SnapshotRevision::new(0),
                CoreCommand::SelectRole {
                    role: AppRole::Host,
                },
            )
            .expect("valid first command"),
        )
        .expect("first command admitted");

    for _ in 0..1_000 {
        if handle
            .current_snapshot()
            .is_ok_and(|snapshot| snapshot.revision == SnapshotRevision::new(1))
        {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(
        handle
            .current_snapshot()
            .expect("first command snapshot is available")
            .revision,
        SnapshotRevision::new(1)
    );

    handle
        .submit_command(
            CoreCommandRequest::new(
                SnapshotRevision::new(1),
                CoreCommand::SelectRole {
                    role: AppRole::Listener,
                },
            )
            .expect("valid second command"),
        )
        .expect("second command admitted");

    let overflow = (0..1_000).find_map(|_| match handle.current_snapshot() {
        Ok(_) => {
            std::thread::yield_now();
            None
        }
        Err(error) => Some(error),
    });
    let overflow = overflow.expect("notification overflow becomes visible");
    assert_eq!(overflow.code, CoreErrorCode::QueueOverflow);
    assert_eq!(overflow.message, "notification queue is full");

    let (released, wake) = &*release_gate;
    *released.lock().expect("release gate lock") = true;
    wake.notify_all();

    assert!(runtime.shutdown().is_err());
    assert!(observer_calls.load(Ordering::SeqCst) >= 1);
}
