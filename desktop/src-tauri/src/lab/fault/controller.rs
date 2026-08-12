#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LabLatencyConfig {
    pub(crate) fixed_latency_ms: u64,
    pub(crate) jitter_ms: u64,
    pub(crate) seed: u64,
}

#[derive(Debug, Clone, Copy)]
struct LabReceiveFaultProfile {
    latency: LabLatencyConfig,
    loss_permille: u16,
}

#[derive(Clone)]
struct LabTraceContext {
    receiver_node: String,
    recorder: TransportTraceRecorder,
}

#[derive(Clone)]
pub(crate) struct LabFaultController {
    profile: Arc<Mutex<LabReceiveFaultProfile>>,
    trace: Option<LabTraceContext>,
}

impl LabFaultController {
    #[must_use]
    pub(crate) fn new(config: LabLatencyConfig, loss_permille: u16) -> Self {
        Self::new_internal(config, loss_permille, None)
    }

    #[must_use]
    pub(crate) fn new_traced(
        config: LabLatencyConfig,
        loss_permille: u16,
        receiver_node: String,
        recorder: TransportTraceRecorder,
    ) -> Self {
        Self::new_internal(
            config,
            loss_permille,
            Some(LabTraceContext {
                receiver_node,
                recorder,
            }),
        )
    }

    fn new_internal(
        config: LabLatencyConfig,
        loss_permille: u16,
        trace: Option<LabTraceContext>,
    ) -> Self {
        Self {
            profile: Arc::new(Mutex::new(LabReceiveFaultProfile {
                latency: config,
                loss_permille,
            })),
            trace,
        }
    }

    /// Atomically replaces the live receive-fault profile.
    ///
    /// # Errors
    ///
    /// Returns a visible runtime error if a prior panic poisoned the shared
    /// profile state. A poisoned profile is never silently reused.
    pub(crate) fn update_checked(
        &self,
        fixed_latency_ms: u64,
        jitter_ms: u64,
        loss_permille: u16,
    ) -> Result<(), TransportError> {
        let mut profile = self.profile.lock().map_err(|_| fault_state_poisoned())?;
        profile.latency.fixed_latency_ms = fixed_latency_ms;
        profile.latency.jitter_ms = jitter_ms;
        profile.loss_permille = loss_permille;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn update(&self, fixed_latency_ms: u64, jitter_ms: u64, loss_permille: u16) {
        self.update_checked(fixed_latency_ms, jitter_ms, loss_permille)
            .expect("Lab fault profile state must not be poisoned in a test");
    }

    fn snapshot(&self) -> Result<LabReceiveFaultProfile, TransportError> {
        self.profile
            .lock()
            .map(|profile| *profile)
            .map_err(|_| fault_state_poisoned())
    }

    fn record_packet(
        &self,
        event: &TransportEvent,
    ) -> Result<Option<PacketTraceIdentity>, TransportError> {
        let Some(trace) = &self.trace else {
            return Ok(None);
        };
        trace
            .recorder
            .record_packet(&trace.receiver_node, event)
            .map_err(|error| trace_transport_error(&error))
    }

    fn record_decision(
        &self,
        packet: Option<&PacketTraceIdentity>,
        profile: LabReceiveFaultProfile,
        decision: RecordedFaultDecision,
        decided_at_ms: u64,
        deadline_ms: Option<u64>,
    ) -> Result<(), TransportError> {
        let (Some(trace), Some(packet)) = (&self.trace, packet) else {
            return Ok(());
        };
        trace
            .recorder
            .record_fault_decision(
                packet,
                RecordedFaultProfile {
                    fixed_latency_ms: profile.latency.fixed_latency_ms,
                    jitter_ms: profile.latency.jitter_ms,
                    loss_permille: profile.loss_permille,
                },
                decision,
                decided_at_ms,
                deadline_ms,
            )
            .map_err(|error| trace_transport_error(&error))
    }
}

fn fault_state_poisoned() -> TransportError {
    TransportError {
        kind: TransportErrorKind::WorkerPanicked,
        channel: TransportChannel::Runtime,
        message: "Lab Mode receive-fault profile mutex was poisoned".to_owned(),
    }
}

fn trace_transport_error(error: &TransportTraceError) -> TransportError {
    let kind = match error {
        TransportTraceError::Encode(_) => TransportErrorKind::Protocol,
        TransportTraceError::StatePoisoned
        | TransportTraceError::SequenceExhausted
        | TransportTraceError::DropCounterExhausted
        | TransportTraceError::LengthOutOfRange => TransportErrorKind::WorkerPanicked,
    };
    TransportError {
        kind,
        channel: TransportChannel::Runtime,
        message: format!("Lab Mode transport trace failed: {error}"),
    }
}

#[derive(Clone)]
pub(crate) struct LabLatencyTransportFactory<F> {
    inner: F,
    clock: Arc<LabClock>,
    controller: LabFaultController,
}

impl<F> LabLatencyTransportFactory<F> {
    #[must_use]
    pub(crate) fn new(inner: F, clock: Arc<LabClock>, config: LabLatencyConfig) -> Self {
        Self::new_dynamic(inner, clock, LabFaultController::new(config, 0))
    }

    #[must_use]
    pub(crate) fn new_dynamic(
        inner: F,
        clock: Arc<LabClock>,
        controller: LabFaultController,
    ) -> Self {
        Self {
            inner,
            clock,
            controller,
        }
    }
}

impl<F: TransportFactory> TransportFactory for LabLatencyTransportFactory<F> {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        let inner = self.inner.bind_host(config, clock)?;
        let seed = self.controller.snapshot()?.latency.seed;
        Ok(Box::new(LabFaultHostTransport {
            inner,
            controller: self.controller.clone(),
            prngs: ChannelPrngs {
                synchronization: DeterministicPrng::new(seed),
                audio: DeterministicPrng::new(seed ^ 0xA5A5_A5A5_A5A5_A5A5),
            },
        }))
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        let inner_clock: Arc<dyn TransportClock> = Arc::new(LabBaseTransportClock {
            clock: Arc::clone(&self.clock),
        });
        let inner = self.inner.connect_listener(config, inner_clock)?;
        let seed = self.controller.snapshot()?.latency.seed;
        Ok(Box::new(LabLatencyListenerTransport {
            inner,
            clock: Arc::clone(&self.clock),
            delivery_clock: clock,
            controller: self.controller.clone(),
            held: Mutex::new(HeldEvents {
                synchronization_prng: DeterministicPrng::new(seed),
                audio_prng: DeterministicPrng::new(seed ^ 0xA5A5_A5A5_A5A5_A5A5),
                queue: BinaryHeap::new(),
            }),
        }))
    }
}
