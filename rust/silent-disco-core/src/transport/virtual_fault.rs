use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest, decode_frame, encode_frame};
use crate::runtime::NetworkEndpoint;

use super::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,
    ListenerTransportNode, TransportChannel, TransportClock, TransportCounters, TransportDelivery,
    TransportError, TransportErrorKind, TransportEvent, TransportFactory, TransportPeer,
    VirtualTransportFactory,
};

/// Minimal deterministic pseudo-random generator (`SplitMix64`) for Block
/// 39.2's "use a seeded deterministic PRNG where randomness is required".
/// Reproducibility, not unpredictability, is the point -- this is never
/// used for anything security-sensitive (identity, invite codes, etc.
/// already use `getrandom`, a real CSPRNG, elsewhere in this codebase).
#[derive(Debug, Clone, Copy)]
pub struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next pseudo-random `u64`, advancing the generator.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns a value in `0..1000` -- compared against a per-mille
    /// probability threshold without floating point.
    pub fn next_permille(&mut self) -> u16 {
        u16::try_from(self.next_u64() % 1000).unwrap_or(999)
    }

    /// Returns a value in `0..bound`, or `0` when `bound` is `0`.
    pub fn next_below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            usize::try_from(self.next_u64() % u64::try_from(bound).unwrap_or(u64::MAX)).unwrap_or(0)
        }
    }
}

/// Deterministic receive-side UDP faults for explicit virtual-transport
/// tests (Block 39.2 fault model).
///
/// Each host or listener node receives an independent copy of this
/// policy, so the same `VirtualUdpFaultConfig` applied to both sides of
/// one connection produces two independently-seeded fault streams
/// (matching how `drop_next_*_events` already worked before this block).
/// Successful send accounting is intentionally unchanged for every
/// non-corruption fault -- a real UDP send can succeed even when the
/// network later drops, reorders, or duplicates a datagram.
///
/// `Control`/`Runtime` events are never faulted, matching the pre-Block-39
/// scope (loss/reorder/etc. only ever applied to the `Synchronization`/
/// `Audio` datagram channels).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VirtualUdpFaultConfig {
    pub drop_next_sync_events: u32,
    pub drop_next_audio_events: u32,
    pub reorder_next_sync_pair: bool,
    pub reorder_next_audio_pair: bool,
    /// Seeds this config's deterministic PRNG (Block 39.2 "use a seeded
    /// deterministic PRNG"). Two otherwise-identical configs with the
    /// same seed, driven through the same sequence of events, make the
    /// exact same probabilistic decisions every time (Block 39.3
    /// "identical seed produces identical trace"); changing only the
    /// seed changes them (Block 39.3 "different seed changes trace where
    /// expected").
    pub seed: u64,
    /// Probability (0-1000, i.e. per-mille) that an event is dropped,
    /// independent of `drop_next_*_events`'s exact-count mechanism.
    pub loss_permille: u16,
    /// Probability (0-1000) that an event is delivered a second time
    /// immediately after the first (Block 39.2 "duplication").
    pub duplicate_permille: u16,
    /// Bounded reorder buffer size in events; `0` disables it (Block
    /// 39.2 "reordering" beyond the exact-pair mechanism above). Once
    /// this many events are held, a new arrival evicts one -- chosen by
    /// the seeded PRNG, not FIFO -- so the release order is scrambled
    /// within a bounded window rather than swapping exactly one pair.
    pub reorder_window: u8,
    /// Corrupts (bit-flips) the next this-many outgoing **audio**
    /// datagrams before they are sent (Block 39.2 "corruption"). Scoped
    /// to audio only: it is the one frame kind whose wire format carries
    /// a payload checksum (`FLAG_PAYLOAD_INTEGRITY`), so a single
    /// corrupted byte is *guaranteed* to trip a real decode failure --
    /// control and sync frames carry no such checksum, so corrupting them
    /// the same way would not reliably fail to decode at all. Unlike
    /// every other fault here, this is send-side and produces a real
    /// decode failure at send time -- see the module doc comment for why
    /// the virtual transport cannot model a receive-side decode failure.
    pub corrupt_next_events: u32,
    /// After this many events have passed through this channel, every
    /// later one is replaced by a synthesized disconnect instead of
    /// being delivered (Block 39.2 "disconnect").
    pub disconnect_after_events: Option<u32>,
    /// Cumulative encoded-byte budget for this channel; once exceeded,
    /// further events are dropped (Block 39.2 "bandwidth limit"). A
    /// deliberate simplification of a true bytes-per-second limit -- see
    /// the module doc comment.
    pub bandwidth_limit_bytes: Option<u64>,
}

/// Explicit Lab Mode wrapper adding deterministic UDP loss, reordering,
/// duplication, corruption, bandwidth limiting, disconnects, and
/// connection refusal.
///
/// # Corruption's send-side semantics
///
/// Every other fault here acts on the *receive* side: it wraps
/// `recv_event` and decides what a real, already-decoded `TransportEvent`
/// becomes once it reaches the recipient. The underlying virtual transport
/// now carries encoded bytes and decodes them at receive time, but this
/// wrapper intentionally sits *outside* that wire behind the transport-node
/// trait and therefore only sees the decoded event returned by the inner
/// node. It cannot mutate the inner queue's private bytes in place. This
/// wrapper instead corrupts the *encoded*
/// bytes and attempts a real `decode_frame` on them at send time, before
/// the (now-guaranteed-to-fail) send would otherwise proceed; the caller
/// that attempted to send receives a genuine `ProtocolError`-derived
/// `TransportError`, exercising the exact same production decoder Block
/// 39.1 requires, on genuinely mutated bytes -- it does not model UDP's
/// fire-and-forget semantics where a sender would see apparent success.
#[derive(Clone)]
pub struct FaultInjectingVirtualTransportFactory {
    inner: VirtualTransportFactory,
    config: VirtualUdpFaultConfig,
    refuse_remaining: Arc<Mutex<u32>>,
}

impl FaultInjectingVirtualTransportFactory {
    #[must_use]
    pub fn new(inner: VirtualTransportFactory, config: VirtualUdpFaultConfig) -> Self {
        Self {
            inner,
            config,
            refuse_remaining: Arc::new(Mutex::new(0)),
        }
    }

    /// Refuses the next `count` `connect_listener` attempts on this
    /// factory outright, before they ever reach the underlying virtual
    /// network (Block 39.2 "connection refusal"). Shared across every
    /// listener that connects through this factory -- unlike the
    /// per-node `VirtualUdpFaultConfig` fields, a refusal is a property
    /// of the connection attempt itself, before any per-node fault state
    /// could exist.
    #[must_use]
    pub fn with_connection_refusals(self, count: u32) -> Self {
        *self
            .refuse_remaining
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = count;
        self
    }
}

impl VirtualTransportFactory {
    /// Wraps this isolated virtual network with deterministic UDP faults.
    #[must_use]
    pub fn with_udp_faults(
        self,
        config: VirtualUdpFaultConfig,
    ) -> FaultInjectingVirtualTransportFactory {
        FaultInjectingVirtualTransportFactory::new(self, config)
    }
}

impl TransportFactory for FaultInjectingVirtualTransportFactory {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        let inner = self.inner.bind_host(config, clock)?;
        Ok(Box::new(FaultInjectingHostTransport {
            inner,
            faults: VirtualUdpFaultState::new(self.config),
            send_faults: Mutex::new(SendFaultState::new(self.config)),
        }))
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        {
            let mut remaining = self
                .refuse_remaining
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *remaining > 0 {
                *remaining -= 1;
                return Err(TransportError::new(
                    TransportErrorKind::Connect,
                    TransportChannel::Runtime,
                    "virtual connection attempt was deterministically refused by Lab Mode fault injection",
                ));
            }
        }
        let inner = self.inner.connect_listener(config, clock)?;
        Ok(Box::new(FaultInjectingListenerTransport {
            inner,
            faults: Mutex::new(VirtualUdpFaultState::new(self.config)),
        }))
    }
}

struct FaultInjectingHostTransport {
    inner: Box<dyn HostTransportNode>,
    faults: VirtualUdpFaultState,
    /// Behind a mutex because `HostTransportNode::broadcast_audio` takes
    /// `&self`, matching the same interior-mutability need `faults` has
    /// on the listener side.
    send_faults: Mutex<SendFaultState>,
}

impl HostTransportNode for FaultInjectingHostTransport {
    fn endpoint(&self) -> NetworkEndpoint {
        self.inner.endpoint()
    }

    fn authorize_peer(
        &self,
        device_id: &DeviceId,
        routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        self.inner.authorize_peer(device_id, routes)
    }

    fn authorize_peer_ports(
        &self,
        device_id: &DeviceId,
        sync_port: u16,
        audio_port: u16,
    ) -> Result<(), TransportError> {
        self.inner
            .authorize_peer_ports(device_id, sync_port, audio_port)
    }

    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError> {
        self.inner.disconnect_peer(device_id)
    }

    fn send_pending_control(
        &self,
        device_id: &DeviceId,
        message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        self.inner.send_pending_control(device_id, message)
    }

    fn send_control(
        &self,
        device_id: &DeviceId,
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

    fn broadcast_sync(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        self.inner.broadcast_sync(frame)
    }

    fn broadcast_audio(&self, frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        {
            let mut send_faults = self
                .send_faults
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            maybe_corrupt_audio(&mut send_faults, frame)?;
        }
        self.inner.broadcast_audio(frame)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let Self { inner, faults, .. } = self;
        recv_faulted_event(timeout, faults, |remaining| inner.recv_event(remaining))
    }

    fn counters(&self) -> TransportCounters {
        self.inner.counters()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.inner.shutdown()
    }
}

struct FaultInjectingListenerTransport {
    inner: Box<dyn ListenerTransportNode>,
    /// Behind a mutex because `ListenerTransportNode::recv_event` takes
    /// `&self` -- deliberately, so a receive cannot serialise against the
    /// send methods. Fault injection is the one listener implementation that
    /// genuinely mutates while receiving, and this is a test-only helper, so
    /// the lock costs nothing that matters.
    faults: Mutex<VirtualUdpFaultState>,
}

impl ListenerTransportNode for FaultInjectingListenerTransport {
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
        let mut faults = self
            .faults
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        recv_faulted_event(timeout, &mut faults, |remaining| {
            self.inner.recv_event(remaining)
        })
    }

    fn counters(&self) -> TransportCounters {
        self.inner.counters()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.inner.shutdown()
    }
}

/// Send-side fault state (corruption only -- see the module doc comment
/// for why corruption cannot be modeled receive-side, and
/// [`VirtualUdpFaultConfig::corrupt_next_events`] for why this is
/// audio-only). Kept separate from [`VirtualUdpFaultState`] (receive-side)
/// since the two run on completely different call paths and would
/// otherwise need to share a lock for no reason.
struct SendFaultState {
    audio_corrupt_remaining: u32,
}

impl SendFaultState {
    fn new(config: VirtualUdpFaultConfig) -> Self {
        Self {
            audio_corrupt_remaining: config.corrupt_next_events,
        }
    }

    fn take_corruption(&mut self) -> bool {
        if self.audio_corrupt_remaining > 0 {
            self.audio_corrupt_remaining -= 1;
            true
        } else {
            false
        }
    }
}

/// Encodes `frame`, corrupts one byte if a corruption fault is due, and
/// attempts to decode the (now-corrupted) bytes. A corrupted frame's
/// decode failure is returned as a real `TransportError` -- see the
/// module doc comment for the send-side semantics this implies, and
/// [`VirtualUdpFaultConfig::corrupt_next_events`] for why this is only
/// ever called for the audio channel.
fn maybe_corrupt_audio(
    send_faults: &mut SendFaultState,
    frame: &ProtocolFrame,
) -> Result<(), TransportError> {
    if !send_faults.take_corruption() {
        return Ok(());
    }
    let channel = TransportChannel::Audio;
    let mut bytes =
        encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
    let Some(last) = bytes.last_mut() else {
        return Ok(());
    };
    *last ^= 0xFF;
    match decode_frame(&bytes) {
        Ok(_) => Ok(()),
        Err(error) => Err(TransportError::protocol(channel, &error)),
    }
}

struct VirtualUdpFaultState {
    synchronization: ChannelFaultState,
    audio: ChannelFaultState,
    deferred: VecDeque<TransportEvent>,
}

impl VirtualUdpFaultState {
    fn new(config: VirtualUdpFaultConfig) -> Self {
        Self {
            synchronization: ChannelFaultState::new(
                config,
                config.seed,
                config.drop_next_sync_events,
                config.reorder_next_sync_pair,
            ),
            // A distinct PRNG stream per channel (seed offset by a fixed
            // constant) -- otherwise interleaved sync/audio events would
            // silently share one draw sequence and moving traffic between
            // channels would change the trace, which is not what "seeded
            // deterministic" should mean.
            audio: ChannelFaultState::new(
                config,
                config.seed ^ 0xA5A5_A5A5_A5A5_A5A5,
                config.drop_next_audio_events,
                config.reorder_next_audio_pair,
            ),
            deferred: VecDeque::new(),
        }
    }

    fn process(&mut self, event: TransportEvent) -> Option<TransportEvent> {
        let channel = match &event {
            TransportEvent::FrameReceived { channel, .. } => Some(*channel),
            _ => None,
        };
        let action = match channel {
            Some(TransportChannel::Synchronization) => self.synchronization.apply(event),
            Some(TransportChannel::Audio) => self.audio.apply(event),
            Some(TransportChannel::Control | TransportChannel::Runtime) | None => {
                FaultAction::Deliver(event)
            }
        };
        match action {
            FaultAction::Drop => None,
            FaultAction::Deliver(event) => Some(event),
            FaultAction::DeliverThenDefer { event, deferred } => {
                self.deferred.push_back(deferred);
                Some(event)
            }
        }
    }
}

struct ChannelFaultState {
    config: VirtualUdpFaultConfig,
    drops_remaining: u32,
    reorder_next_pair: bool,
    held: Option<TransportEvent>,
    prng: DeterministicPrng,
    events_processed: u32,
    bytes_delivered: u64,
    reorder_buffer: VecDeque<TransportEvent>,
}

impl ChannelFaultState {
    fn new(config: VirtualUdpFaultConfig, seed: u64, drop_count: u32, reorder_pair: bool) -> Self {
        Self {
            config,
            drops_remaining: drop_count,
            reorder_next_pair: reorder_pair,
            held: None,
            prng: DeterministicPrng::new(seed),
            events_processed: 0,
            bytes_delivered: 0,
            reorder_buffer: VecDeque::new(),
        }
    }

    fn apply(&mut self, event: TransportEvent) -> FaultAction {
        if let Some(after) = self.config.disconnect_after_events
            && self.events_processed >= after
        {
            self.events_processed = self.events_processed.saturating_add(1);
            return FaultAction::Deliver(synthesize_disconnect(&event));
        }
        self.events_processed = self.events_processed.saturating_add(1);

        if self.drops_remaining > 0 {
            self.drops_remaining -= 1;
            return FaultAction::Drop;
        }

        if let Some(limit) = self.config.bandwidth_limit_bytes {
            let size = estimated_event_bytes(&event);
            if self.bytes_delivered.saturating_add(size) > limit {
                return FaultAction::Drop;
            }
            self.bytes_delivered = self.bytes_delivered.saturating_add(size);
        }

        if self.prng.next_permille() < self.config.loss_permille {
            return FaultAction::Drop;
        }

        if self.reorder_next_pair {
            if let Some(deferred) = self.held.take() {
                self.reorder_next_pair = false;
                return FaultAction::DeliverThenDefer { event, deferred };
            }
            self.held = Some(event);
            return FaultAction::Drop;
        }

        if self.config.reorder_window > 0 {
            self.reorder_buffer.push_back(event);
            if self.reorder_buffer.len() >= usize::from(self.config.reorder_window) {
                let index = self.prng.next_below(self.reorder_buffer.len());
                let Some(released) = self.reorder_buffer.remove(index) else {
                    return FaultAction::Drop;
                };
                return self.maybe_duplicate(released);
            }
            return FaultAction::Drop;
        }

        self.maybe_duplicate(event)
    }

    fn maybe_duplicate(&mut self, event: TransportEvent) -> FaultAction {
        if self.prng.next_permille() < self.config.duplicate_permille {
            FaultAction::DeliverThenDefer {
                event: event.clone(),
                deferred: event,
            }
        } else {
            FaultAction::Deliver(event)
        }
    }
}

enum FaultAction {
    Drop,
    Deliver(TransportEvent),
    DeliverThenDefer {
        event: TransportEvent,
        deferred: TransportEvent,
    },
}

fn synthesize_disconnect(event: &TransportEvent) -> TransportEvent {
    let (peer, received_at) = match event {
        TransportEvent::FrameReceived {
            peer, received_at, ..
        }
        | TransportEvent::PeerAccepted { peer, received_at }
        | TransportEvent::PeerAuthorized {
            peer, received_at, ..
        }
        | TransportEvent::PeerDisconnected {
            peer, received_at, ..
        } => (peer.clone(), *received_at),
        TransportEvent::Rejected {
            source,
            received_at,
            ..
        } => (
            TransportPeer {
                device_id: None,
                control_address: *source,
            },
            *received_at,
        ),
    };
    TransportEvent::PeerDisconnected {
        peer,
        error: None,
        received_at,
    }
}

fn estimated_event_bytes(event: &TransportEvent) -> u64 {
    let TransportEvent::FrameReceived { frame, .. } = event else {
        return 0;
    };
    encode_frame(frame).map_or(0, |bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
}

fn recv_faulted_event(
    timeout: Duration,
    faults: &mut VirtualUdpFaultState,
    mut recv: impl FnMut(Duration) -> Result<TransportEvent, TransportError>,
) -> Result<TransportEvent, TransportError> {
    if let Some(event) = faults.deferred.pop_front() {
        return Ok(event);
    }
    let started_at = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            return Err(TransportError::new(
                TransportErrorKind::Timeout,
                TransportChannel::Runtime,
                "virtual fault-injection event receive timed out",
            ));
        }
        let event = recv(remaining)?;
        if let Some(event) = faults.process(event) {
            return Ok(event);
        }
    }
}
