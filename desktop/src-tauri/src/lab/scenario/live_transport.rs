//! Live virtual-transport adapter for Lab scenarios.
//!
//! This is the missing bridge between Block 40's scenario schema and Block
//! 39's real virtual/fault transport. It executes actor-emitted platform and
//! transport effects against one shared [`VirtualTransportNetwork`], then
//! feeds facts observed on that wire back into the authoritative actors.
//! Control traffic still goes through the production codec because the
//! virtual transport itself round-trips every frame through encode/decode.

use super::{NodeId, Scenario, ScenarioAction};
use crate::dto::DesktopErrorDto;
use crate::lab::fault::{LabLatencyConfig, LabLatencyTransportFactory};
use crate::lab::recorder::{RecordingObserver, ScenarioRecorder};
use crate::lab::{LabClock, LabNodeId, LabRuntime};
use crate::platform::host_transport_events::HostTransportEventProcessor;
use silent_disco_core::domain::{DeliverySeverity, DeviceId, ErrorSeverity, OperationId};
use silent_disco_core::error::{CoreError, CoreErrorCode};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, Disconnect, JoinApproval, JoinRejection, JoinRequest,
    ProtocolFrame,
};
use silent_disco_core::runtime::{
    CoreActorHandle, CoreNotification, CoreObserver, DeliveryReport, PlatformEffect,
    PlatformEffectRequest, PlatformEvent, PlatformOperationCompletion, SessionAdvertisement,
    TransportEffect, TransportEffectRequest, TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    TransportChannel, TransportClock, TransportErrorKind, TransportEvent as RuntimeTransportEvent,
    TransportFactory, VirtualTransportFactory, VirtualTransportNetwork, VirtualUdpFaultConfig,
};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::time::Duration;

const EFFECT_QUEUE_CAPACITY: usize = 128;
const MAX_PUMP_ITERATIONS: usize = 512;
const NONBLOCKING_RECV_BUDGET: Duration = Duration::from_nanos(1);

/// Observer used by a live scenario node. Every notification still reaches
/// the ordinary recorder first. Effect notifications are additionally copied
/// into a small bounded queue consumed synchronously by [`LiveTransportDriver`].
pub(super) struct LiveScenarioObserver {
    recorder: RecordingObserver,
    effects: SyncSender<CoreNotification>,
}

impl LiveScenarioObserver {
    pub(super) fn new(recorder: Arc<ScenarioRecorder>) -> (Self, Receiver<CoreNotification>) {
        let (effects, receiver) = mpsc::sync_channel(EFFECT_QUEUE_CAPACITY);
        (
            Self {
                recorder: RecordingObserver(recorder),
                effects,
            },
            receiver,
        )
    }
}

impl CoreObserver for LiveScenarioObserver {
    fn on_notification(&self, notification: CoreNotification) -> Result<(), CoreError> {
        self.recorder.on_notification(notification.clone())?;
        if !matches!(
            notification,
            CoreNotification::Effect(_) | CoreNotification::TransportEffect(_)
        ) {
            return Ok(());
        }
        self.effects.try_send(notification).map_err(|error| {
            let message = match error {
                TrySendError::Full(_) => "Lab live-effect queue reached its bounded capacity",
                TrySendError::Disconnected(_) => "Lab live-effect consumer disconnected",
            };
            CoreError::new(
                CoreErrorCode::QueueOverflow,
                message,
                ErrorSeverity::Error,
                true,
                None,
            )
            .expect("static Lab live-effect observer error is bounded and control-free")
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ReceiveFaultProfile {
    latency_ms: u64,
    jitter_ms: u64,
    loss_permille: u16,
    seed: u64,
}

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
    host_node: NodeId,
}

/// Synchronous Lab platform/transport adapter. It owns no background worker:
/// the scenario runner calls [`Self::pump`] after commands and virtual-clock
/// advances, so all transport progress stays causally tied to the scenario.
pub(super) struct LiveTransportDriver {
    network: VirtualTransportNetwork,
    shared_clock: Arc<LabClock>,
    links: Vec<super::ScenarioLink>,
    profiles: HashMap<NodeId, ReceiveFaultProfile>,
    actors: HashMap<NodeId, ActorEndpoint>,
    hosts: HashMap<NodeId, LiveHost>,
    listeners: HashMap<NodeId, LiveListener>,
}

impl LiveTransportDriver {
    pub(super) fn new(
        lab: &LabRuntime,
        scenario: &Scenario,
        lab_node_ids: &HashMap<&str, LabNodeId>,
        effect_receivers: HashMap<NodeId, Receiver<CoreNotification>>,
    ) -> Result<Self, DesktopErrorDto> {
        let profiles = build_receive_profiles(scenario)?;
        let mut pending_invites: HashMap<&str, VecDeque<Option<String>>> = HashMap::new();
        for step in &scenario.steps {
            if let ScenarioAction::SubmitJoin { invite_code } = &step.action {
                pending_invites
                    .entry(step.node.as_str())
                    .or_default()
                    .push_back(invite_code.clone());
            }
        }

        let mut actors = HashMap::new();
        for node in &scenario.nodes {
            let lab_id = lab_node_ids.get(node.id.as_str()).copied().ok_or_else(|| {
                error("desktop.lab.live_transport_unknown_node", "scenario node was not started")
            })?;
            let handle = lab.node_handle(lab_id).ok_or_else(|| {
                error("desktop.lab.live_transport_unknown_node", "scenario node actor is unavailable")
            })?;
            let identity = lab.node_identity(lab_id).ok_or_else(|| {
                error("desktop.lab.live_transport_unknown_node", "scenario node identity is unavailable")
            })?;
            let clock = lab.node_clock(lab_id).ok_or_else(|| {
                error("desktop.lab.live_transport_unknown_node", "scenario node clock is unavailable")
            })?;
            let effects = effect_receivers.get(&node.id).ok_or_else(|| {
                error("desktop.lab.live_transport_observer_missing", "scenario node has no live-effect receiver")
            })?;
            // Receiver is not Clone; remove it below after all lookups are proven.
            let _ = effects;
            actors.insert(
                node.id.clone(),
                ActorEndpoint {
                    handle,
                    device_id: identity.device_id().clone(),
                    clock,
                    effects: mpsc::sync_channel(1).1,
                    pending_invite_codes: pending_invites
                        .remove(node.id.as_str())
                        .unwrap_or_default(),
                },
            );
        }
        let mut effect_receivers = effect_receivers;
        for (node_id, actor) in &mut actors {
            actor.effects = effect_receivers.remove(node_id).ok_or_else(|| {
                error("desktop.lab.live_transport_observer_missing", "scenario node has no live-effect receiver")
            })?;
        }

        Ok(Self {
            network: VirtualTransportNetwork::default(),
            shared_clock: Arc::clone(&lab.clock),
            links: scenario.links.clone(),
            profiles,
            actors,
            hosts: HashMap::new(),
            listeners: HashMap::new(),
        })
    }

    /// Executes all currently-observable effects and wire events. Returns
    /// once a complete pass makes no progress; actor workers may enqueue a
    /// later effect, so the scenario settlement loop calls this repeatedly.
    pub(super) fn pump(&mut self) -> Result<(), DesktopErrorDto> {
        for _ in 0..MAX_PUMP_ITERATIONS {
            let mut progressed = self.process_effects()?;
            progressed |= self.process_host_events()?;
            progressed |= self.process_listener_events()?;
            if !progressed {
                return Ok(());
            }
        }
        Err(error(
            "desktop.lab.live_transport_did_not_quiesce",
            "Lab live transport exceeded its bounded pump iteration limit",
        ))
    }

    fn process_effects(&mut self) -> Result<bool, DesktopErrorDto> {
        let mut pending = Vec::new();
        for (node_id, actor) in &self.actors {
            loop {
                match actor.effects.try_recv() {
                    Ok(notification) => pending.push((node_id.clone(), notification)),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        return Err(error(
                            "desktop.lab.live_transport_observer_disconnected",
                            "Lab actor effect observer disconnected before scenario completion",
                        ));
                    }
                }
            }
        }
        let progressed = !pending.is_empty();
        for (node_id, notification) in pending {
            match notification {
                CoreNotification::Effect(effect) => self.process_platform_effect(&node_id, effect)?,
                CoreNotification::TransportEffect(effect) => {
                    self.process_transport_effect(&node_id, effect)?;
                }
                _ => {}
            }
        }
        Ok(progressed)
    }

    fn process_platform_effect(
        &mut self,
        node_id: &NodeId,
        effect: PlatformEffect,
    ) -> Result<(), DesktopErrorDto> {
        match effect.request {
            PlatformEffectRequest::StartAdvertising(mut advertisement) => {
                let profile = self.profile(node_id);
                if profile.latency_ms != 0 || profile.jitter_ms != 0 {
                    return self.fail_platform(
                        node_id,
                        effect.operation_id,
                        "host-side Lab latency/jitter is unsupported by the current listener-receive latency adapter",
                    );
                }
                let actor = self.actor(node_id)?;
                let factory = VirtualTransportFactory::new(self.network.clone()).with_udp_faults(
                    VirtualUdpFaultConfig {
                        seed: profile.seed,
                        loss_permille: profile.loss_permille,
                        ..VirtualUdpFaultConfig::default()
                    },
                );
                let clock: Arc<dyn TransportClock> = actor.clock.clone();
                let transport = factory
                    .bind_host(HostTransportConfig::loopback(advertisement.session_id.clone()), Arc::clone(&clock))
                    .map_err(|e| transport_error("bind host", &e))?;
                advertisement.endpoint = Some(transport.endpoint());
                let processor = HostTransportEventProcessor::new(clock);
                self.hosts.insert(
                    node_id.clone(),
                    LiveHost {
                        transport,
                        advertisement: advertisement.clone(),
                        processor,
                    },
                );
                actor
                    .handle
                    .submit_platform_event(PlatformEvent::OperationSucceeded {
                        operation_id: effect.operation_id,
                        completion: PlatformOperationCompletion::AdvertisingStarted,
                    })
                    .map_err(core_error)?;
                self.publish_advertisement(node_id, &advertisement)?;
            }
            PlatformEffectRequest::StopAdvertising => {
                if let Some(mut host) = self.hosts.remove(node_id) {
                    host.transport.shutdown().map_err(|e| transport_error("stop host", &e))?;
                    self.expire_advertisement(&host.advertisement)?;
                }
                self.actor(node_id)?
                    .handle
                    .submit_platform_event(PlatformEvent::OperationSucceeded {
                        operation_id: effect.operation_id,
                        completion: PlatformOperationCompletion::AdvertisingStopped,
                    })
                    .map_err(core_error)?;
            }
            PlatformEffectRequest::StartDiscovery(_) => {
                self.actor(node_id)?
                    .handle
                    .submit_platform_event(PlatformEvent::OperationSucceeded {
                        operation_id: effect.operation_id,
                        completion: PlatformOperationCompletion::DiscoveryStarted,
                    })
                    .map_err(core_error)?;
                let visible: Vec<SessionAdvertisement> = self
                    .hosts
                    .iter()
                    .filter(|(host_id, _)| self.has_link(host_id, node_id))
                    .map(|(_, host)| host.advertisement.clone())
                    .collect();
                for advertisement in visible {
                    self.actor(node_id)?
                        .handle
                        .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement))
                        .map_err(core_error)?;
                }
            }
            PlatformEffectRequest::StopDiscovery => {
                self.actor(node_id)?
                    .handle
                    .submit_platform_event(PlatformEvent::OperationSucceeded {
                        operation_id: effect.operation_id,
                        completion: PlatformOperationCompletion::DiscoveryStopped,
                    })
                    .map_err(core_error)?;
            }
            PlatformEffectRequest::EstablishNetwork(request) => {
                let (host_id, advertisement) = self
                    .hosts
                    .iter()
                    .find(|(host_id, host)| {
                        host.advertisement.session_id == request.session_id
                            && self.has_link(host_id, node_id)
                    })
                    .map(|(id, host)| (id.clone(), host.advertisement.clone()))
                    .ok_or_else(|| error(
                        "desktop.lab.live_transport_no_route",
                        "selected Lab session has no declared link to the listener",
                    ))?;
                let endpoint = advertisement.endpoint.ok_or_else(|| {
                    error("desktop.lab.live_transport_no_endpoint", "Lab host advertisement has no endpoint")
                })?;
                let profile = self.profile(node_id);
                let faulted = VirtualTransportFactory::new(self.network.clone()).with_udp_faults(
                    VirtualUdpFaultConfig {
                        seed: profile.seed,
                        loss_permille: profile.loss_permille,
                        ..VirtualUdpFaultConfig::default()
                    },
                );
                let factory = LabLatencyTransportFactory::new(
                    faulted,
                    Arc::clone(&self.shared_clock),
                    LabLatencyConfig {
                        fixed_latency_ms: profile.latency_ms,
                        jitter_ms: profile.jitter_ms,
                        seed: profile.seed,
                    },
                );
                let actor = self.actor(node_id)?;
                let clock: Arc<dyn TransportClock> = actor.clock.clone();
                let transport = factory
                    .connect_listener(
                        ListenerTransportConfig::loopback(
                            request.session_id.clone(),
                            actor.device_id.clone(),
                            endpoint,
                        ),
                        clock,
                    )
                    .map_err(|e| transport_error("connect listener", &e))?;
                let routes = transport.local_routes();
                let invite_code = self
                    .actors
                    .get_mut(node_id)
                    .and_then(|actor| actor.pending_invite_codes.pop_front())
                    .flatten();
                transport
                    .send_control(&ControlMessage::JoinRequest(JoinRequest {
                        session_id: request.session_id.clone(),
                        device: DeviceIdentity {
                            device_id: actor.device_id.clone(),
                            display_name: node_id.as_str().to_owned(),
                        },
                        invite_code,
                        sync_port: routes.synchronization.port(),
                        audio_port: routes.audio.port(),
                    }))
                    .map_err(|e| transport_error("send join request", &e))?;
                self.listeners.insert(
                    node_id.clone(),
                    LiveListener {
                        transport,
                        host_node: host_id,
                    },
                );
                actor
                    .handle
                    .submit_platform_event(PlatformEvent::OperationSucceeded {
                        operation_id: effect.operation_id,
                        completion: PlatformOperationCompletion::NetworkEndpointReady(endpoint),
                    })
                    .map_err(core_error)?;
            }
            PlatformEffectRequest::ReleaseNetwork => {
                if let Some(mut listener) = self.listeners.remove(node_id) {
                    listener.transport.shutdown().map_err(|e| transport_error("release listener", &e))?;
                }
                self.actor(node_id)?
                    .handle
                    .submit_platform_event(PlatformEvent::OperationSucceeded {
                        operation_id: effect.operation_id,
                        completion: PlatformOperationCompletion::NetworkReleased,
                    })
                    .map_err(core_error)?;
            }
            PlatformEffectRequest::RequestCapabilities(_) => {
                return self.fail_platform(
                    node_id,
                    effect.operation_id,
                    "Lab live transport does not synthesize platform capability availability",
                );
            }
            PlatformEffectRequest::PrepareAudioSource(_)
            | PlatformEffectRequest::StartAudioOutput(_)
            | PlatformEffectRequest::StopAudioOutput
            | PlatformEffectRequest::ShareDiagnostics { .. } => {
                // These platform responsibilities are outside live transport.
                // Leaving their operation pending would strand the actor, so
                // fail them explicitly rather than claiming fake success.
                return self.fail_platform(
                    node_id,
                    effect.operation_id,
                    "platform effect is outside the Lab live-transport adapter",
                );
            }
        }
        Ok(())
    }

    fn process_transport_effect(
        &mut self,
        node_id: &NodeId,
        effect: TransportEffect,
    ) -> Result<(), DesktopErrorDto> {
        let host = self.hosts.get_mut(node_id).ok_or_else(|| {
            error("desktop.lab.live_transport_host_missing", "transport effect has no live Lab host")
        })?;
        let (delivery, authorize) = match effect.request {
            TransportEffectRequest::DeliverJoinApproval {
                session_id,
                listener_id,
                trusted_for_future,
                ..
            } => {
                let authorize = host
                    .processor
                    .take_pending_ports(&listener_id)
                    .map(|(sync, audio)| (listener_id.clone(), sync, audio));
                (
                    host.transport.send_pending_control(
                        &listener_id,
                        &ControlMessage::JoinApproval(JoinApproval {
                            session_id,
                            listener_id,
                            trusted_for_future,
                        }),
                    ),
                    authorize,
                )
            }
            TransportEffectRequest::DeliverJoinRejection {
                session_id,
                listener_id,
                reason_code,
                ..
            } => {
                host.processor.take_pending_ports(&listener_id);
                (
                    host.transport.send_pending_control(
                        &listener_id,
                        &ControlMessage::JoinRejection(JoinRejection {
                            session_id,
                            listener_id,
                            reason: reason_code,
                        }),
                    ),
                    None,
                )
            }
            TransportEffectRequest::DisconnectListener {
                session_id,
                listener_id,
                reason_code,
            } => {
                host.processor.take_pending_ports(&listener_id);
                (
                    host.transport.send_pending_control(
                        &listener_id,
                        &ControlMessage::Disconnect(Disconnect {
                            session_id,
                            listener_id,
                            reason: reason_code,
                        }),
                    ),
                    None,
                )
            }
        };

        let mut report = match delivery {
            Ok(delivery) => delivery.report,
            Err(_) => failed_delivery_report(),
        };
        if let Some((listener_id, sync_port, audio_port)) = authorize
            && report.successful_peers > 0
            && host
                .transport
                .authorize_peer_ports(&listener_id, sync_port, audio_port)
                .is_err()
        {
            report = failed_delivery_report();
        }
        self.actor(node_id)?
            .handle
            .submit_transport_event(CoreTransportEvent::DeliveryCompleted {
                operation_id: effect.operation_id,
                report,
            })
            .map_err(core_error)
    }

    fn process_host_events(&mut self) -> Result<bool, DesktopErrorDto> {
        let host_ids: Vec<NodeId> = self.hosts.keys().cloned().collect();
        let mut progressed = false;
        for host_id in host_ids {
            loop {
                let event = {
                    let host = self.hosts.get_mut(&host_id).ok_or_else(|| {
                        error("desktop.lab.live_transport_host_missing", "Lab host disappeared while pumping")
                    })?;
                    match host.transport.recv_event(NONBLOCKING_RECV_BUDGET) {
                        Ok(event) => event,
                        Err(error) if error.kind == TransportErrorKind::Timeout => break,
                        Err(error) => return Err(transport_error("receive host event", &error)),
                    }
                };
                progressed = true;
                let actor = self.actor(&host_id)?.handle.clone();
                let host = self.hosts.get_mut(&host_id).ok_or_else(|| {
                    error("desktop.lab.live_transport_host_missing", "Lab host disappeared while processing event")
                })?;
                if let Some(message) = host
                    .processor
                    .process_for_lab(event, host.transport.as_ref(), &host.advertisement, &actor)
                    .map_err(|message| error("desktop.lab.live_transport_host_event_failed", &message))?
                {
                    return Err(error("desktop.lab.live_transport_host_event_rejected", &message));
                }
            }
        }
        Ok(progressed)
    }

    fn process_listener_events(&mut self) -> Result<bool, DesktopErrorDto> {
        let listener_ids: Vec<NodeId> = self.listeners.keys().cloned().collect();
        let mut progressed = false;
        for listener_id in listener_ids {
            loop {
                let event = {
                    let listener = self.listeners.get(&listener_id).ok_or_else(|| {
                        error("desktop.lab.live_transport_listener_missing", "Lab listener disappeared while pumping")
                    })?;
                    match listener.transport.recv_event(NONBLOCKING_RECV_BUDGET) {
                        Ok(event) => event,
                        Err(error) if error.kind == TransportErrorKind::Timeout => break,
                        Err(error) => return Err(transport_error("receive listener event", &error)),
                    }
                };
                progressed = true;
                self.apply_listener_event(&listener_id, event)?;
            }
        }
        Ok(progressed)
    }

    fn apply_listener_event(
        &self,
        listener_id: &NodeId,
        event: RuntimeTransportEvent,
    ) -> Result<(), DesktopErrorDto> {
        let handle = &self.actor(listener_id)?.handle;
        match event {
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Hello(_)),
                ..
            } => handle
                .submit_transport_event(CoreTransportEvent::AwaitingApproval)
                .map_err(core_error),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(value)),
                ..
            } => handle
                .submit_transport_event(CoreTransportEvent::JoinApproved {
                    trusted_for_future: value.trusted_for_future,
                })
                .map_err(core_error),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinRejection(value)),
                ..
            } => handle
                .submit_transport_event(CoreTransportEvent::JoinRejected { reason: value.reason })
                .map_err(core_error),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Disconnect(value)),
                ..
            } => handle
                .submit_transport_event(CoreTransportEvent::SessionEnded {
                    session_id: value.session_id,
                })
                .map_err(core_error),
            RuntimeTransportEvent::Rejected { error: transport, .. } => Err(transport_error(
                "listener received rejected frame",
                &transport,
            )),
            RuntimeTransportEvent::PeerDisconnected { error: Some(transport), .. } => {
                Err(transport_error("listener peer disconnected", &transport))
            }
            RuntimeTransportEvent::PeerDisconnected { error: None, .. }
            | RuntimeTransportEvent::PeerAccepted { .. }
            | RuntimeTransportEvent::PeerAuthorized { .. }
            | RuntimeTransportEvent::FrameReceived { .. } => Ok(()),
        }
    }

    fn publish_advertisement(
        &self,
        host_id: &NodeId,
        advertisement: &SessionAdvertisement,
    ) -> Result<(), DesktopErrorDto> {
        for (listener_id, actor) in &self.actors {
            if !self.has_link(host_id, listener_id) {
                continue;
            }
            let snapshot = actor.handle.current_snapshot().map_err(core_error)?;
            if snapshot.discovery_active {
                actor
                    .handle
                    .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement.clone()))
                    .map_err(core_error)?;
            }
        }
        Ok(())
    }

    fn expire_advertisement(
        &self,
        advertisement: &SessionAdvertisement,
    ) -> Result<(), DesktopErrorDto> {
        for actor in self.actors.values() {
            let snapshot = actor.handle.current_snapshot().map_err(core_error)?;
            if snapshot.discovery_active {
                actor
                    .handle
                    .submit_platform_event(PlatformEvent::SessionExpired {
                        session_id: advertisement.session_id.clone(),
                    })
                    .map_err(core_error)?;
            }
        }
        Ok(())
    }

    fn fail_platform(
        &self,
        node_id: &NodeId,
        operation_id: OperationId,
        message: &str,
    ) -> Result<(), DesktopErrorDto> {
        let failure = CoreError::new(
            CoreErrorCode::PlatformOperationFailed,
            message,
            ErrorSeverity::Error,
            false,
            Some(operation_id.clone()),
        )
        .map_err(|validation| error("desktop.lab.live_transport_error_shape", &validation.to_string()))?;
        self.actor(node_id)?
            .handle
            .submit_platform_event(PlatformEvent::OperationFailed {
                operation_id,
                error: failure,
            })
            .map_err(core_error)
    }

    fn actor(&self, node_id: &NodeId) -> Result<&ActorEndpoint, DesktopErrorDto> {
        self.actors.get(node_id).ok_or_else(|| {
            error("desktop.lab.live_transport_unknown_node", "Lab live transport does not know this node")
        })
    }

    fn profile(&self, node_id: &NodeId) -> ReceiveFaultProfile {
        self.profiles.get(node_id).copied().unwrap_or_default()
    }

    fn has_link(&self, from: &NodeId, to: &NodeId) -> bool {
        self.links
            .iter()
            .any(|link| &link.from == from && &link.to == to)
    }
}

fn build_receive_profiles(
    scenario: &Scenario,
) -> Result<HashMap<NodeId, ReceiveFaultProfile>, DesktopErrorDto> {
    let mut profiles = HashMap::new();
    for link in &scenario.links {
        let candidate = ReceiveFaultProfile {
            latency_ms: link.latency_ms,
            jitter_ms: link.jitter_ms,
            loss_permille: link.loss_permille,
            seed: link_seed(scenario.seed, &link.from, &link.to),
        };
        if let Some(existing) = profiles.get(&link.to) {
            if existing.latency_ms != candidate.latency_ms
                || existing.jitter_ms != candidate.jitter_ms
                || existing.loss_permille != candidate.loss_permille
            {
                return Err(error(
                    "desktop.lab.live_transport_ambiguous_link_faults",
                    "multiple links targeting one Lab node must use the same receive-side latency/jitter/loss profile",
                ));
            }
        } else {
            profiles.insert(link.to.clone(), candidate);
        }
    }
    Ok(profiles)
}

fn link_seed(base: u64, from: &NodeId, to: &NodeId) -> u64 {
    let mut value = base ^ 0x9E37_79B9_7F4A_7C15;
    for byte in from.as_str().bytes().chain([0xFF]).chain(to.as_str().bytes()) {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x100_0000_01B3);
    }
    value
}

const fn failed_delivery_report() -> DeliveryReport {
    DeliveryReport {
        intended_peers: 1,
        successful_peers: 0,
        failed_peers: 1,
        severity: DeliverySeverity::PartialFailure,
    }
}

fn core_error(error_value: CoreError) -> DesktopErrorDto {
    error(
        "desktop.lab.live_transport_core_rejected_fact",
        &error_value.to_string(),
    )
}

fn transport_error(context: &str, transport: &silent_disco_core::transport::TransportError) -> DesktopErrorDto {
    error(
        "desktop.lab.live_transport_failed",
        &format!("{context}: {transport}"),
    )
}

fn error(code: &str, message: &str) -> DesktopErrorDto {
    DesktopErrorDto::new(code, "transport", "error", false, message)
}
