//! Production-shaped live virtual-transport adapter for deterministic Lab scenarios.

mod observer;
mod support;
mod sync;

pub(super) use observer::LiveScenarioObserver;

use self::support::{
    ReceiveFaultProfile, build_fault_controllers, build_receive_profiles, core_error,
    failed_delivery_report, live_error, transport_error,
};
use self::sync::LiveSyncState;
use super::{NodeId, Scenario, ScenarioAction, scenario_node_parts};
use crate::dto::DesktopErrorDto;
use crate::lab::fault::{LabFaultController, LabLatencyTransportFactory};
use crate::lab::{LabClock, LabNodeId, LabRuntime};
use crate::platform::host_transport_events::HostTransportEventProcessor;
use silent_disco_core::domain::{DeviceId, OperationId};
use silent_disco_core::error::{CoreError, CoreErrorCode, ErrorSeverity};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, Disconnect, JoinApproval, JoinRejection, JoinRequest,
    ProtocolFrame,
};
use silent_disco_core::runtime::{
    AudioEvent, CoreActorHandle, CoreNotification, PlatformEffect, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, SessionAdvertisement, TransportEffect,
    TransportEffectRequest, TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    TransportChannel, TransportClock, TransportErrorKind, TransportEvent as RuntimeTransportEvent,
    TransportFactory, VirtualTransportFactory, VirtualTransportNetwork,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

const MAX_PUMP_ITERATIONS: usize = 512;
const NONBLOCKING_RECV_BUDGET: Duration = Duration::from_millis(1);

struct ActorEndpoint {
    handle: CoreActorHandle,
    device_id: DeviceId,
    clock: Arc<crate::lab::clock::LabNodeClock>,
    effects: Receiver<CoreNotification>,
    pending_invite_codes: VecDeque<Option<String>>,
}

struct LiveHost {
    transport: Box<dyn HostTransportNode>,
    advertisement: SessionAdvertisement,
    processor: HostTransportEventProcessor,
}

struct LiveListener {
    transport: Box<dyn ListenerTransportNode>,
    sync: LiveSyncState,
}

pub(super) struct LiveTransportDriver {
    network: VirtualTransportNetwork,
    shared_clock: Arc<LabClock>,
    links: Vec<super::ScenarioLink>,
    profiles: HashMap<NodeId, ReceiveFaultProfile>,
    fault_controllers: HashMap<NodeId, LabFaultController>,
    actors: HashMap<NodeId, ActorEndpoint>,
    hosts: HashMap<NodeId, LiveHost>,
    listeners: HashMap<NodeId, LiveListener>,
}

include!("live_transport/driver_start.rs");
include!("live_transport/driver_effects.rs");
