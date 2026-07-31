//! Typed bridge from the local desktop transport worker to the authoritative Rust actor.

use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    CoreActorHandle, CoreSnapshot, TransportEvent as CoreTransportEvent,
};

pub(super) trait DesktopHostTransportEventSink: Send + Sync + 'static {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError>;
    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError>;
}

impl DesktopHostTransportEventSink for CoreActorHandle {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        CoreActorHandle::current_snapshot(self)
    }

    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError> {
        CoreActorHandle::submit_transport_event(self, event)
    }
}

#[cfg(test)]
pub(super) struct TestTransportEventSink;

#[cfg(test)]
impl DesktopHostTransportEventSink for TestTransportEventSink {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(CoreSnapshot::default())
    }

    fn submit_transport_event(&self, _event: CoreTransportEvent) -> Result<(), CoreError> {
        Ok(())
    }
}
