//! Lab-clock-aware receive fault injection (Block 39.2 latency/jitter and
//! Block 40 mid-run fault mutation), layered on the shared virtual transport.
//!
//! `LabFaultController` is deliberately mutable while the transport is live:
//! scenario steps can atomically replace the receive-side latency, jitter,
//! and loss profile for a node without rebuilding its transport. Datagrams
//! already held for a latency deadline retain the deadline computed when they
//! arrived; the new profile applies to subsequent receives. That keeps fault
//! changes deterministic and avoids retroactively moving in-flight packets.
//!
//! Listener latency/jitter uses the shared [`crate::lab::clock::LabClock`],
//! never wall-clock sleep. Host-side latency/jitter remains unsupported (the
//! same policy as before Block 40), but host-side receive loss is driven by
//! the same controller so a link targeting a host can still mutate loss.

use super::clock::LabClock;
use silent_disco_core::domain::MonotonicMillis;
use silent_disco_core::protocol::{ControlMessage, SyncRequest};
use silent_disco_core::transport::{
    DeterministicPrng, HostTransportConfig, HostTransportNode, ListenerDatagramRoutes,
    ListenerTransportConfig, ListenerTransportNode, TransportChannel, TransportClock,
    TransportCounters, TransportDelivery, TransportError, TransportErrorKind, TransportEvent,
    TransportFactory,
};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Deterministic latency/jitter configuration. Applies only to datagram
/// channels (`Synchronization`/`Audio`); control/runtime events are released
/// immediately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LabLatencyConfig {
    pub(crate) fixed_latency_ms: u64,
    pub(crate) jitter_ms: u64,
    pub(crate) seed: u64,
}

/// Shared live receive-fault settings for one Lab node.
#[derive(Debug, Clone, Copy)]
struct LabReceiveFaultProfile {
    latency: LabLatencyConfig,
    loss_permille: u16,
}

#[derive(Clone)]
pub(crate) struct LabFaultController {
    profile: Arc<Mutex<LabReceiveFaultProfile>>,
}

impl LabFaultController {
    #[must_use]
    pub(crate) fn new(config: LabLatencyConfig, loss_permille: u16) -> Self {
        Self {
            profile: Arc::new(Mutex::new(LabReceiveFaultProfile {
                latency: config,
                loss_permille,
            })),
        }
    }

    pub(crate) fn update(&self, fixed_latency_ms: u64, jitter_ms: u64, loss_permille: u16) {
        let mut profile = self
            .profile
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        profile.latency.fixed_latency_ms = fixed_latency_ms;
        profile.latency.jitter_ms = jitter_ms;
        profile.loss_permille = loss_permille;
    }

    fn snapshot(&self) -> LabReceiveFaultProfile {
        *self
            .profile
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Wraps a [`TransportFactory`] with one node's receive-fault controller.
/// Listener datagrams get latency/jitter/loss; host datagrams get loss only.
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
        let seed = self.controller.snapshot().latency.seed;
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
        // The caller-provided clock is the listener's own offset/drift view
        // and therefore belongs only at the wrapper's outward boundary.
        // Give the inner transport a base-domain clock so latency deadlines
        // are comparable to `self.clock.now()` for every Lab node.
        let inner_clock: Arc<dyn TransportClock> = Arc::new(LabBaseTransportClock {
            clock: Arc::clone(&self.clock),
        });
        let inner = self.inner.connect_listener(config, inner_clock)?;
        let seed = self.controller.snapshot().latency.seed;
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
            let profile = self.controller.snapshot();
            if !self.prngs.should_drop(&event, profile.loss_permille) {
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
            TransportChannel::Control | TransportChannel::Runtime => {
                return arrived_at_ms;
            }
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
            return Ok(self.stamp_delivery_time(released));
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
            let (channel, arrived_at_ms) = event_channel_and_time(&event);
            if !matches!(
                channel,
                Some(TransportChannel::Synchronization | TransportChannel::Audio)
            ) {
                return Ok(self.stamp_delivery_time(event));
            }

            let profile = self.controller.snapshot();
            let channel = channel.expect("datagram channel was matched above");
            if held.should_drop(channel, profile.loss_permille) {
                continue;
            }

            let deadline_ms = held.deadline(channel, profile.latency, arrived_at_ms);
            if deadline_ms <= self.clock.now().get() {
                return Ok(self.stamp_delivery_time(event));
            }
            held.queue.push(Reverse(HeldEvent { deadline_ms, event }));
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

fn event_channel_and_time(event: &TransportEvent) -> (Option<TransportChannel>, u64) {
    match event {
        TransportEvent::FrameReceived {
            channel,
            received_at,
            ..
        } => (Some(*channel), received_at.get()),
        TransportEvent::PeerAccepted { received_at, .. }
        | TransportEvent::PeerAuthorized { received_at, .. }
        | TransportEvent::PeerDisconnected { received_at, .. }
        | TransportEvent::Rejected { received_at, .. } => (None, received_at.get()),
    }
}

fn should_drop_event(
    event: &TransportEvent,
    loss_permille: u16,
    synchronization_prng: &mut DeterministicPrng,
    audio_prng: &mut DeterministicPrng,
) -> bool {
    let (channel, _) = event_channel_and_time(event);
    match channel {
        Some(channel) => {
            should_drop_channel(channel, loss_permille, synchronization_prng, audio_prng)
        }
        None => false,
    }
}

fn should_drop_channel(
    channel: TransportChannel,
    loss_permille: u16,
    synchronization_prng: &mut DeterministicPrng,
    audio_prng: &mut DeterministicPrng,
) -> bool {
    if loss_permille == 0 {
        return false;
    }
    let prng = match channel {
        TransportChannel::Synchronization => synchronization_prng,
        TransportChannel::Audio => audio_prng,
        TransportChannel::Control | TransportChannel::Runtime => return false,
    };
    prng.next_permille() < loss_permille
}

fn compute_deadline(
    config: LabLatencyConfig,
    prng: &mut DeterministicPrng,
    arrived_at_ms: u64,
) -> u64 {
    let jitter_offset = if config.jitter_ms == 0 {
        0
    } else {
        let span = config.jitter_ms.saturating_mul(2).saturating_add(1);
        let raw = prng.next_below(usize::try_from(span).unwrap_or(usize::MAX));
        i64::try_from(raw).unwrap_or(0) - i64::try_from(config.jitter_ms).unwrap_or(0)
    };
    let base = i64::try_from(arrived_at_ms).unwrap_or(i64::MAX)
        + i64::try_from(config.fixed_latency_ms).unwrap_or(i64::MAX)
        + jitter_offset;
    u64::try_from(base.max(0)).unwrap_or(u64::MAX)
}

fn take_due(queue: &mut BinaryHeap<Reverse<HeldEvent>>, now_ms: u64) -> Option<TransportEvent> {
    let is_due = matches!(queue.peek(), Some(Reverse(held)) if held.deadline_ms <= now_ms);
    if !is_due {
        return None;
    }
    queue.pop().map(|Reverse(held)| held.event)
}

#[cfg(test)]
mod tests;
