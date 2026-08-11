from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(
            f"{path}: expected exactly one match, found {count}: {old[:120]!r}"
        )
    target.write_text(text.replace(old, new))


def write_new(path: str, content: str) -> None:
    target = Path(path)
    if target.exists():
        raise SystemExit(f"{path}: refusing to overwrite an existing source file")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content)


write_new(
    "rust/silent-disco-core/src/transport/virtual_fault_control.rs",
    '''use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use super::{TransportChannel, TransportError, TransportErrorKind};

const MAX_LOSS_PERMILLE: u16 = 1_000;

/// Live, deterministic loss control for an already-created virtual fault transport.
///
/// Changing this value does not reconnect a node or reset its seeded PRNG stream;
/// subsequent receive decisions simply use the new threshold. That makes a
/// timestamped Lab mutation deterministic without rewriting events already processed.
#[derive(Clone)]
pub struct VirtualUdpLossController {
    loss_permille: Arc<AtomicU16>,
}

impl VirtualUdpLossController {
    pub(super) fn new(loss_permille: u16) -> Self {
        Self {
            loss_permille: Arc::new(AtomicU16::new(loss_permille)),
        }
    }

    #[must_use]
    pub fn loss_permille(&self) -> u16 {
        self.loss_permille.load(Ordering::Relaxed)
    }

    /// Changes the loss probability used by transports already sharing this controller.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfiguration` when `loss_permille` exceeds 1000.
    pub fn set_loss_permille(&self, loss_permille: u16) -> Result<(), TransportError> {
        if loss_permille > MAX_LOSS_PERMILLE {
            return Err(TransportError::new(
                TransportErrorKind::InvalidConfiguration,
                TransportChannel::Runtime,
                "virtual UDP loss must be between 0 and 1000 permille",
            ));
        }
        self.loss_permille.store(loss_permille, Ordering::Relaxed);
        Ok(())
    }
}
''',
)

replace_once(
    "rust/silent-disco-core/src/transport/mod.rs",
    "mod virtual_fault;\nmod virtual_transport;\n",
    "mod virtual_fault;\nmod virtual_fault_control;\nmod virtual_transport;\n",
)
replace_once(
    "rust/silent-disco-core/src/transport/mod.rs",
    "pub use virtual_fault::{\n    DeterministicPrng, FaultInjectingVirtualTransportFactory, VirtualUdpFaultConfig,\n};\n",
    "pub use virtual_fault::{\n    DeterministicPrng, FaultInjectingVirtualTransportFactory, VirtualUdpFaultConfig,\n};\npub use virtual_fault_control::VirtualUdpLossController;\n",
)

vf = "rust/silent-disco-core/src/transport/virtual_fault.rs"
replace_once(
    vf,
    "use super::{\n    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,\n",
    "use super::virtual_fault_control::VirtualUdpLossController;\nuse super::{\n    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,\n",
)
replace_once(
    vf,
    "pub struct FaultInjectingVirtualTransportFactory {\n    inner: VirtualTransportFactory,\n    config: VirtualUdpFaultConfig,\n    refuse_remaining: Arc<Mutex<u32>>,\n}\n",
    "pub struct FaultInjectingVirtualTransportFactory {\n    inner: VirtualTransportFactory,\n    config: VirtualUdpFaultConfig,\n    loss_controller: VirtualUdpLossController,\n    refuse_remaining: Arc<Mutex<u32>>,\n}\n",
)
replace_once(
    vf,
    '''    pub fn new(inner: VirtualTransportFactory, config: VirtualUdpFaultConfig) -> Self {
        Self {
            inner,
            config,
            refuse_remaining: Arc::new(Mutex::new(0)),
        }
    }

    /// Refuses the next `count`''',
    '''    pub fn new(inner: VirtualTransportFactory, config: VirtualUdpFaultConfig) -> Self {
        Self {
            inner,
            config,
            loss_controller: VirtualUdpLossController::new(config.loss_permille),
            refuse_remaining: Arc::new(Mutex::new(0)),
        }
    }

    /// Returns the live loss control shared by transports created from this factory.
    #[must_use]
    pub fn loss_controller(&self) -> VirtualUdpLossController {
        self.loss_controller.clone()
    }

    /// Refuses the next `count`''',
)
replace_once(
    vf,
    "            faults: VirtualUdpFaultState::new(self.config),\n            send_faults: Mutex::new(SendFaultState::new(self.config)),\n",
    "            faults: VirtualUdpFaultState::new(self.config, self.loss_controller.clone()),\n            send_faults: Mutex::new(SendFaultState::new(self.config)),\n",
)
replace_once(
    vf,
    "            faults: Mutex::new(VirtualUdpFaultState::new(self.config)),\n",
    '''            faults: Mutex::new(VirtualUdpFaultState::new(
                self.config,
                self.loss_controller.clone(),
            )),
''',
)
replace_once(
    vf,
    '''impl VirtualUdpFaultState {
    fn new(config: VirtualUdpFaultConfig) -> Self {
        Self {
            synchronization: ChannelFaultState::new(
                config,
                config.seed,
''',
    '''impl VirtualUdpFaultState {
    fn new(config: VirtualUdpFaultConfig, loss_controller: VirtualUdpLossController) -> Self {
        Self {
            synchronization: ChannelFaultState::new(
                config,
                loss_controller.clone(),
                config.seed,
''',
)
replace_once(
    vf,
    '''            audio: ChannelFaultState::new(
                config,
                config.seed ^ 0xA5A5_A5A5_A5A5_A5A5,
''',
    '''            audio: ChannelFaultState::new(
                config,
                loss_controller,
                config.seed ^ 0xA5A5_A5A5_A5A5_A5A5,
''',
)
replace_once(
    vf,
    "struct ChannelFaultState {\n    config: VirtualUdpFaultConfig,\n    drops_remaining: u32,\n",
    "struct ChannelFaultState {\n    config: VirtualUdpFaultConfig,\n    loss_controller: VirtualUdpLossController,\n    drops_remaining: u32,\n",
)
replace_once(
    vf,
    '''    fn new(config: VirtualUdpFaultConfig, seed: u64, drop_count: u32, reorder_pair: bool) -> Self {
        Self {
            config,
            drops_remaining: drop_count,
''',
    '''    fn new(
        config: VirtualUdpFaultConfig,
        loss_controller: VirtualUdpLossController,
        seed: u64,
        drop_count: u32,
        reorder_pair: bool,
    ) -> Self {
        Self {
            config,
            loss_controller,
            drops_remaining: drop_count,
''',
)
replace_once(
    vf,
    "        if self.prng.next_permille() < self.config.loss_permille {\n",
    "        if self.prng.next_permille() < self.loss_controller.loss_permille() {\n",
)

vft = "rust/silent-disco-core/src/transport/virtual_fault_tests.rs"
core_test_marker = '''/// Block 39.3 "deterministic loss sequence" / "identical seed produces
'''
core_test = '''/// A loss update must affect a listener that is already connected; rebuilding
/// or reconnecting the virtual transport would make a timestamped mutation fake.
#[test]
fn loss_controller_updates_an_existing_listener_without_reconnect() {
    let session_id = SessionId::new("fault-live-loss").expect("test session ID is valid");
    let device_id = DeviceId::new("fault-live-loss-listener").expect("test device ID is valid");
    let factory = VirtualTransportFactory::new(VirtualTransportNetwork::default())
        .with_udp_faults(VirtualUdpFaultConfig::default());
    let loss = factory.loss_controller();
    let clock = Arc::new(ManualTransportClock::new(1_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("virtual host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("virtual listener should connect");
    authorize_listener(&mut *host, &mut *listener, &session_id, &device_id);

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("zero-loss send should succeed");
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(1));

    loss.set_loss_permille(1_000)
        .expect("1000 permille is a valid loss profile");
    host.broadcast_audio(&audio_frame(&session_id, 2))
        .expect("UDP send still succeeds before receive-side loss");
    let Err(dropped) = listener.recv_event(LOSS_TIMEOUT) else {
        panic!("the updated 100% loss profile must drop the next receive");
    };
    assert_eq!(dropped.kind, TransportErrorKind::Timeout);

    loss.set_loss_permille(0)
        .expect("zero permille is a valid loss profile");
    host.broadcast_audio(&audio_frame(&session_id, 3))
        .expect("restored zero-loss send should succeed");
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(3));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

'''
replace_once(vft, core_test_marker, core_test + core_test_marker)

write_new(
    "desktop/src-tauri/src/lab/fault/control.rs",
    '''use super::LabLatencyConfig;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Live latency/jitter control shared with listeners that already exist.
///
/// Already-held events keep the deadline computed when they arrived. New
/// datagrams use the latest values, so a mutation never rewrites prior
/// virtual-time decisions.
#[derive(Clone)]
pub(crate) struct LabLatencyController {
    fixed_latency_ms: Arc<AtomicU64>,
    jitter_ms: Arc<AtomicU64>,
}

impl LabLatencyController {
    pub(super) fn new(config: LabLatencyConfig) -> Self {
        Self {
            fixed_latency_ms: Arc::new(AtomicU64::new(config.fixed_latency_ms)),
            jitter_ms: Arc::new(AtomicU64::new(config.jitter_ms)),
        }
    }

    pub(crate) fn set(&self, fixed_latency_ms: u64, jitter_ms: u64) {
        self.fixed_latency_ms
            .store(fixed_latency_ms, Ordering::Relaxed);
        self.jitter_ms.store(jitter_ms, Ordering::Relaxed);
    }

    pub(super) fn current(&self) -> (u64, u64) {
        (
            self.fixed_latency_ms.load(Ordering::Relaxed),
            self.jitter_ms.load(Ordering::Relaxed),
        )
    }
}
''',
)

fault = "desktop/src-tauri/src/lab/fault.rs"
replace_once(
    fault,
    "use super::clock::LabClock;\n",
    "mod control;\n\npub(crate) use control::LabLatencyController;\n\nuse super::clock::LabClock;\n",
)
replace_once(
    fault,
    '''pub(crate) struct LabLatencyTransportFactory<F> {
    inner: F,
    clock: Arc<LabClock>,
    config: LabLatencyConfig,
}
''',
    '''pub(crate) struct LabLatencyTransportFactory<F> {
    inner: F,
    clock: Arc<LabClock>,
    controller: LabLatencyController,
    seed: u64,
}
''',
)
replace_once(
    fault,
    '''    pub(crate) fn new(inner: F, clock: Arc<LabClock>, config: LabLatencyConfig) -> Self {
        Self {
            inner,
            clock,
            config,
        }
    }
''',
    '''    pub(crate) fn new(inner: F, clock: Arc<LabClock>, config: LabLatencyConfig) -> Self {
        Self {
            inner,
            clock,
            controller: LabLatencyController::new(config),
            seed: config.seed,
        }
    }

    #[must_use]
    pub(crate) fn controller(&self) -> LabLatencyController {
        self.controller.clone()
    }
''',
)
replace_once(
    fault,
    '''            delivery_clock: clock,
            config: self.config,
            held: Mutex::new(HeldEvents {
                prng: DeterministicPrng::new(self.config.seed),
''',
    '''            delivery_clock: clock,
            controller: self.controller.clone(),
            held: Mutex::new(HeldEvents {
                prng: DeterministicPrng::new(self.seed),
''',
)
replace_once(
    fault,
    "    delivery_clock: Arc<dyn TransportClock>,\n    config: LabLatencyConfig,\n    held: Mutex<HeldEvents>,\n",
    "    delivery_clock: Arc<dyn TransportClock>,\n    controller: LabLatencyController,\n    held: Mutex<HeldEvents>,\n",
)
replace_once(
    fault,
    '''    fn compute_deadline(&self, prng: &mut DeterministicPrng, arrived_at_ms: u64) -> u64 {
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
''',
    '''    fn compute_deadline(&self, prng: &mut DeterministicPrng, arrived_at_ms: u64) -> u64 {
        let (fixed_latency_ms, jitter_ms) = self.controller.current();
        let jitter_offset = if jitter_ms == 0 {
            0
        } else {
            let span = jitter_ms.saturating_mul(2).saturating_add(1);
            let raw = prng.next_below(usize::try_from(span).unwrap_or(usize::MAX));
            i64::try_from(raw).unwrap_or(0) - i64::try_from(jitter_ms).unwrap_or(0)
        };
        let base = i64::try_from(arrived_at_ms).unwrap_or(i64::MAX)
            + i64::try_from(fixed_latency_ms).unwrap_or(i64::MAX)
            + jitter_offset;
''',
)

write_new(
    "desktop/src-tauri/src/lab/scenario/live_transport/state.rs",
    '''use super::sync::LiveSyncState;
use crate::lab::clock::LabNodeClock;
use crate::lab::fault::LabLatencyController;
use crate::platform::host_transport_events::HostTransportEventProcessor;
use silent_disco_core::domain::DeviceId;
use silent_disco_core::runtime::{CoreActorHandle, CoreNotification, SessionAdvertisement};
use silent_disco_core::transport::{
    HostTransportNode, ListenerTransportNode, VirtualUdpLossController,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

pub(super) struct ActorEndpoint {
    pub(super) handle: CoreActorHandle,
    pub(super) device_id: DeviceId,
    pub(super) clock: Arc<LabNodeClock>,
    pub(super) effects: Receiver<CoreNotification>,
    pub(super) pending_invite_codes: VecDeque<Option<String>>,
}

pub(super) struct LiveHost {
    pub(super) transport: Box<dyn HostTransportNode>,
    pub(super) advertisement: SessionAdvertisement,
    pub(super) processor: HostTransportEventProcessor,
    pub(super) loss: VirtualUdpLossController,
}

pub(super) struct LiveListener {
    pub(super) transport: Box<dyn ListenerTransportNode>,
    pub(super) sync: LiveSyncState,
    pub(super) loss: VirtualUdpLossController,
    pub(super) latency: LabLatencyController,
}
''',
)

write_new(
    "desktop/src-tauri/src/lab/scenario/live_transport/fault_control.rs",
    '''use super::super::NodeId;
use super::LiveTransportDriver;
use super::support::{live_error, transport_error};

const MAX_LOSS_PERMILLE: u16 = 1_000;

impl LiveTransportDriver {
    pub(super) fn set_receive_fault(
        &mut self,
        node_id: &NodeId,
        latency_ms: u64,
        jitter_ms: u64,
        loss_permille: u16,
    ) -> Result<(), crate::dto::DesktopErrorDto> {
        if !self.links.iter().any(|link| link.to == *node_id) {
            return Err(live_error(
                "receive_fault_route_missing",
                &format!("Lab node '{node_id}' has no declared inbound link to fault"),
            ));
        }
        if !self.profiles.contains_key(node_id) {
            return Err(live_error(
                "receive_fault_profile_missing",
                &format!("Lab node '{node_id}' has no receive-fault profile"),
            ));
        }
        if loss_permille > MAX_LOSS_PERMILLE {
            return Err(live_error(
                "receive_fault_invalid",
                "Lab receive loss must be between 0 and 1000 permille",
            ));
        }
        if self.hosts.contains_key(node_id) && (latency_ms != 0 || jitter_ms != 0) {
            return Err(live_error(
                "host_latency_unsupported",
                "host-side Lab latency/jitter is unsupported by the listener-receive latency adapter",
            ));
        }

        if let Some(host) = self.hosts.get(node_id) {
            host.loss
                .set_loss_permille(loss_permille)
                .map_err(|error| transport_error("update Lab host receive loss", &error))?;
        }
        if let Some(listener) = self.listeners.get(node_id) {
            listener
                .loss
                .set_loss_permille(loss_permille)
                .map_err(|error| transport_error("update Lab listener receive loss", &error))?;
            listener.latency.set(latency_ms, jitter_ms);
        }

        let profile = self.profiles.get_mut(node_id).ok_or_else(|| {
            live_error(
                "receive_fault_profile_missing",
                &format!("Lab node '{node_id}' lost its receive-fault profile"),
            )
        })?;
        profile.latency_ms = latency_ms;
        profile.jitter_ms = jitter_ms;
        profile.loss_permille = loss_permille;
        for link in &mut self.links {
            if link.to == *node_id {
                link.latency_ms = latency_ms;
                link.jitter_ms = jitter_ms;
                link.loss_permille = loss_permille;
            }
        }
        Ok(())
    }
}
''',
)

lt = "desktop/src-tauri/src/lab/scenario/live_transport.rs"
replace_once(
    lt,
    "mod observer;\nmod support;\nmod sync;\n",
    "mod fault_control;\nmod observer;\nmod state;\nmod support;\nmod sync;\n",
)
replace_once(
    lt,
    '''use self::support::{
    ReceiveFaultProfile, build_receive_profiles, core_error, failed_delivery_report, live_error,
    transport_error,
};
use self::sync::LiveSyncState;
''',
    '''use self::state::{ActorEndpoint, LiveHost, LiveListener};
use self::support::{
    ReceiveFaultProfile, build_receive_profiles, core_error, failed_delivery_report, live_error,
    transport_error,
};
use self::sync::LiveSyncState;
''',
)
replace_once(
    lt,
    '''struct ActorEndpoint {
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

''',
    "",
)
replace_once(
    lt,
    '''        let clock: Arc<dyn TransportClock> = node_clock;
        let transport = factory
            .bind_host(
''',
    '''        let loss = factory.loss_controller();
        let clock: Arc<dyn TransportClock> = node_clock;
        let transport = factory
            .bind_host(
''',
)
replace_once(
    lt,
    '''                advertisement: advertisement.clone(),
                processor: HostTransportEventProcessor::new(clock),
            },
''',
    '''                advertisement: advertisement.clone(),
                processor: HostTransportEventProcessor::new(clock),
                loss,
            },
''',
)
replace_once(
    lt,
    '''        let faulted = VirtualTransportFactory::new(self.network.clone()).with_udp_faults(
            VirtualUdpFaultConfig {
                seed: profile.seed,
                loss_permille: profile.loss_permille,
                ..VirtualUdpFaultConfig::default()
            },
        );
        let factory = LabLatencyTransportFactory::new(
''',
    '''        let faulted = VirtualTransportFactory::new(self.network.clone()).with_udp_faults(
            VirtualUdpFaultConfig {
                seed: profile.seed,
                loss_permille: profile.loss_permille,
                ..VirtualUdpFaultConfig::default()
            },
        );
        let loss = faulted.loss_controller();
        let factory = LabLatencyTransportFactory::new(
''',
)
replace_once(
    lt,
    '''        let factory = LabLatencyTransportFactory::new(
            faulted,
            Arc::clone(&self.shared_clock),
            LabLatencyConfig {
                fixed_latency_ms: profile.latency_ms,
                jitter_ms: profile.jitter_ms,
                seed: profile.seed,
            },
        );
        let (handle, device_id, node_clock) = self.actor_parts(node_id)?;
''',
    '''        let factory = LabLatencyTransportFactory::new(
            faulted,
            Arc::clone(&self.shared_clock),
            LabLatencyConfig {
                fixed_latency_ms: profile.latency_ms,
                jitter_ms: profile.jitter_ms,
                seed: profile.seed,
            },
        );
        let latency = factory.controller();
        let (handle, device_id, node_clock) = self.actor_parts(node_id)?;
''',
)
replace_once(
    lt,
    '''                sync: LiveSyncState::new(session_id)
                    .map_err(|error| live_error("sync_estimator_failed", &error))?,
            },
''',
    '''                sync: LiveSyncState::new(session_id)
                    .map_err(|error| live_error("sync_estimator_failed", &error))?,
                loss,
                latency,
            },
''',
)

types = "desktop/src-tauri/src/lab/scenario/schema/types.rs"
replace_once(
    types,
    '''    AmbiguousInboundLinkFaults {
        node: String,
    },
    StepsNotTimeOrdered {
''',
    '''    AmbiguousInboundLinkFaults {
        node: String,
    },
    ReceiveFaultOutOfBounds {
        field: &'static str,
        limit: u64,
    },
    ReceiveFaultTargetHasNoInboundLink {
        node: String,
    },
    StepsNotTimeOrdered {
''',
)
replace_once(
    types,
    '''            Self::AmbiguousInboundLinkFaults { node } => write!(
                formatter,
                "node '{node}' has conflicting inbound receive-fault profiles; the current virtual transport applies latency/jitter/loss per receiving node, not per peer"
            ),
            Self::StepsNotTimeOrdered { index } => write!(
''',
    '''            Self::AmbiguousInboundLinkFaults { node } => write!(
                formatter,
                "node '{node}' has conflicting inbound receive-fault profiles; the current virtual transport applies latency/jitter/loss per receiving node, not per peer"
            ),
            Self::ReceiveFaultOutOfBounds { field, limit } => {
                write!(formatter, "receive fault {field} exceeds the bound of {limit}")
            }
            Self::ReceiveFaultTargetHasNoInboundLink { node } => write!(
                formatter,
                "node '{node}' cannot change receive faults because it has no declared inbound link"
            ),
            Self::StepsNotTimeOrdered { index } => write!(
''',
)
replace_once(
    types,
    '''    SetLocalVolume {
        linear_gain: f32,
    },
    RequestResync,
''',
    '''    SetLocalVolume {
        linear_gain: f32,
    },
    #[serde(rename_all = "camelCase")]
    SetReceiveFault {
        latency_ms: u64,
        jitter_ms: u64,
        loss_permille: u16,
    },
    RequestResync,
''',
)

validation = "desktop/src-tauri/src/lab/scenario/schema/validation.rs"
validation_marker = '''            if let ScenarioAction::RemoveListener { listener_node } = &step.action
                && !known_nodes.contains(listener_node.as_str())
            {
                return Err(ScenarioValidationError::UnknownNode {
                    field: "steps[].action.listenerNode",
                    node: listener_node.to_string(),
                });
            }
'''
validation_addition = validation_marker + '''            if let ScenarioAction::SetReceiveFault {
                latency_ms,
                jitter_ms,
                loss_permille,
            } = &step.action
            {
                for (field, actual, limit) in [
                    ("latencyMs", *latency_ms, MAX_LINK_LATENCY_MS),
                    ("jitterMs", *jitter_ms, MAX_LINK_JITTER_MS),
                    (
                        "lossPermille",
                        u64::from(*loss_permille),
                        u64::from(MAX_LOSS_PERMILLE),
                    ),
                ] {
                    if actual > limit {
                        return Err(ScenarioValidationError::ReceiveFaultOutOfBounds {
                            field,
                            limit,
                        });
                    }
                }
                if !self.links.iter().any(|link| link.to == step.node) {
                    return Err(ScenarioValidationError::ReceiveFaultTargetHasNoInboundLink {
                        node: step.node.to_string(),
                    });
                }
            }
'''
replace_once(validation, validation_marker, validation_addition)

commands = "desktop/src-tauri/src/lab/scenario/commands.rs"
replace_once(
    commands,
    '''pub(super) fn action_revision_delta(action: &ScenarioAction) -> u64 {
    match action {
        ScenarioAction::CreateHostSession
''',
    '''pub(super) fn action_revision_delta(action: &ScenarioAction) -> u64 {
    match action {
        ScenarioAction::SetReceiveFault { .. } => 0,
        ScenarioAction::CreateHostSession
''',
)
replace_once(
    commands,
    '''        ScenarioAction::RemoveListener { .. }
        | ScenarioAction::InjectUnderrun { .. }
''',
    '''        ScenarioAction::RemoveListener { .. }
        | ScenarioAction::SetReceiveFault { .. }
        | ScenarioAction::InjectUnderrun { .. }
''',
)

runner = "desktop/src-tauri/src/lab/scenario/live_runner.rs"
replace_once(
    runner,
    '''    AssertionOutcome, AssertionResult, ClockAdvance, NodeId, Scenario, ScenarioExecutionError,
    ScenarioOutcome, ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
''',
    '''    AssertionOutcome, AssertionResult, ClockAdvance, NodeId, Scenario, ScenarioAction,
    ScenarioExecutionError, ScenarioOutcome, ScenarioReport, ScenarioTrace, StepResult, StepSettlement,
''',
)
runner_marker = '''        let recorder = recorders
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        let revision_before = current_revision(&handle)?;
'''
runner_replacement = '''        let recorder = recorders
            .get(step.node.as_str())
            .ok_or_else(|| ScenarioExecutionError::UnknownNode(step.node.clone()))?;
        if let ScenarioAction::SetReceiveFault {
            latency_ms,
            jitter_ms,
            loss_permille,
        } = &step.action
        {
            driver
                .set_receive_fault(&step.node, *latency_ms, *jitter_ms, *loss_permille)
                .map_err(ScenarioExecutionError::Lab)?;
            driver.pump().map_err(ScenarioExecutionError::Lab)?;
            step_results.push(StepResult {
                index,
                at_ms: step.at_ms,
                node: step.node.clone(),
                submit_error: None,
                settlement: StepSettlement::Settled,
            });
            continue;
        }
        let revision_before = current_revision(&handle)?;
'''
replace_once(runner, runner_marker, runner_replacement)

proof = "desktop/src-tauri/src/lab/scenario/live_transport_proof_tests.rs"
proof_marker = '''fn run_live_join(
    latency_ms: u64,
'''
proof_helper = r'''fn mid_run_receive_fault_scenario(
    latency_ms: u64,
    jitter_ms: u64,
    loss_permille: u16,
    barrier_ms: u64,
    timeout_ms: u64,
) -> super::Scenario {
    let document = format!(
        r#"{{
            "schemaVersion": 1,
            "seed": 77,
            "nodes": [{{"id": "host1"}}, {{"id": "listener1"}}],
            "links": [{{
                "from": "host1",
                "to": "listener1",
                "latencyMs": 0,
                "jitterMs": 0,
                "lossPermille": 0
            }}],
            "fixtures": [{{"id": "track", "displayName": "Lab Track"}}],
            "steps": [
                {{"atMs": 0, "node": "host1", "action": {{"kind": "selectRole", "role": "host"}}}},
                {{"atMs": 1, "node": "host1", "action": {{"kind": "configureHost", "sessionName": "Lab Party", "fixture": "track"}}}},
                {{"atMs": 2, "node": "host1", "action": {{"kind": "createHostSession"}}}},
                {{"atMs": 3, "node": "listener1", "action": {{"kind": "selectRole", "role": "listener"}}}},
                {{"atMs": 4, "node": "listener1", "action": {{"kind": "startDiscovery"}}}},
                {{"atMs": 5, "node": "listener1", "action": {{"kind": "selectSession", "sessionId": "session-1"}}}},
                {{"atMs": 6, "node": "listener1", "action": {{"kind": "submitJoin"}}}},
                {{"atMs": 7, "node": "host1", "action": {{"kind": "injectUnderrun", "missingFrames": 0}}}},
                {{"atMs": 7, "node": "listener1", "action": {{
                    "kind": "setReceiveFault",
                    "latencyMs": {latency_ms},
                    "jitterMs": {jitter_ms},
                    "lossPermille": {loss_permille}
                }}}},
                {{"atMs": 8, "node": "host1", "action": {{"kind": "approveJoin", "requestId": "desktop-join-1"}}}},
                {{"atMs": {barrier_ms}, "node": "listener1", "action": {{"kind": "injectUnderrun", "missingFrames": 0}}}}
            ],
            "assertions": [
                {{"kind": "listenerCountAtLeast", "byMs": {timeout_ms}, "node": "host1", "count": 1}},
                {{"kind": "lifecycleReached", "byMs": {timeout_ms}, "node": "listener1", "target": {{"machine": "listener", "state": "approved"}}}},
                {{"kind": "synchronizationWithinBounds", "byMs": {timeout_ms}, "node": "listener1", "maxAbsOffsetMs": 1000.0, "maxRoundTripMs": 1000.0}}
            ],
            "timeoutMs": {timeout_ms}
        }}"#
    );
    load_scenario_json(document.as_bytes()).expect("mid-run receive-fault scenario should parse")
}

'''
replace_once(proof, proof_marker, proof_helper + proof_marker)
new_test_marker = '''#[test]
fn one_hundred_percent_sync_loss_never_fabricates_sync_success() {
'''
new_tests = '''#[test]
fn mid_run_latency_update_changes_the_existing_listener_transport() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = mid_run_receive_fault_scenario(25, 0, 0, 33, 40);
    let (report, trace) = run_scenario_with_trace(&lab, &scenario)
        .expect("mid-run latency scenario should execute");

    assert_eq!(report.outcome, ScenarioOutcome::Completed);
    let round_trip = last_listener_round_trip_ms(&trace)
        .expect("mid-run latency must reach the live synchronization estimator");
    assert!(
        (round_trip - 25.0).abs() <= f64::EPSILON,
        "mid-run 25 ms receive latency must affect the existing listener; observed {round_trip} ms"
    );
}

#[test]
fn mid_run_full_loss_update_never_fabricates_sync_success() {
    let root = TestDirectory::new();
    let lab = LabRuntime::new(&root.0, 0).expect("lab runtime");
    let scenario = mid_run_receive_fault_scenario(0, 0, 1_000, 20, 30);
    let (report, trace) = run_scenario_with_trace(&lab, &scenario)
        .expect("mid-run loss scenario should execute");

    assert_eq!(report.outcome, ScenarioOutcome::TimedOut);
    assert_eq!(report.assertion_results[0].outcome, AssertionOutcome::Held);
    assert_eq!(report.assertion_results[1].outcome, AssertionOutcome::Held);
    assert_eq!(
        synchronization_assertion(&report),
        AssertionOutcome::TimedOut
    );
    assert!(
        last_listener_round_trip_ms(&trace).is_none(),
        "mid-run 100% receive loss must not create a synthetic sync sample"
    );
}

'''
replace_once(proof, new_test_marker, new_tests + new_test_marker)
