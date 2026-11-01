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

    /// Preserves FIFO ordering between PCM and control on the SAME queue.
    /// Lossy audio always returns immediately when congested. A critical
    /// control may wait for a short bounded interval on the playback pump
    /// thread (never in a native audio callback) rather than disappearing
    /// behind a burst of already-queued PCM.
    pub(super) fn broadcast_frame(&self, frame: ProtocolFrame) -> Result<(), DesktopNetworkError> {
        enqueue_broadcast_frame(
            &self.broadcast_sender,
            &self.status.broadcast,
            &self.stop,
            frame,
        )
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

const CRITICAL_CONTROL_ENQUEUE_TIMEOUT: Duration = Duration::from_millis(500);
const CRITICAL_CONTROL_RETRY_INTERVAL: Duration = Duration::from_millis(2);

/// Audio remains lossy and nonblocking; critical controls keep their place
/// behind earlier audio on the same ordered queue. A timeout is an EXPLICIT
/// transport error, never an unobserved dropped Start/Pause/Stop/boundary.
/// The caller is the host pump/control worker, not the render callback.
fn enqueue_broadcast_frame(
    sender: &SyncSender<ProtocolFrame>,
    counters: &BroadcastCounters,
    stop: &AtomicBool,
    frame: ProtocolFrame,
) -> Result<(), DesktopNetworkError> {
    if stop.load(Ordering::Acquire) {
        return Err(DesktopNetworkError::unavailable(
            "desktop host transport is shutting down",
        ));
    }
    let critical = matches!(frame, ProtocolFrame::Control(_));
    let deadline = Instant::now() + CRITICAL_CONTROL_ENQUEUE_TIMEOUT;
    // Reserve before send so the transport worker can never dequeue an
    // uncounted frame. Roll back the one reservation on any final failure.
    let reserved_depth = counters.reserve_enqueue();
    let mut pending = frame;
    loop {
        if stop.load(Ordering::Acquire) {
            counters.record_dequeued();
            return Err(DesktopNetworkError::unavailable(
                "desktop host transport is shutting down",
            ));
        }
        match sender.try_send(pending) {
            Ok(()) => {
                counters.record_enqueue_success(reserved_depth);
                return Ok(());
            }
            Err(TrySendError::Full(frame)) => {
                if !critical || Instant::now() >= deadline {
                    counters.record_dequeued();
                    counters.queue_overflows.fetch_add(1, Ordering::Relaxed);
                    return Err(DesktopNetworkError::resource_limit(
                        "desktop host transport broadcast queue full; critical control delivery timed out or audio was dropped",
                    ));
                }
                pending = frame;
                thread::sleep(CRITICAL_CONTROL_RETRY_INTERVAL);
            }
            Err(TrySendError::Disconnected(_)) => {
                counters.record_dequeued();
                return Err(DesktopNetworkError::unavailable(
                    "desktop host transport worker is unavailable",
                ));
            }
        }
    }
}

#[cfg(test)]
mod critical_control_queue_tests {
    use super::{
        AtomicBool, BroadcastCounters, CRITICAL_CONTROL_ENQUEUE_TIMEOUT, MonotonicMillis,
        ProtocolFrame, enqueue_broadcast_frame,
    };
    use silent_disco_core::domain::{PacketSequence, SampleIndex, SessionId, StreamId};
    use silent_disco_core::protocol::{
        AudioCodec, AudioDatagram, ControlMessage, Stop,
    };
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, mpsc::sync_channel};
    use std::thread;
    use std::time::{Duration, Instant};

    fn stop(at_ms: u64) -> ProtocolFrame {
        ProtocolFrame::Control(ControlMessage::Stop(Stop {
            session_id: SessionId::new("desktop-queue-pressure").expect("session"),
            stream_id: StreamId::new("desktop-queue-stream").expect("stream"),
            host_stop_time_ms: MonotonicMillis::new(at_ms),
        }))
    }

    fn audio() -> ProtocolFrame {
        ProtocolFrame::Audio(AudioDatagram {
            session_id: SessionId::new("desktop-queue-pressure").expect("session"),
            stream_id: StreamId::new("desktop-queue-stream").expect("stream"),
            sequence: PacketSequence::new(0),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 240,
            first_sample_index: SampleIndex::new(0),
            host_presentation_time_ms: MonotonicMillis::new(1_000),
            payload: vec![0; 960],
        })
    }

    #[test]
    fn stop_survives_audio_queue_pressure_in_fifo_order() {
        let (sender, receiver) = sync_channel(1);
        let counters = Arc::new(BroadcastCounters::default());
        let stop_flag = Arc::new(AtomicBool::new(false));
        enqueue_broadcast_frame(&sender, &counters, &stop_flag, audio())
            .expect("first audio admitted");
        let pending_sender = sender.clone();
        let pending_counters = Arc::clone(&counters);
        let pending_stop = Arc::clone(&stop_flag);
        let waiter = thread::spawn(move || {
            enqueue_broadcast_frame(
                &pending_sender,
                &pending_counters,
                &pending_stop,
                stop(1_200),
            )
        });
        thread::sleep(Duration::from_millis(25));
        assert!(matches!(receiver.recv().expect("older audio"), ProtocolFrame::Audio(_)));
        counters.record_dequeued();
        waiter.join().expect("send thread").expect("critical Stop eventually admitted");
        assert!(matches!(
            receiver.recv().expect("later Stop"),
            ProtocolFrame::Control(ControlMessage::Stop(Stop {
                host_stop_time_ms,
                ..
            })) if host_stop_time_ms.get() == 1_200
        ));
        counters.record_dequeued();
        assert_eq!(counters.snapshot().queue_depth, 0);
        assert_eq!(counters.snapshot().queue_overflows, 0);
    }

    #[test]
    fn lossy_audio_does_not_block_behind_full_queue() {
        let (sender, _receiver) = sync_channel(1);
        let counters = BroadcastCounters::default();
        let stop_flag = AtomicBool::new(false);
        enqueue_broadcast_frame(&sender, &counters, &stop_flag, audio())
            .expect("first audio");
        let start = Instant::now();
        assert!(enqueue_broadcast_frame(&sender, &counters, &stop_flag, audio()).is_err());
        assert!(start.elapsed() < Duration::from_millis(150));
        assert_eq!(counters.snapshot().queue_depth, 1);
        assert_eq!(counters.snapshot().queue_overflows, 1);
    }

    #[test]
    fn stalled_worker_refuses_critical_control_after_finite_wait() {
        let (sender, _receiver) = sync_channel(1);
        let counters = BroadcastCounters::default();
        let stop_flag = AtomicBool::new(false);
        enqueue_broadcast_frame(&sender, &counters, &stop_flag, audio())
            .expect("fill queue");
        let start = Instant::now();
        assert!(enqueue_broadcast_frame(&sender, &counters, &stop_flag, stop(1_200)).is_err());
        assert!(start.elapsed() >= CRITICAL_CONTROL_ENQUEUE_TIMEOUT);
        assert_eq!(counters.snapshot().queue_depth, 1);
        assert_eq!(counters.snapshot().queue_overflows, 1);
    }

    #[test]
    fn stopped_transport_interrupts_control_wait_without_leaking_queue_depth() {
        let (sender, _receiver) = sync_channel(1);
        let counters = Arc::new(BroadcastCounters::default());
        let stop_flag = Arc::new(AtomicBool::new(false));
        enqueue_broadcast_frame(&sender, &counters, &stop_flag, audio())
            .expect("fill queue");
        let sender_wait = sender.clone();
        let counters_wait = Arc::clone(&counters);
        let stop_wait = Arc::clone(&stop_flag);
        let waiter = thread::spawn(move || {
            enqueue_broadcast_frame(&sender_wait, &counters_wait, &stop_wait, stop(1_300))
        });
        thread::sleep(Duration::from_millis(25));
        stop_flag.store(true, Ordering::Release);
        assert!(waiter.join().expect("waiter thread").is_err());
        assert_eq!(counters.snapshot().queue_depth, 1);
        assert_eq!(counters.snapshot().queue_overflows, 0);
    }
}
