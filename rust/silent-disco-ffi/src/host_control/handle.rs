use super::conversions::network_endpoint;
use super::types::{
    FfiAppRole, FfiBridgeError, FfiCommandReceipt, FfiCoreError, FfiCoreObserver, FfiCoreSnapshot,
    FfiDeliveryReport, FfiHostDraft, FfiJoinRequestInput, FfiListenerSummary,
    FfiPlatformCompletion, FfiPlaybackState, FfiSessionAdvertisement, FfiTransportState,
    FfiTuningPatch, FfiTuningSettings,
};
use silent_disco_core::domain::{
    ApprovalMode, DeviceId, MonotonicMillis, OperationId, RequestId, SessionId, SyncConfidence,
    TransportState, TrustState,
};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::runtime::{
    AudioEvent, AudioSourcePatch, CoreActorConfig, CoreActorHandle, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, HostDraftPatch, InviteCodePatch, JoinRequestSummary,
    ListenerSummary, PlatformEvent, SessionAdvertisement, SnapshotRevision, StorageCompletion,
    StorageEvent, SynchronizationSummary, TransportEvent,
};
use std::sync::{Arc, Mutex};

#[derive(uniffi::Object)]
pub struct FfiCoreHandle {
    runtime: Mutex<Option<CoreActorRuntime>>,
    handle: CoreActorHandle,
}

#[allow(
    clippy::missing_errors_doc,
    reason = "all exported methods use the uniform validated FfiBridgeError contract"
)]
#[uniffi::export]
impl FfiCoreHandle {
    #[uniffi::constructor]
    pub fn open(
        local_device_id: String,
        observer: Arc<dyn FfiCoreObserver>,
    ) -> Result<Arc<Self>, FfiBridgeError> {
        let device_id = DeviceId::new(local_device_id)
            .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        let runtime = CoreActorRuntime::start(
            CoreActorConfig::new(device_id),
            move |notification: CoreNotification| {
                observer
                    .on_notification(notification.into())
                    .map_err(|_| observer_callback_error())
            },
        )?;
        let handle = runtime.handle();
        Ok(Arc::new(Self {
            runtime: Mutex::new(Some(runtime)),
            handle,
        }))
    }

    pub fn current_snapshot(&self) -> Result<FfiCoreSnapshot, FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .current_snapshot()
            .map(Into::into)
            .map_err(Into::into)
    }

    pub fn select_role(
        &self,
        expected_revision: u64,
        role: FfiAppRole,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::SelectRole { role: role.into() },
        )
    }

    pub fn update_host_draft(
        &self,
        expected_revision: u64,
        draft: FfiHostDraft,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.ensure_open()?;
        let current = self.handle.current_snapshot()?;
        if FfiTuningSettings::from(current.tuning) != draft.tuning {
            return Err(FfiBridgeError::Core(
                "host draft tuning must be changed with update_tuning".to_owned(),
            ));
        }
        let invite_code = match draft.invite_code {
            Some(code) => InviteCodePatch::Set(code),
            None => InviteCodePatch::Clear,
        };
        let audio_source = match draft.audio_source {
            Some(source) => AudioSourcePatch::Set(source.try_into()?),
            None => AudioSourcePatch::Clear,
        };
        self.submit_command(
            expected_revision,
            CoreCommand::UpdateHostDraft(HostDraftPatch {
                session_name: Some(draft.session_name),
                approval_mode: Some(draft.approval_mode.into()),
                invite_code,
                audio_source,
                remember_approved_devices: Some(draft.remember_approved_devices),
            }),
        )
    }

    pub fn update_tuning(
        &self,
        expected_revision: u64,
        patch: FfiTuningPatch,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::UpdateTuning(patch.try_into()?),
        )
    }

    pub fn create_host_session(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::CreateHostSession)
    }

    pub fn approve_join(
        &self,
        expected_revision: u64,
        request_id: String,
        remember_for_future: bool,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::ApproveJoin {
                request_id: request_id_from_string(request_id)?,
                remember_for_future,
            },
        )
    }

    pub fn reject_join(
        &self,
        expected_revision: u64,
        request_id: String,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::RejectJoin {
                request_id: request_id_from_string(request_id)?,
            },
        )
    }

    pub fn remove_listener(
        &self,
        expected_revision: u64,
        listener_id: String,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::RemoveListener {
                listener_id: device_id_from_string(listener_id)?,
            },
        )
    }

    pub fn end_host_session(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::EndHostSession)
    }

    pub fn retry_recoverable_failure(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::RetryRecoverableFailure)
    }

    pub fn start_discovery(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::StartDiscovery)
    }

    pub fn stop_discovery(
        &self,
        expected_revision: u64,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::StopDiscovery)
    }

    pub fn select_session(
        &self,
        expected_revision: u64,
        session_id: String,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(
            expected_revision,
            CoreCommand::SelectSession {
                session_id: session_id_from_string(session_id)?,
            },
        )
    }

    pub fn submit_join(
        &self,
        expected_revision: u64,
        invite_code: Option<String>,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::SubmitJoin { invite_code })
    }

    pub fn cancel_join(&self, expected_revision: u64) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.submit_command(expected_revision, CoreCommand::CancelJoin)
    }

    pub fn platform_operation_succeeded(
        &self,
        operation_id: String,
        completion: FfiPlatformCompletion,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_platform_event(PlatformEvent::OperationSucceeded {
                operation_id: operation_id_from_string(operation_id)?,
                completion: completion.try_into()?,
            })?;
        Ok(())
    }

    pub fn platform_operation_failed(
        &self,
        operation_id: String,
        message: String,
        retryable: bool,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        let operation_id = operation_id_from_string(operation_id)?;
        self.handle
            .submit_platform_event(PlatformEvent::OperationFailed {
                operation_id: operation_id.clone(),
                error: input_error(
                    CoreErrorCode::PlatformOperationFailed,
                    message,
                    retryable,
                    Some(operation_id),
                )?,
            })?;
        Ok(())
    }

    pub fn transport_state_changed(&self, state: FfiTransportState) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::StateChanged(state.into()))?;
        Ok(())
    }

    pub fn submit_session_discovered(
        &self,
        session: FfiSessionAdvertisement,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        let endpoint = match (
            session.address,
            session.control_port,
            session.sync_port,
            session.audio_port,
        ) {
            (Some(address), Some(control_port), Some(sync_port), Some(audio_port)) => Some(
                network_endpoint(&address, control_port, sync_port, audio_port)?,
            ),
            _ => None,
        };
        let advertisement = SessionAdvertisement::new(
            session_id_from_string(session.session_id)?,
            device_id_from_string(session.host_device_id)?,
            session.session_name,
            session.approval_mode.into(),
            session.protocol_version,
            endpoint,
        )
        .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        self.handle
            .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement))?;
        Ok(())
    }

    pub fn submit_session_expired(&self, session_id: String) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_platform_event(PlatformEvent::SessionExpired {
                session_id: session_id_from_string(session_id)?,
            })?;
        Ok(())
    }

    pub fn submit_awaiting_approval(&self) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::AwaitingApproval)?;
        Ok(())
    }

    pub fn submit_join_approved(&self, trusted_for_future: bool) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::JoinApproved { trusted_for_future })?;
        Ok(())
    }

    pub fn submit_join_rejected(&self, reason: String) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::JoinRejected { reason })?;
        Ok(())
    }

    pub fn submit_join_request(&self, request: FfiJoinRequestInput) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        let snapshot = self.handle.current_snapshot()?;
        let invite_code_valid = match snapshot.host_draft.approval_mode {
            ApprovalMode::InviteCode => {
                snapshot.host_draft.invite_code.as_deref() == request.invite_code.as_deref()
            }
            ApprovalMode::Manual | ApprovalMode::TrustedDevices => true,
        };
        let request = JoinRequestSummary::new(
            request_id_from_string(request.request_id)?,
            device_id_from_string(request.device_id)?,
            request.display_name,
            TrustState::try_from(request.trust_state)?,
            invite_code_valid,
            MonotonicMillis::new(request.received_at_ms),
        )
        .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        self.handle
            .submit_transport_event(TransportEvent::JoinRequested(request))?;
        Ok(())
    }

    pub fn submit_listener_connected(
        &self,
        listener: FfiListenerSummary,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        let mut summary = ListenerSummary::new(
            device_id_from_string(listener.device_id)?,
            listener.display_name,
            TrustState::try_from(listener.trust_state)?,
            TransportState::from(listener.transport_state),
        )
        .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        summary.synchronization = listener
            .synchronization
            .map(|value| {
                let confidence = SyncConfidence::from_wire_name(&value.confidence)
                    .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
                SynchronizationSummary::new(
                    confidence,
                    value.offset_ms,
                    value.round_trip_ms,
                    value.drift_ppm,
                )
                .map_err(|error| FfiBridgeError::Core(error.to_string()))
            })
            .transpose()?;
        summary.last_contact = listener.last_contact_ms.map(MonotonicMillis::new);
        summary.last_error = listener
            .last_error
            .map(|error| input_transport_error(error.message, error.retryable))
            .transpose()?;
        self.handle
            .submit_transport_event(TransportEvent::ListenerConnected(summary))?;
        Ok(())
    }

    pub fn submit_listener_disconnected(
        &self,
        device_id: String,
        error: Option<FfiCoreError>,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::ListenerDisconnected {
                device_id: device_id_from_string(device_id)?,
                error: error
                    .map(|value| input_transport_error(value.message, value.retryable))
                    .transpose()?,
            })?;
        Ok(())
    }

    pub fn transport_delivery_completed(
        &self,
        operation_id: String,
        report: FfiDeliveryReport,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::DeliveryCompleted {
                operation_id: operation_id_from_string(operation_id)?,
                report: report.try_into()?,
            })?;
        Ok(())
    }

    pub fn transport_failed(&self, message: String, retryable: bool) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_transport_event(TransportEvent::Failed(input_transport_error(
                message, retryable,
            )?))?;
        Ok(())
    }

    pub fn settings_saved(&self, operation_id: String) -> Result<(), FfiBridgeError> {
        self.submit_storage_success(operation_id, StorageCompletion::SettingsSaved)
    }

    pub fn trusted_device_updated(
        &self,
        operation_id: String,
        device_id: String,
    ) -> Result<(), FfiBridgeError> {
        self.submit_storage_success(
            operation_id,
            StorageCompletion::TrustedDeviceUpdated {
                device_id: device_id_from_string(device_id)?,
            },
        )
    }

    pub fn storage_operation_failed(
        &self,
        operation_id: String,
        message: String,
        retryable: bool,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        let operation_id = operation_id_from_string(operation_id)?;
        self.handle
            .submit_storage_event(StorageEvent::OperationFailed {
                operation_id: operation_id.clone(),
                error: input_error(
                    CoreErrorCode::StorageWriteFailed,
                    message,
                    retryable,
                    Some(operation_id),
                )?,
            })?;
        Ok(())
    }

    pub fn playback_state_changed(&self, state: FfiPlaybackState) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_audio_event(AudioEvent::PlaybackStateChanged(state.into()))?;
        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), FfiBridgeError> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| FfiBridgeError::Closed("core runtime lock was poisoned".to_owned()))?
            .take()
            .ok_or_else(|| FfiBridgeError::Closed("core runtime is already closed".to_owned()))?;
        runtime.shutdown().map_err(Into::into)
    }
}

impl FfiCoreHandle {
    fn ensure_open(&self) -> Result<(), FfiBridgeError> {
        let open = self
            .runtime
            .lock()
            .map_err(|_| FfiBridgeError::Closed("core runtime lock was poisoned".to_owned()))?
            .is_some();
        if open {
            Ok(())
        } else {
            Err(FfiBridgeError::Closed("core runtime is closed".to_owned()))
        }
    }

    fn submit_command(
        &self,
        expected_revision: u64,
        command: CoreCommand,
    ) -> Result<FfiCommandReceipt, FfiBridgeError> {
        self.ensure_open()?;
        let request = CoreCommandRequest::new(SnapshotRevision::new(expected_revision), command)
            .map_err(|error| FfiBridgeError::Core(error.to_string()))?;
        self.handle
            .submit_command(request)
            .map(Into::into)
            .map_err(Into::into)
    }

    fn submit_storage_success(
        &self,
        operation_id: String,
        completion: StorageCompletion,
    ) -> Result<(), FfiBridgeError> {
        self.ensure_open()?;
        self.handle
            .submit_storage_event(StorageEvent::OperationSucceeded {
                operation_id: operation_id_from_string(operation_id)?,
                completion,
            })?;
        Ok(())
    }
}

impl Drop for FfiCoreHandle {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.get_mut().ok().and_then(Option::take) else {
            return;
        };
        if let Err(error) = runtime.shutdown() {
            eprintln!("silent-disco UniFFI emergency shutdown failed: {error}");
        }
    }
}

fn request_id_from_string(value: String) -> Result<RequestId, FfiBridgeError> {
    RequestId::new(value).map_err(|error| FfiBridgeError::Core(error.to_string()))
}

fn device_id_from_string(value: String) -> Result<DeviceId, FfiBridgeError> {
    DeviceId::new(value).map_err(|error| FfiBridgeError::Core(error.to_string()))
}

fn session_id_from_string(value: String) -> Result<SessionId, FfiBridgeError> {
    SessionId::new(value).map_err(|error| FfiBridgeError::Core(error.to_string()))
}

fn operation_id_from_string(value: String) -> Result<OperationId, FfiBridgeError> {
    OperationId::new(value).map_err(|error| FfiBridgeError::Core(error.to_string()))
}

fn input_transport_error(message: String, retryable: bool) -> Result<CoreError, FfiBridgeError> {
    input_error(
        CoreErrorCode::TransportConnectionLost,
        message,
        retryable,
        None,
    )
}

fn input_error(
    code: CoreErrorCode,
    message: String,
    retryable: bool,
    operation_id: Option<OperationId>,
) -> Result<CoreError, FfiBridgeError> {
    CoreError::new(code, message, ErrorSeverity::Error, retryable, operation_id)
        .map_err(|error| FfiBridgeError::Core(error.to_string()))
}

fn observer_callback_error() -> CoreError {
    CoreError::new(
        CoreErrorCode::FfiCallbackFailed,
        "foreign core observer rejected a notification",
        ErrorSeverity::Fatal,
        true,
        None,
    )
    .expect("static observer callback error must satisfy the bounded error contract")
}

#[cfg(test)]
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
