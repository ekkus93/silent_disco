pub(crate) struct DesktopHostTransportRuntime {
    endpoint: silent_disco_core::runtime::NetworkEndpoint,
    stop: Arc<AtomicBool>,
    effect_sender: SyncSender<TransportEffect>,
    broadcast_sender: SyncSender<ProtocolFrame>,
    status: Arc<SharedStatus>,
    clock: Arc<dyn TransportClock>,
    worker: Option<JoinHandle<Result<(), DesktopNetworkError>>>,
}

impl DesktopHostTransportRuntime {
    pub(super) fn start(
        node: Box<dyn HostTransportNode>,
        advertisement: SessionAdvertisement,
        sink: Arc<dyn DesktopHostTransportEventSink>,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, DesktopNetworkError> {
        let endpoint = node.endpoint();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(SharedStatus {
            running: AtomicBool::new(true),
            last_error: Mutex::new(None),
            broadcast: BroadcastCounters::default(),
        });
        let (effect_sender, effect_receiver) = sync_channel(TRANSPORT_EFFECT_QUEUE_CAPACITY);
        let (broadcast_sender, broadcast_receiver) =
            sync_channel(usize::from(BROADCAST_FRAME_QUEUE_CAPACITY));
        let worker_stop = Arc::clone(&stop);
        let worker_status = Arc::clone(&status);
        let worker_clock = Arc::clone(&clock);
        let worker = thread::Builder::new()
            .name("silent-disco-desktop-host-transport".to_owned())
            .spawn(move || {
                run_transport_worker(
                    node,
                    &advertisement,
                    &sink,
                    &effect_receiver,
                    &broadcast_receiver,
                    &worker_stop,
                    &worker_status,
                    &worker_clock,
                )
            })
            .map_err(|error| {
                DesktopNetworkError::unavailable(format!(
                    "failed to start desktop host transport worker: {error}"
                ))
            })?;
        Ok(Self {
            endpoint,
            stop,
            effect_sender,
            broadcast_sender,
            status,
            clock,
            worker: Some(worker),
        })
    }

    /// Enqueues one control/sync/audio frame for the worker thread to
    /// broadcast on its next tick. Non-blocking: a full queue or a shut-down
    /// worker is reported as an error rather than stalling the caller (a
    /// playback pump thread), since audio delivery is inherently best-effort.
    pub(super) fn broadcast_frame(&self, frame: ProtocolFrame) -> Result<(), DesktopNetworkError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(DesktopNetworkError::unavailable(
                "desktop host transport is shutting down",
            ));
        }
        // Reserve depth before publishing the frame so the worker cannot
        // dequeue it before accounting sees it. A failed `try_send` rolls the
        // reservation back immediately.
        let reserved_depth = self.status.broadcast.reserve_enqueue();
        match self.broadcast_sender.try_send(frame) {
            Ok(()) => {
                self.status
                    .broadcast
                    .record_enqueue_success(reserved_depth);
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                self.status.broadcast.record_dequeued();
                self.status
                    .broadcast
                    .queue_overflows
                    .fetch_add(1, Ordering::Relaxed);
                Err(DesktopNetworkError::resource_limit(
                    "desktop host transport broadcast queue is full",
                ))
            }
            Err(TrySendError::Disconnected(_)) => {
                self.status.broadcast.record_dequeued();
                Err(DesktopNetworkError::unavailable(
                    "desktop host transport worker is unavailable",
                ))
            }
        }
    }

    #[must_use]
    pub(crate) const fn endpoint(&self) -> silent_disco_core::runtime::NetworkEndpoint {
        self.endpoint
    }

    #[must_use]
    pub(crate) fn observed_at(&self) -> MonotonicMillis {
        self.clock.now()
    }

    pub(crate) fn dispatch(&self, effect: TransportEffect) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        if self.stop.load(Ordering::Acquire) {
            return Err(DesktopNetworkError::unavailable(
                "desktop host transport is shutting down",
            )
            .core_error(Some(operation_id)));
        }
        match self.effect_sender.try_send(effect) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(DesktopNetworkError::resource_limit(
                "desktop host transport effect queue is full",
            )
            .core_error(Some(operation_id))),
            Err(TrySendError::Disconnected(_)) => Err(DesktopNetworkError::unavailable(
                "desktop host transport effect worker is unavailable",
            )
            .core_error(Some(operation_id))),
        }
    }

    pub(super) fn status(&self) -> Result<HostTransportStatus, DesktopNetworkError> {
        let last_error = self
            .status
            .last_error
            .lock()
            .map_err(|_| {
                DesktopNetworkError::invalid_state(
                    "desktop host transport status mutex was poisoned",
                )
            })?
            .clone();
        Ok(HostTransportStatus {
            running: self.status.running.load(Ordering::Acquire),
            last_error,
            broadcast: self.status.broadcast.snapshot(),
        })
    }

    #[cfg(test)]
    pub(super) fn stop_worker_for_test(&mut self) -> Result<(), DesktopNetworkError> {
        self.stop.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        match worker.join() {
            Ok(result) => result,
            Err(_) => Err(DesktopNetworkError::unavailable(
                "desktop host transport worker panicked during test shutdown",
            )),
        }
    }

    pub(super) fn shutdown(mut self) -> Result<(), DesktopNetworkError> {
        self.stop.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        match worker.join() {
            Ok(result) => result,
            Err(_) => Err(DesktopNetworkError::unavailable(
                "desktop host transport worker panicked during shutdown",
            )),
        }
    }
}

