//! Lab-clock-aware latency/jitter fault injection (Block 39.2 "latency",
//! "jitter"), layered on top of the shared core's virtual transport and
//! fault model (`silent_disco_core::transport`).
//!
//! This lives in the desktop Lab module, not the shared core, because it
//! needs Block 38's [`crate::lab::clock::LabClock`] -- something the core
//! has no concept of. Every other fault type (loss, duplication, reorder,
//! corruption, bandwidth limit, disconnect, connection refusal) is
//! synchronous and receive-side-only, so it fits the shared core's own
//! `VirtualUdpFaultConfig`/`FaultInjectingVirtualTransportFactory`
//! (`rust/silent-disco-core/src/transport/virtual_fault.rs`) without
//! needing virtual time at all. Latency and jitter are different: they
//! must *hold* an event until a computed deadline, and "deadline" is
//! meaningless without a clock to measure it against.
//!
//! Deliberately scoped to the **listener** receive path only (a host
//! broadcasts once; each connecting listener that goes through this
//! wrapper experiences delayed, optionally jittered receipt). This
//! matches the far more common real-world asymmetry this project cares
//! about -- a listener's own Wi-Fi path being slow or jittery -- and keeps
//! this wrapper's surface to one trait instead of two. `bind_host` is not
//! wrapped at all; a Lab scenario that also wants host-side latency can
//! layer this around a *different* factory used only for `connect_listener`.
//!
//! No wall-clock sleep anywhere: a delayed event's release is decided
//! purely by comparing [`LabClock::now`] to a precomputed deadline. The
//! only real waiting here is for the underlying (already real, channel-
//! based) transport to produce a *new* event -- exactly like the shared
//! core's own `recv_faulted_event`. A scenario makes a held event
//! releasable by calling [`crate::lab::LabRuntime::advance`], not by
//! waiting.

use super::clock::LabClock;
use silent_disco_core::protocol::{ControlMessage, SyncRequest};
use silent_disco_core::transport::{
    DeterministicPrng, HostTransportConfig, HostTransportNode, ListenerDatagramRoutes,
    ListenerTransportConfig, ListenerTransportNode, TransportChannel, TransportClock,
    TransportCounters, TransportDelivery, TransportError, TransportEvent, TransportFactory,
};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// Deterministic latency/jitter configuration (Block 39.2). Applies only
/// to the datagram channels (`Synchronization`/`Audio`), matching the
/// shared core's own `VirtualUdpFaultConfig` scope; control-channel
/// events are always released immediately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LabLatencyConfig {
    /// Every held event's deadline is at least this far past the moment
    /// it actually arrived (Block 39.2 "latency").
    pub(crate) fixed_latency_ms: u64,
    /// A seeded-random offset in `[-jitter_ms, +jitter_ms]` added on top
    /// of `fixed_latency_ms` (Block 39.2 "jitter"). `0` disables jitter
    /// entirely -- latency becomes exact.
    pub(crate) jitter_ms: u64,
    /// Seeds this configuration's deterministic PRNG (Block 39.2 "use a
    /// seeded deterministic PRNG"); irrelevant when `jitter_ms == 0`.
    pub(crate) seed: u64,
}

/// Wraps a [`TransportFactory`] so every listener connected through it
/// experiences delayed (and optionally jittered) datagram receipt.
/// `bind_host` is untouched -- see the module doc comment.
#[derive(Clone)]
pub(crate) struct LabLatencyTransportFactory<F> {
    inner: F,
    clock: Arc<LabClock>,
    config: LabLatencyConfig,
}

impl<F> LabLatencyTransportFactory<F> {
    #[must_use]
    pub(crate) fn new(inner: F, clock: Arc<LabClock>, config: LabLatencyConfig) -> Self {
        Self {
            inner,
            clock,
            config,
        }
    }
}

impl<F: TransportFactory> TransportFactory for LabLatencyTransportFactory<F> {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        self.inner.bind_host(config, clock)
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        let delivery_clock = Arc::clone(&clock);
        let inner = self.inner.connect_listener(config, clock)?;
        Ok(Box::new(LabLatencyListenerTransport {
            inner,
            clock: Arc::clone(&self.clock),
            delivery_clock,
            config: self.config,
            held: Mutex::new(HeldEvents {
                prng: DeterministicPrng::new(self.config.seed),
                queue: BinaryHeap::new(),
            }),
        }))
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
    prng: DeterministicPrng,
    queue: BinaryHeap<Reverse<HeldEvent>>,
}

struct LabLatencyListenerTransport {
    inner: Box<dyn ListenerTransportNode>,
    clock: Arc<LabClock>,
    delivery_clock: Arc<dyn TransportClock>,
    config: LabLatencyConfig,
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
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Anything already due is released before touching the
        // underlying transport at all -- this is the only path a
        // scenario's `LabRuntime::advance()` followed by a fresh
        // `recv_event` call takes. A delayed datagram's receive timestamp
        // must reflect this virtual delivery instant, not the underlying
        // pre-fault event timestamp, or synchronization would silently
        // under-report injected network latency.
        if let Some(released) = take_due(&mut held.queue, self.clock.now().get()) {
            return Ok(self.stamp_delivery_time(released));
        }

        let event = self.inner.recv_event(timeout)?;
        // The deadline is anchored to the underlying transport's event
        // timestamp rather than when this call happened to poll for it.
        // Anchoring to poll time would make the hold duration depend on
        // how promptly a scenario calls `recv_event`, which is exactly
        // the kind of accidental nondeterminism Block 38's virtual clock
        // exists to avoid. Once the faulted event is actually delivered,
        // `received_at` is re-stamped from the listener's TransportClock.
        let (channel, arrived_at_ms) = match &event {
            TransportEvent::FrameReceived {
                channel,
                received_at,
                ..
            } => (Some(*channel), received_at.get()),
            _ => (None, self.clock.now().get()),
        };
        if !matches!(
            channel,
            Some(TransportChannel::Synchronization | TransportChannel::Audio)
        ) {
            // Control/Runtime events, and every non-`FrameReceived`
            // variant, are never delayed.
            return Ok(event);
        }

        let deadline_ms = self.compute_deadline(&mut held.prng, arrived_at_ms);
        if deadline_ms <= self.clock.now().get() {
            return Ok(self.stamp_delivery_time(event));
        }
        held.queue.push(Reverse(HeldEvent { deadline_ms, event }));
        Err(TransportError::timeout(
            TransportChannel::Runtime,
            "Lab Mode latency fault is holding the newest event for a later virtual deadline",
        ))
    }

    fn counters(&self) -> TransportCounters {
        self.inner.counters()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.inner.shutdown()
    }
}

impl LabLatencyListenerTransport {
    fn compute_deadline(&self, prng: &mut DeterministicPrng, arrived_at_ms: u64) -> u64 {
        let jitter_offset = if self.config.jitter_ms == 0 {
            0
        } else {
            let span = self.config.jitter_ms.saturating_mul(2).saturating_add(1);
            let raw = prng.next_below(usize::try_from(span).unwrap_or(usize::MAX));
            i64::try_from(raw).unwrap_or(0) - i64::try_from(self.config.jitter_ms).unwrap_or(0)
        };
        let base = i64::try_from(arrived_at_ms).unwrap_or(i64::MAX)
            + i64::try_from(self.config.fixed_latency_ms).unwrap_or(i64::MAX)
            + jitter_offset;
        u64::try_from(base.max(0)).unwrap_or(u64::MAX)
    }

    fn stamp_delivery_time(&self, mut event: TransportEvent) -> TransportEvent {
        if let TransportEvent::FrameReceived { received_at, .. } = &mut event {
            *received_at = self.delivery_clock.now();
        }
        event
    }
}

/// Pops and returns the earliest held event once its deadline is at or
/// before `now_ms`; leaves the queue untouched otherwise.
fn take_due(queue: &mut BinaryHeap<Reverse<HeldEvent>>, now_ms: u64) -> Option<TransportEvent> {
    let is_due = matches!(queue.peek(), Some(Reverse(held)) if held.deadline_ms <= now_ms);
    if !is_due {
        return None;
    }
    queue.pop().map(|Reverse(held)| held.event)
}

#[cfg(test)]
mod tests;
