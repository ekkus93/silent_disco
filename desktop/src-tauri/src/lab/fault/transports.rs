struct LabBaseTransportClock {
    clock: Arc<LabClock>,
}

impl TransportClock for LabBaseTransportClock {
    fn now(&self) -> MonotonicMillis {
        self.clock.now()
    }
}

struct ChannelPrngs {
    synchronization: DeterministicPrng,
    audio: DeterministicPrng,
}

impl ChannelPrngs {
    fn should_drop(&mut self, event: &TransportEvent, loss_permille: u16) -> bool {
        should_drop_event(
            event,
            loss_permille,
            &mut self.synchronization,
            &mut self.audio,
        )
    }
}

struct LabFaultHostTransport {
    inner: Box<dyn HostTransportNode>,
    controller: LabFaultController,
    prngs: ChannelPrngs,
}

impl HostTransportNode for LabFaultHostTransport {
    fn endpoint(&self) -> silent_disco_core::runtime::NetworkEndpoint {
        self.inner.endpoint()
    }

    fn authorize_peer(
        &self,
        device_id: &silent_disco_core::domain::DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        self.inner.authorize_peer(device_id, routes)
    }

    fn authorize_peer_ports(
        &self,
        device_id: &silent_disco_core::domain::DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError> {
        self.inner
            .authorize_peer_ports(device_id, sync_port, audio_port)
    }

    fn disconnect_peer(
        &self,
        device_id: &silent_disco_core::domain::DeviceId,
    ) -> Result<(), TransportError> {
        self.inner.disconnect_peer(device_id)
    }

    fn send_pending_control(
        &self,
        device_id: &silent_disco_core::domain::DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.send_pending_control(device_id, message)
    }

    fn send_control(
        &self,
        device_id: &silent_disco_core::domain::DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.send_control(device_id, message)
    }

    fn broadcast_control(
        &self,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.broadcast_control(message)
    }

    fn broadcast_sync(
        &self,
        frame: &silent_disco_core::protocol::ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.broadcast_sync(frame)
    }

    fn broadcast_audio(
        &self,
        frame: &silent_disco_core::protocol::ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.broadcast_audio(frame)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let started = Instant::now();
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(TransportError::timeout(
                    TransportChannel::Runtime,
                    "Lab Mode dynamic receive fault timed out after dropping datagrams",
                ));
            }
            let event = self.inner.recv_event(remaining)?;
            let packet_trace = self.controller.record_packet(&event)?;
            let profile = self.controller.snapshot()?;
            let should_drop = self.prngs.should_drop(&event, profile.loss_permille);
            if is_faultable_datagram(&event) {
                let decision = if should_drop {
                    RecordedFaultDecision::Drop
                } else {
                    RecordedFaultDecision::Pass
                };
                let decided_at_ms = event_channel_and_time(&event).1;
                self.controller.record_decision(
                    packet_trace.as_ref(),
                    profile,
                    decision,
                    decided_at_ms,
                    None,
                )?;
            }
            if !should_drop {
                return Ok(event);
            }
        }
    }

    fn counters(&self) -> TransportCounters {
        self.inner.counters()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.inner.shutdown()
    }
}

struct HeldEvent {
    deadline_ms: u64,
    event: TransportEvent,
    trace: Option<PacketTraceIdentity>,
    profile: LabReceiveFaultProfile,
}

impl PartialEq for HeldEvent {
    fn eq(&self, other: &Self) -> bool {
        self.deadline_ms == other.deadline_ms
    }
}

impl Eq for HeldEvent {}

impl PartialOrd for HeldEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeldEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deadline_ms.cmp(&other.deadline_ms)
    }
}

struct HeldEvents {
    synchronization_prng: DeterministicPrng,
    audio_prng: DeterministicPrng,
    queue: BinaryHeap<Reverse<HeldEvent>>,
}

impl HeldEvents {
    fn should_drop(&mut self, channel: TransportChannel, loss_permille: u16) -> bool {
        should_drop_channel(
            channel,
            loss_permille,
            &mut self.synchronization_prng,
            &mut self.audio_prng,
        )
    }

    fn deadline(
        &mut self,
        channel: TransportChannel,
        config: LabLatencyConfig,
        arrived_at_ms: u64,
    ) -> u64 {
        let prng = match channel {
            TransportChannel::Synchronization => &mut self.synchronization_prng,
            TransportChannel::Audio => &mut self.audio_prng,
            TransportChannel::Control | TransportChannel::Runtime => return arrived_at_ms,
        };
        compute_deadline(config, prng, arrived_at_ms)
    }
}

struct LabLatencyListenerTransport {
    inner: Box<dyn ListenerTransportNode>,
    clock: Arc<LabClock>,
    delivery_clock: Arc<dyn TransportClock>,
    controller: LabFaultController,
    held: Mutex<HeldEvents>,
}

impl ListenerTransportNode for LabLatencyListenerTransport {
    fn local_routes(&self) -> ListenerDatagramRoutes {
        self.inner.local_routes()
    }

    fn send_control(&self, message: &ControlMessage) -> Result<TransportDelivery, TransportError> {
        self.inner.send_control(message)
    }

    fn send_sync_request(
        &self,
        request: &SyncRequest,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.send_sync_request(request)
    }

    fn recv_event(&self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let started = Instant::now();
        let mut held = self.held.lock().map_err(|_| TransportError {
            kind: TransportErrorKind::WorkerPanicked,
            channel: TransportChannel::Runtime,
            message: "Lab Mode latency fault state mutex was poisoned".to_owned(),
        })?;

        if let Some(released) = take_due(&mut held.queue, self.clock.now().get()) {
            self.controller.record_decision(
                released.trace.as_ref(),
                released.profile,
                RecordedFaultDecision::Release,
                self.clock.now().get(),
                Some(released.deadline_ms),
            )?;
            return Ok(self.stamp_delivery_time(released.event));
        }

        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(TransportError::timeout(
                    TransportChannel::Runtime,
                    "Lab Mode dynamic receive fault timed out while holding or dropping datagrams",
                ));
            }
            let event = self.inner.recv_event(remaining)?;
            let packet_trace = self.controller.record_packet(&event)?;
            let (channel, arrived_at_ms) = event_channel_and_time(&event);
            if !matches!(
                channel,
                Some(TransportChannel::Synchronization | TransportChannel::Audio)
            ) {
                return Ok(self.stamp_delivery_time(event));
            }

            let profile = self.controller.snapshot()?;
            let channel = channel.expect("datagram channel was matched above");
            if held.should_drop(channel, profile.loss_permille) {
                self.controller.record_decision(
                    packet_trace.as_ref(),
                    profile,
                    RecordedFaultDecision::Drop,
                    self.clock.now().get(),
                    None,
                )?;
                continue;
            }

            let deadline_ms = held.deadline(channel, profile.latency, arrived_at_ms);
            if deadline_ms <= self.clock.now().get() {
                self.controller.record_decision(
                    packet_trace.as_ref(),
                    profile,
                    RecordedFaultDecision::Pass,
                    self.clock.now().get(),
                    Some(deadline_ms),
                )?;
                return Ok(self.stamp_delivery_time(event));
            }
            self.controller.record_decision(
                packet_trace.as_ref(),
                profile,
                RecordedFaultDecision::Hold,
                self.clock.now().get(),
                Some(deadline_ms),
            )?;
            held.queue.push(Reverse(HeldEvent {
                deadline_ms,
                event,
                trace: packet_trace,
                profile,
            }));
            return Err(TransportError::timeout(
                TransportChannel::Runtime,
                "Lab Mode latency fault is holding the newest event for a later virtual deadline",
            ));
        }
    }

    fn counters(&self) -> TransportCounters {
        self.inner.counters()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.inner.shutdown()
    }
}

impl LabLatencyListenerTransport {
    fn stamp_delivery_time(&self, mut event: TransportEvent) -> TransportEvent {
        let received_at = match &mut event {
            TransportEvent::PeerAccepted { received_at, .. }
            | TransportEvent::PeerAuthorized { received_at, .. }
            | TransportEvent::FrameReceived { received_at, .. }
            | TransportEvent::PeerDisconnected { received_at, .. }
            | TransportEvent::Rejected { received_at, .. } => received_at,
        };
        *received_at = self.delivery_clock.now();
        event
    }
}

