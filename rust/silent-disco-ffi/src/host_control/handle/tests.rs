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

mod legacy_v3_trust_boundary_tests {
    use super::*;
    use crate::host_control::types::{FfiCoreNotification, FfiTransportState, FfiTrustState};

    struct NoopObserver;

    impl FfiCoreObserver for NoopObserver {
        fn on_notification(&self, _: FfiCoreNotification) -> Result<(), FfiBridgeError> {
            Ok(())
        }
    }

    #[test]
    fn plaintext_ffi_rejects_claimed_trust_before_any_actor_admission() {
        let handle = FfiCoreHandle::open("legacy-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        for state in [FfiTrustState::Trusted, FfiTrustState::Revoked] {
            let request = FfiJoinRequestInput {
                request_id: "join-1".to_owned(),
                device_id: "claimed-listener".to_owned(),
                display_name: "Claimed listener".to_owned(),
                trust_state: state,
                invite_code: None,
                received_at_ms: 1,
            };
            assert!(matches!(
                handle.submit_join_request(request),
                Err(FfiBridgeError::Core(message)) if message.contains("Legacy v3")
            ));
            let listener = FfiListenerSummary {
                device_id: "claimed-listener".to_owned(),
                display_name: "Claimed listener".to_owned(),
                trust_state: state,
                transport_state: FfiTransportState::Connected,
                synchronization: None,
                last_contact_ms: None,
                last_error: None,
            };
            assert!(matches!(
                handle.submit_listener_connected(listener),
                Err(FfiBridgeError::Core(message)) if message.contains("Legacy v3")
            ));
        }
        assert_eq!(
            legacy_v3_unverified_trust(FfiTrustState::Unknown).expect("unverified"),
            TrustState::SessionOnly
        );
        assert_eq!(
            legacy_v3_unverified_trust(FfiTrustState::SessionOnly).expect("manual session"),
            TrustState::SessionOnly
        );
        handle.shutdown().expect("shutdown");
    }

    #[test]
    fn plaintext_ffi_rejects_durable_approval_and_unsupported_host_drafts() {
        let handle = FfiCoreHandle::open("legacy-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        assert!(matches!(
            handle.submit_join_approved(true),
            Err(FfiBridgeError::Core(message)) if message.contains("Legacy v3")
        ));
        assert!(matches!(
            handle.approve_join(0, "join-1".to_owned(), true),
            Err(FfiBridgeError::Core(message)) if message.contains("Legacy v3")
        ));
        let mut draft = handle.current_snapshot().expect("snapshot").host_draft;
        draft.approval_mode = FfiApprovalMode::TrustedDevices;
        assert!(matches!(
            handle.update_host_draft(0, draft.clone()),
            Err(FfiBridgeError::Core(message)) if message.contains("Legacy v3")
        ));
        draft.approval_mode = FfiApprovalMode::Manual;
        draft.remember_approved_devices = true;
        assert!(matches!(
            handle.update_host_draft(0, draft),
            Err(FfiBridgeError::Core(message)) if message.contains("Legacy v3")
        ));
        assert!(reject_legacy_durable_host_draft(FfiApprovalMode::Manual, false).is_ok());
        assert!(reject_legacy_durable_host_draft(FfiApprovalMode::InviteCode, false).is_ok());
        handle.shutdown().expect("shutdown");
    }
}
