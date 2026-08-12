use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::DeviceId;
use crate::protocol::{ControlMessage, SyncRequest};
use crate::runtime::NetworkEndpoint;

use super::{
    FaultInjectingVirtualTransportFactory, HostTransportConfig, HostTransportNode,
    ListenerDatagramRoutes, ListenerTransportConfig, ListenerTransportNode, TransportChannel,
    TransportClock, TransportCounters, TransportDelivery, TransportError, TransportErrorKind,
    TransportEvent, TransportFactory, VirtualTransportFactory,
};

type ReconnectKey = (NetworkEndpoint, DeviceId);

/// Deterministic reconnect backoff around a virtual transport factory.
///
/// The wrapper never sleeps. Once a listener observes a disconnect (or
/// explicitly shuts down), the same endpoint/device identity cannot reconnect
/// until the *new connection attempt's injected* [`TransportClock`] reaches
/// the recorded deadline. A [`super::ManualTransportClock`] therefore makes
/// reconnect timing fully deterministic and instant to advance in tests/Lab.
#[derive(Clone)]
pub struct ReconnectDelayingTransportFactory<F> {
    inner: F,
    delay_ms: u64,
    deadlines: Arc<Mutex<HashMap<ReconnectKey, u64>>>,
}

impl<F> ReconnectDelayingTransportFactory<F> {
    #[must_use]
    pub fn new(inner: F, delay_ms: u64) -> Self {
        Self {
            inner,
            delay_ms,
            deadlines: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl VirtualTransportFactory {
    /// Adds deterministic virtual-clock reconnect backoff to this isolated
    /// virtual network.
    #[must_use]
    pub fn with_reconnect_delay(self, delay_ms: u64) -> ReconnectDelayingTransportFactory<Self> {
        ReconnectDelayingTransportFactory::new(self, delay_ms)
    }
}

impl FaultInjectingVirtualTransportFactory {
    /// Adds deterministic virtual-clock reconnect backoff after the existing
    /// loss/reorder/etc. fault wrapper.
    #[must_use]
    pub fn with_reconnect_delay(self, delay_ms: u64) -> ReconnectDelayingTransportFactory<Self> {
        ReconnectDelayingTransportFactory::new(self, delay_ms)
    }
}

impl<F: TransportFactory> TransportFactory for ReconnectDelayingTransportFactory<F> {
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
        let key = (config.endpoint, config.device_id.clone());
        let now_ms = clock.now().get();
        {
            let mut deadlines = self
                .deadlines
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(deadline_ms) = deadlines.get(&key).copied() {
                if now_ms < deadline_ms {
                    return Err(TransportError::new(
                        TransportErrorKind::Connect,
                        TransportChannel::Runtime,
                        format!(
                            "virtual reconnect is delayed until {deadline_ms}ms (current virtual time {now_ms}ms)"
                        ),
                    ));
                }
                deadlines.remove(&key);
            }
        }

        let inner = self.inner.connect_listener(config, Arc::clone(&clock))?;
        Ok(Box::new(ReconnectDelayingListenerTransport {
            inner,
            reconnect: ReconnectDelayState {
                key,
                delay_ms: self.delay_ms,
                clock,
                deadlines: Arc::clone(&self.deadlines),
                armed: AtomicBool::new(false),
            },
            shutdown_complete: false,
        }))
    }
}

struct ReconnectDelayState {
    key: ReconnectKey,
    delay_ms: u64,
    clock: Arc<dyn TransportClock>,
    deadlines: Arc<Mutex<HashMap<ReconnectKey, u64>>>,
    armed: AtomicBool,
}

impl ReconnectDelayState {
    fn arm(&self) {
        if self.delay_ms == 0 || self.armed.swap(true, Ordering::AcqRel) {
            return;
        }
        let now_ms = self.clock.now().get();
        let deadline_ms = now_ms.saturating_add(self.delay_ms);
        let mut deadlines = self
            .deadlines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match deadlines.get_mut(&self.key) {
            // A disconnect event followed by explicit shutdown must not
            // silently extend the original backoff window.
            Some(existing) if *existing > now_ms => {}
            Some(existing) => *existing = deadline_ms,
            None => {
                deadlines.insert(self.key.clone(), deadline_ms);
            }
        }
    }
}

struct ReconnectDelayingListenerTransport {
    inner: Box<dyn ListenerTransportNode>,
    reconnect: ReconnectDelayState,
    shutdown_complete: bool,
}

impl ListenerTransportNode for ReconnectDelayingListenerTransport {
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
        let event = self.inner.recv_event(timeout)?;
        if matches!(event, TransportEvent::PeerDisconnected { .. }) {
            self.reconnect.arm();
        }
        Ok(event)
    }

    fn counters(&self) -> TransportCounters {
        self.inner.counters()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown_complete {
            return Ok(());
        }
        self.inner.shutdown()?;
        self.reconnect.arm();
        self.shutdown_complete = true;
        Ok(())
    }
}

impl Drop for ReconnectDelayingListenerTransport {
    fn drop(&mut self) {
        if self.shutdown_complete {
            return;
        }
        match self.inner.shutdown() {
            Ok(()) => {
                self.reconnect.arm();
                self.shutdown_complete = true;
            }
            Err(error) => {
                // This wrapper is Lab/test-only, but its cleanup must still
                // never convert a real inner shutdown failure into success.
                assert!(
                    std::thread::panicking(),
                    "reconnect-delay listener dropped after shutdown failure: {error}"
                );
            }
        }
    }
}
