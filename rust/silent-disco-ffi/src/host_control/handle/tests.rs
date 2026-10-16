use super::*;

mod concurrency_tests {
    use super::FfiCoreHandle;
    use crate::host_control::types::{
        FfiAppRole, FfiBridgeError, FfiCoreNotification, FfiCoreObserver,
    };
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    /// Records every notification it is handed. Never rejects one, so this
    /// test exercises ordinary delivery racing shutdown rather than the
    /// separate `FfiCallbackFailed` path `observer_callback_error` covers.
    struct RecordingObserver {
        delivered: AtomicU64,
        /// Notifications delivered strictly after the owning
        /// `FfiCoreHandle::shutdown` call returned would mean the
        /// notification worker outlived the runtime that is supposed to own
        /// it -- exactly the "observer removal during notification" failure
        /// mode this test exists to catch.
        delivered_after_shutdown_observed: Mutex<Vec<()>>,
        shutdown_observed: AtomicBool,
    }

    impl FfiCoreObserver for RecordingObserver {
        fn on_notification(
            &self,
            _notification: FfiCoreNotification,
        ) -> Result<(), FfiBridgeError> {
            self.delivered.fetch_add(1, Ordering::SeqCst);
            if self.shutdown_observed.load(Ordering::SeqCst)
                && let Ok(mut log) = self.delivered_after_shutdown_observed.lock()
            {
                log.push(());
            }
            Ok(())
        }
    }

    /// Block 24 ("observer removal during notification"): many threads
    /// submit commands -- each of which produces a notification dispatched
    /// to the observer on the runtime's own notification thread -- while
    /// another thread concurrently calls `shutdown`, which is documented to
    /// join both the actor and notification workers before returning. No
    /// thread may panic or deadlock, `shutdown` must actually return, and
    /// once it has returned, the observer must never be invoked again: a
    /// notification delivered after that point would mean Kotlin/Swift could
    /// free the observer object and still have Rust call into it.
    ///
    /// Scale note: an earlier version of this test used 4 submitter threads
    /// x 200 submissions x 15 outer iterations. That volume reproduced a
    /// genuine production bug (see the `snapshot: Mutex`, formerly
    /// `RwLock`, field doc comment on `ActorShared`) but, once fixed, became
    /// purely a scheduler-contention hazard on a busy shared host: run
    /// alone it always finished in well under a second, but run alongside
    /// this crate's other parallel tests (and, in this environment,
    /// another concurrently building process) it could occasionally stall
    /// for minutes from oversubscription alone, confirmed by isolating it
    /// with `--test-threads=1` (always fast) versus the full suite
    /// (occasionally very slow) and by tracing every phase of `shutdown`
    /// and the actor loop with temporary instrumentation, which showed
    /// steady progress rather than a stuck lock. The smaller volume below
    /// still exercises the identical race at a size proven fast even under
    /// contention.
    #[test]
    fn shutdown_races_with_concurrent_command_submission_and_notification_delivery() {
        for iteration in 0_u32..8 {
            let observer = Arc::new(RecordingObserver {
                delivered: AtomicU64::new(0),
                delivered_after_shutdown_observed: Mutex::new(Vec::new()),
                shutdown_observed: AtomicBool::new(false),
            });
            let handle = Arc::new(
                FfiCoreHandle::open(
                    format!("device-shutdown-race-{iteration}"),
                    observer.clone(),
                )
                .expect("core handle opens"),
            );

            let submitters: Vec<_> = (0..3)
                .map(|_| {
                    let submitter_handle = Arc::clone(&handle);
                    thread::spawn(move || {
                        for _ in 0..40 {
                            let revision = submitter_handle
                                .current_snapshot()
                                .map_or(0, |snapshot| snapshot.revision);
                            // A stale-revision rejection or a post-shutdown
                            // `Closed` error are both legitimate, typed
                            // outcomes here; only a panic is not.
                            let _ = submitter_handle.select_role(revision, FfiAppRole::Host);
                            // A cooperative yield after every iteration: a
                            // pure busy spin across several threads can crowd
                            // out the actor/notification/main threads under
                            // real scheduler contention (observed directly on
                            // this environment's shared host, and far more
                            // pronounced under sanitizer instrumentation,
                            // where a tight spin can stall the shutdown side
                            // for minutes even though the lock itself is
                            // never actually deadlocked). Yielding keeps this
                            // a genuine concurrent race without depending on
                            // this process getting a fair scheduling slice on
                            // a contended host.
                            thread::yield_now();
                        }
                    })
                })
                .collect();

            // Let real commands, and the notifications they cause, actually
            // flow before racing the shutdown.
            thread::sleep(Duration::from_micros(200));
            let shutdown_result = handle.shutdown();
            observer.shutdown_observed.store(true, Ordering::SeqCst);

            for submitter in submitters {
                submitter.join().expect("submitter thread must not panic");
            }

            assert!(
                shutdown_result.is_ok(),
                "shutdown must succeed even racing concurrent command submission: {shutdown_result:?}"
            );
            assert!(
                observer
                    .delivered_after_shutdown_observed
                    .lock()
                    .expect("observer log lock")
                    .is_empty(),
                "a notification was delivered to the observer after shutdown returned"
            );

            // Shutdown consumes the runtime; a second call must be an
            // explicit `Closed` failure, not a panic against freed state.
            assert!(matches!(handle.shutdown(), Err(FfiBridgeError::Closed(_))));
        }
    }
}
