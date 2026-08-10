use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest};
use crate::runtime::NetworkEndpoint;

use super::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,
    ListenerTransportNode, TransportChannel, TransportClock, TransportCounters, TransportDelivery,
    TransportError, TransportErrorKind, TransportEvent, TransportFactory, VirtualTransportFactory,
};

/// Deterministic receive-side UDP faults for explicit virtual-transport tests.
///
/// Each host or listener node receives an independent copy of this policy.
/// Successful send accounting is intentionally unchanged because a real UDP
/// send can succeed even when the network later drops or reorders a datagram.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VirtualUdpFaultConfig {
    pub drop_next_sync_events: u32,
    pub drop_next_audio_events: u32,
    pub reorder_next_sync_pair: bool,
    pub reorder_next_audio_pair: bool,
}

/// Explicit Lab Mode wrapper adding deterministic UDP loss and reordering.
#[derive(Clone)]
pub struct FaultInjectingVirtualTransportFactory {
    inner: VirtualTransportFactory,
    config: VirtualUdpFaultConfig,
}

impl FaultInjectingVirtualTransportFactory {
    #[must_use]
    pub const fn new(inner: VirtualTransportFactory, config: VirtualUdpFaultConfig) -> Self {
        Self { inner, config }
    }
}

impl VirtualTransportFactory {
    /// Wraps this isolated virtual network with deterministic UDP faults.
    #[must_use]
    pub const fn with_udp_faults(
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
        }))
    }

    fn connect_listener(
        &self,
        config: ListenerTransportConfig,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
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
        self.inner.broadcast_audio(frame)
    }

    fn recv_event(&mut self, timeout: Duration) -> Result<TransportEvent, TransportError> {
        let Self { inner, faults } = self;
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

struct VirtualUdpFaultState {
    synchronization: ChannelFaultState,
    audio: ChannelFaultState,
    deferred: VecDeque<TransportEvent>,
}

impl VirtualUdpFaultState {
    fn new(config: VirtualUdpFaultConfig) -> Self {
        Self {
            synchronization: ChannelFaultState {
                drops_remaining: config.drop_next_sync_events,
                reorder_next_pair: config.reorder_next_sync_pair,
                held: None,
            },
            audio: ChannelFaultState {
                drops_remaining: config.drop_next_audio_events,
                reorder_next_pair: config.reorder_next_audio_pair,
                held: None,
            },
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
    drops_remaining: u32,
    reorder_next_pair: bool,
    held: Option<TransportEvent>,
}

impl ChannelFaultState {
    fn apply(&mut self, event: TransportEvent) -> FaultAction {
        if self.drops_remaining > 0 {
            self.drops_remaining -= 1;
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
        FaultAction::Deliver(event)
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
