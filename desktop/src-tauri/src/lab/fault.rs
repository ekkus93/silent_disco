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
//! Block 41 attaches an optional bounded trace recorder to that same
//! controller. Packet observations and pass/drop/hold/release decisions are
//! recorded from the real receive path; tracing never reimplements or predicts
//! a fault decision independently of the adapter that actually applies it.

pub(crate) mod trace;

use self::trace::{
    PacketTraceIdentity, RecordedFaultDecision, RecordedFaultProfile, TransportTraceError,
    TransportTraceRecorder,
};
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

include!("fault/controller.rs");
include!("fault/transports.rs");
include!("fault/helpers.rs");

#[cfg(test)]
mod tests;
