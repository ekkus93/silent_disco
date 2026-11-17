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
    use silent_disco_core::domain::SessionId;
    use silent_disco_core::transport::secure_identity::{
        PendingListenerProof, VerifiedListenerKey,
    };
    use silent_disco_core::transport::secure_trust::HostVerifiedTrust;

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
                Err(FfiBridgeError::Core(message)) if message.contains("this host's own records")
            ));
        }
        assert_eq!(
            connected_listener_trust(TrustState::SessionOnly, FfiTrustState::Unknown)
                .expect("unverified"),
            TrustState::SessionOnly
        );
        assert_eq!(
            connected_listener_trust(TrustState::SessionOnly, FfiTrustState::SessionOnly)
                .expect("manual session"),
            TrustState::SessionOnly
        );
        assert!(
            connected_listener_trust(TrustState::Trusted, FfiTrustState::Revoked).is_err(),
            "the app never reports a revocation"
        );
        handle.shutdown().expect("shutdown");
    }

    #[test]
    fn approved_devices_drafts_are_accepted_now_that_trust_is_verified() {
        let handle = FfiCoreHandle::open("legacy-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        // Approved-devices mode and "Always allow" are accepted: they are
        // backed by the host's verified records (remediation F1).
        let mut draft = handle.current_snapshot().expect("snapshot").host_draft;
        draft.approval_mode = crate::host_control::types::FfiApprovalMode::TrustedDevices;
        draft.remember_approved_devices = true;
        handle
            .update_host_draft(0, draft)
            .expect("approved-devices draft is accepted");
        handle.shutdown().expect("shutdown");
    }

    /// Fixture host and listener keys (seeds 3 and 5) and a proof verified over a
    /// fixed challenge, the same way a TLS admission verifies a listener.
    fn verified_listener() -> (Vec<u8>, Vec<u8>, VerifiedListenerKey) {
        use p256::{
            ecdsa::{Signature, SigningKey, signature::Signer},
            pkcs8::EncodePublicKey,
        };
        let spki_of = |seed: u8| {
            SigningKey::from_slice(&[seed; 32])
                .expect("fixture key")
                .verifying_key()
                .to_public_key_der()
                .expect("spki")
                .as_bytes()
                .to_vec()
        };
        let host = spki_of(3);
        let listener = spki_of(5);
        let signing = SigningKey::from_slice(&[5_u8; 32]).expect("listener key");
        let mut challenge = PendingListenerProof::new(
            SessionId::new("trust-session").expect("session"),
            1,
            &host,
            [11; 32],
            [19; 32],
        )
        .expect("challenge");
        let transcript = challenge.signing_transcript(&listener).expect("transcript");
        let signed: Signature = signing.sign(&transcript);
        let verified = challenge
            .verify_and_consume(&listener, signed.to_der().as_bytes())
            .expect("verified listener");
        (host, listener, verified)
    }

    #[test]
    fn a_verified_listener_is_trusted_only_while_the_host_records_say_so() {
        let (host, _listener, verified) = verified_listener();
        let device = verified.canonical_device_id();
        let handle = FfiCoreHandle::open("trust-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        // No records loaded: a verified key is still session-only.
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::SessionOnly
        );

        let mut registry = HostVerifiedTrust::new(&host).expect("registry");
        registry.enroll(verified, "Kitchen").expect("enroll");
        handle
            .set_host_trust_snapshot(host.clone(), registry.to_json().expect("json"))
            .expect("load records");
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::Trusted
        );

        assert!(registry.revoke(&verified));
        handle
            .set_host_trust_snapshot(host.clone(), registry.to_json().expect("json"))
            .expect("load revoked records");
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::SessionOnly,
            "a revoked key must not be trusted"
        );
        handle.shutdown().expect("shutdown");
    }

    #[test]
    fn a_device_id_outside_the_key_bound_namespace_is_never_trusted() {
        let (host, _listener, _verified) = verified_listener();
        let handle = FfiCoreHandle::open("trust-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        let mut registry = HostVerifiedTrust::new(&host).expect("registry");
        let (_, _, verified) = verified_listener();
        registry.enroll(verified, "Kitchen").expect("enroll");
        handle
            .set_host_trust_snapshot(host, registry.to_json().expect("json"))
            .expect("load records");
        let plain = DeviceId::new("claimed-listener").expect("device");
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &plain).expect("state"),
            TrustState::SessionOnly,
            "a claimed plaintext ID must never receive trust"
        );
        handle.shutdown().expect("shutdown");
    }

    /// Trust loaded from the host's `SQLite` records (remediation F1): an
    /// enrolled key is trusted, and after revocation it is not.
    #[test]
    fn trust_loaded_from_the_database_follows_enrollment_and_revocation() {
        use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker, VerifiedListenerRecord};
        let (host, _listener, verified) = verified_listener();
        let device = verified.canonical_device_id();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-host-trust-{unique}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let worker =
            DatabaseWorker::start(DatabaseConfig::new(root.join("trust.sqlite3")).expect("config"))
                .expect("database worker");
        let client = worker.client();
        let handle = FfiCoreHandle::open("trust-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");

        assert_eq!(
            crate::host_control::handle_trust::load_host_trust(&handle, &client, &host)
                .expect("loads"),
            0
        );
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::SessionOnly
        );

        client
            .enroll_verified_listener(&VerifiedListenerRecord {
                host_key_fingerprint: *verified.host_fingerprint(),
                listener_key_fingerprint: *verified.fingerprint(),
                display_name: "Kitchen".to_owned(),
                enrolled_at_ms: 1,
                revoked_at_ms: None,
            })
            .expect("enrolls");
        assert_eq!(
            crate::host_control::handle_trust::load_host_trust(&handle, &client, &host)
                .expect("loads"),
            1
        );
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::Trusted
        );

        assert!(
            client
                .revoke_verified_listener(*verified.host_fingerprint(), *verified.fingerprint(), 2)
                .expect("revokes")
        );
        crate::host_control::handle_trust::load_host_trust(&handle, &client, &host)
            .expect("reloads");
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::SessionOnly,
            "a revoked key must not be trusted"
        );
        handle.shutdown().expect("shutdown");
        worker.stop_and_join().expect("database closes");
        let _ = std::fs::remove_dir_all(root);
    }

    /// "Always allow" for a verified listener writes its record and trusts its
    /// next join; a claimed legacy ID is refused and nothing is stored.
    #[test]
    fn an_always_allowed_listener_connects_as_trusted_only_while_the_record_holds() {
        use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
        let (host, _listener, verified) = verified_listener();
        let device = verified.canonical_device_id();
        let directory = std::env::temp_dir().join(format!(
            "sd-ffi-connected-trust-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let worker = DatabaseWorker::start(
            DatabaseConfig::new(directory.join("trust.sqlite3")).expect("config"),
        )
        .expect("database worker");
        let handle = FfiCoreHandle::open("connected-trust-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        let connected = |state| FfiListenerSummary {
            device_id: device.as_str().to_owned(),
            display_name: "Kitchen".to_owned(),
            trust_state: state,
            transport_state: FfiTransportState::Connected,
            synchronization: None,
            last_contact_ms: None,
            last_error: None,
        };
        assert!(
            handle
                .submit_listener_connected(connected(FfiTrustState::Trusted))
                .is_err(),
            "no record yet: a Trusted report is a claim"
        );
        crate::host_control::handle_trust::enroll_verified_listener(
            &handle,
            &worker.client(),
            &host,
            device.as_str(),
            "Kitchen",
            5,
        )
        .expect("Always allow stores the record");
        handle
            .submit_listener_connected(connected(FfiTrustState::Trusted))
            .expect("the record backs the Trusted report after Always allow");
        assert!(
            crate::verified_listeners::revoke_verified_listener(
                &worker.client(),
                &host,
                device.as_str(),
                9
            )
            .expect("revokes")
        );
        assert!(
            handle
                .submit_listener_connected(connected(FfiTrustState::Trusted))
                .is_err(),
            "a revoked record no longer backs it"
        );
        handle.shutdown().expect("shutdown");
        worker.stop_and_join().expect("database closes");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_revoke_elsewhere_reaches_a_running_host_at_its_next_join() {
        use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
        let (host, _listener, verified) = verified_listener();
        let device = verified.canonical_device_id();
        let directory = std::env::temp_dir().join(format!(
            "sd-ffi-live-revoke-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let worker = DatabaseWorker::start(
            DatabaseConfig::new(directory.join("trust.sqlite3")).expect("config"),
        )
        .expect("database worker");
        // The Settings screen writes through its own database connection.
        let settings_worker = DatabaseWorker::start(
            DatabaseConfig::new(directory.join("trust.sqlite3")).expect("config"),
        )
        .expect("second connection");
        let handle = FfiCoreHandle::open("live-revoke-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");
        crate::host_control::handle_trust::enroll_verified_listener(
            &handle,
            &worker.client(),
            &host,
            device.as_str(),
            "Kitchen",
            5,
        )
        .expect("enrols");
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::Trusted
        );

        assert!(
            crate::verified_listeners::revoke_verified_listener(
                &settings_worker.client(),
                &host,
                device.as_str(),
                9,
            )
            .expect("revokes")
        );
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::SessionOnly,
            "the running host re-reads its records at the next join"
        );

        // A host that can no longer read its records trusts nobody.
        crate::host_control::handle_trust::enroll_verified_listener(
            &handle,
            &worker.client(),
            &host,
            device.as_str(),
            "Kitchen",
            10,
        )
        .expect("re-enrols");
        worker.stop_and_join().expect("database closes");
        crate::host_control::trust_records_changed();
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::SessionOnly,
            "an unreadable record store fails closed"
        );
        handle.shutdown().expect("shutdown");
        settings_worker
            .stop_and_join()
            .expect("second connection closes");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn always_allow_enrolls_only_a_key_bound_listener() {
        use silent_disco_core::storage::{DatabaseConfig, DatabaseWorker};
        let (host, _listener, verified) = verified_listener();
        let device = verified.canonical_device_id();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sd-ffi-host-enroll-{unique}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let worker =
            DatabaseWorker::start(DatabaseConfig::new(root.join("trust.sqlite3")).expect("config"))
                .expect("database worker");
        let client = worker.client();
        let handle = FfiCoreHandle::open("trust-host".to_owned(), Arc::new(NoopObserver))
            .expect("open core");

        let refused = crate::host_control::handle_trust::enroll_verified_listener(
            &handle,
            &client,
            &host,
            "claimed-listener",
            "Claimed",
            1,
        )
        .expect_err("a claimed ID cannot be remembered");
        assert!(refused.to_string().contains("proved its key"), "{refused}");
        assert!(
            client
                .list_verified_listeners(*verified.host_fingerprint())
                .expect("lists")
                .is_empty()
        );

        crate::host_control::handle_trust::enroll_verified_listener(
            &handle,
            &client,
            &host,
            device.as_str(),
            "Kitchen",
            5,
        )
        .expect("enrolls");
        let stored = client
            .list_verified_listeners(*verified.host_fingerprint())
            .expect("lists");
        assert_eq!(stored.len(), 1);
        assert_eq!(&stored[0].listener_key_fingerprint, verified.fingerprint());
        assert_eq!(stored[0].display_name, "Kitchen");
        assert_eq!(
            crate::host_control::handle_trust::join_trust_state(&handle, &device).expect("state"),
            TrustState::Trusted,
            "the registry is reloaded after enrolment"
        );
        handle.shutdown().expect("shutdown");
        worker.stop_and_join().expect("database closes");
        let _ = std::fs::remove_dir_all(root);
    }
}
