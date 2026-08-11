//! Production-shaped live virtual-transport adapter for deterministic Lab scenarios.

mod observer;
mod support;
mod sync;

pub(super) use observer::LiveScenarioObserver;

use self::support::{
    ReceiveFaultProfile, build_receive_profiles, core_error, failed_delivery_report, live_error,
    transport_error,
};
use self::sync::LiveSyncState;
use super::{NodeId, Scenario, ScenarioAction, scenario_node_parts};
use crate::dto::DesktopErrorDto;
use crate::lab::fault::{LabLatencyConfig, LabLatencyTransportFactory};
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
    TransportFactory, VirtualTransportFactory, VirtualTransportNetwork, VirtualUdpFaultConfig,
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

/// Synchronous Lab platform/transport adapter. It owns no detached worker:
/// scenario execution explicitly calls [`Self::pump`] after commands and
/// virtual-clock advances.
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
        mut effect_receivers: HashMap<NodeId, Receiver<CoreNotification>>,
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
            let lab_id = lab_node_ids
                .get(node.id.as_str())
                .copied()
                .ok_or_else(|| live_error("unknown_node", "scenario node was not started"))?;
            let (handle, identity, clock) = scenario_node_parts(lab, lab_id)?;
            let effects = effect_receivers.remove(&node.id).ok_or_else(|| {
                live_error(
                    "observer_missing",
                    "scenario node has no live-effect receiver",
                )
            })?;
            actors.insert(
                node.id.clone(),
                ActorEndpoint {
                    handle,
                    device_id: identity.device_id().clone(),
                    clock,
                    effects,
                    pending_invite_codes: pending_invites
                        .remove(node.id.as_str())
                        .unwrap_or_default(),
                },
            );
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

    pub(super) fn pump(&mut self) -> Result<(), DesktopErrorDto> {
        for _ in 0..MAX_PUMP_ITERATIONS {
            let mut progressed = self.process_effects()?;
            progressed |= self.process_host_events()?;
            progressed |= self.process_listener_events()?;
            if !progressed {
                return Ok(());
            }
        }
        Err(live_error(
            "did_not_quiesce",
            "Lab live transport exceeded its bounded pump iteration limit",
        ))
    }

    fn process_effects(&mut self) -> Result<bool, DesktopErrorDto> {
        let mut actor_ids: Vec<NodeId> = self.actors.keys().cloned().collect();
        actor_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let mut pending = Vec::new();
        for node_id in actor_ids {
            let actor = self.actor(&node_id)?;
            loop {
                match actor.effects.try_recv() {
                    Ok(notification) => pending.push((node_id.clone(), notification)),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        return Err(live_error(
                            "observer_disconnected",
                            "Lab actor effect observer disconnected before scenario completion",
                        ));
                    }
                }
            }
        }
        let progressed = !pending.is_empty();
        for (node_id, notification) in pending {
            match notification {
                CoreNotification::Effect(effect) => {
                    self.process_platform_effect(&node_id, effect)?;
                }
                CoreNotification::TransportEffect(effect) => {
                    self.process_transport_effect(&node_id, effect)?;
                }
                CoreNotification::StorageEffect(_) => {
                    return Err(live_error(
                        "storage_effect_unsupported",
                        "Lab live transport does not fabricate durable-storage completion; use non-persistent scenario operations",
                    ));
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
                self.start_advertising(node_id, effect.operation_id, &mut advertisement)
            }
            PlatformEffectRequest::StopAdvertising => {
                self.stop_advertising(node_id, effect.operation_id)
            }
            PlatformEffectRequest::StartDiscovery(_) => {
                self.start_discovery(node_id, effect.operation_id)
            }
            PlatformEffectRequest::StopDiscovery => self.complete_platform(
                node_id,
                effect.operation_id,
                PlatformOperationCompletion::DiscoveryStopped,
            ),
            PlatformEffectRequest::EstablishNetwork(request) => {
                self.establish_network(node_id, effect.operation_id, request.session_id)
            }
            PlatformEffectRequest::ReleaseNetwork => {
                self.release_network(node_id, effect.operation_id)
            }
            PlatformEffectRequest::RequestCapabilities(_) => self.fail_platform(
                node_id,
                effect.operation_id,
                "Lab live transport does not synthesize platform capability availability",
            ),
            PlatformEffectRequest::PrepareAudioSource(_)
            | PlatformEffectRequest::StartAudioOutput(_)
            | PlatformEffectRequest::StopAudioOutput
            | PlatformEffectRequest::ShareDiagnostics { .. } => self.fail_platform(
                node_id,
                effect.operation_id,
                "platform effect is outside the Lab live-transport adapter",
            ),
        }
    }

    fn start_advertising(
        &mut self,
        node_id: &NodeId,
        operation_id: OperationId,
        advertisement: &mut SessionAdvertisement,
    ) -> Result<(), DesktopErrorDto> {
        let profile = self.profile(node_id);
        if profile.latency_ms != 0 || profile.jitter_ms != 0 {
            return self.fail_platform(
                node_id,
                operation_id,
                "host-side Lab latency/jitter is unsupported by the listener-receive latency adapter",
            );
        }
        let (handle, _device_id, node_clock) = self.actor_parts(node_id)?;
        let factory = VirtualTransportFactory::new(self.network.clone()).with_udp_faults(
            VirtualUdpFaultConfig {
                seed: profile.seed,
                loss_permille: profile.loss_permille,
                ..VirtualUdpFaultConfig::default()
            },
        );
        let clock: Arc<dyn TransportClock> = node_clock;
        let transport = factory
            .bind_host(
                HostTransportConfig::loopback(advertisement.session_id.clone()),
                Arc::clone(&clock),
            )
            .map_err(|error| transport_error("bind host", &error))?;
        advertisement.endpoint = Some(transport.endpoint());
        self.hosts.insert(
            node_id.clone(),
            LiveHost {
                transport,
                advertisement: advertisement.clone(),
                processor: HostTransportEventProcessor::new(clock),
            },
        );
        handle
            .submit_platform_event(PlatformEvent::OperationSucceeded {
                operation_id,
                completion: PlatformOperationCompletion::AdvertisingStarted,
            })
            .map_err(core_error)?;
        self.publish_advertisement(node_id, advertisement)
    }

    fn stop_advertising(
        &mut self,
        node_id: &NodeId,
        operation_id: OperationId,
    ) -> Result<(), DesktopErrorDto> {
        if let Some(mut host) = self.hosts.remove(node_id) {
            host.transport
                .shutdown()
                .map_err(|error| transport_error("stop host", &error))?;
            self.expire_advertisement(node_id, &host.advertisement)?;
        }
        self.complete_platform(
            node_id,
            operation_id,
            PlatformOperationCompletion::AdvertisingStopped,
        )
    }

    fn start_discovery(
        &self,
        node_id: &NodeId,
        operation_id: OperationId,
    ) -> Result<(), DesktopErrorDto> {
        self.complete_platform(
            node_id,
            operation_id,
            PlatformOperationCompletion::DiscoveryStarted,
        )?;
        let mut visible: Vec<(NodeId, SessionAdvertisement)> = self
            .hosts
            .iter()
            .filter(|(host_id, _)| self.has_link(host_id, node_id))
            .map(|(host_id, host)| (host_id.clone(), host.advertisement.clone()))
            .collect();
        visible.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
        let handle = self.actor(node_id)?.handle.clone();
        for (_, advertisement) in visible {
            handle
                .submit_platform_event(PlatformEvent::SessionDiscovered(advertisement))
                .map_err(core_error)?;
        }
        Ok(())
    }

    fn establish_network(
        &mut self,
        node_id: &NodeId,
        operation_id: OperationId,
        session_id: silent_disco_core::domain::SessionId,
    ) -> Result<(), DesktopErrorDto> {
        let advertisement = self
            .hosts
            .iter()
            .find(|(host_id, host)| {
                host.advertisement.session_id == session_id && self.has_link(host_id, node_id)
            })
            .map(|(_, host)| host.advertisement.clone())
            .ok_or_else(|| {
                live_error(
                    "no_route",
                    "selected Lab session has no declared link to the listener",
                )
            })?;
        let endpoint = advertisement
            .endpoint
            .ok_or_else(|| live_error("no_endpoint", "Lab host advertisement has no endpoint"))?;
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
        let (handle, device_id, node_clock) = self.actor_parts(node_id)?;
        let clock: Arc<dyn TransportClock> = node_clock;
        let transport = factory
            .connect_listener(
                ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), endpoint),
                clock,
            )
            .map_err(|error| transport_error("connect listener", &error))?;
        let routes = transport.local_routes();
        let invite_code = self
            .actors
            .get_mut(node_id)
            .and_then(|actor| actor.pending_invite_codes.pop_front())
            .flatten();
        transport
            .send_control(&ControlMessage::JoinRequest(JoinRequest {
                session_id: session_id.clone(),
                device: DeviceIdentity {
                    device_id,
                    display_name: node_id.as_str().to_owned(),
                },
                invite_code,
                sync_port: routes.synchronization.port(),
                audio_port: routes.audio.port(),
            }))
            .map_err(|error| transport_error("send join request", &error))?;
        self.listeners.insert(
            node_id.clone(),
            LiveListener {
                transport,
                sync: LiveSyncState::new(session_id)
                    .map_err(|error| live_error("sync_estimator_failed", &error))?,
            },
        );
        handle
            .submit_platform_event(PlatformEvent::OperationSucceeded {
                operation_id,
                completion: PlatformOperationCompletion::NetworkEndpointReady(endpoint),
            })
            .map_err(core_error)
    }

    fn release_network(
        &mut self,
        node_id: &NodeId,
        operation_id: OperationId,
    ) -> Result<(), DesktopErrorDto> {
        if let Some(mut listener) = self.listeners.remove(node_id) {
            listener
                .transport
                .shutdown()
                .map_err(|error| transport_error("release listener", &error))?;
        }
        self.complete_platform(
            node_id,
            operation_id,
            PlatformOperationCompletion::NetworkReleased,
        )
    }

    fn process_transport_effect(
        &mut self,
        node_id: &NodeId,
        effect: TransportEffect,
    ) -> Result<(), DesktopErrorDto> {
        let handle = self.actor(node_id)?.handle.clone();
        let host = self
            .hosts
            .get_mut(node_id)
            .ok_or_else(|| live_error("host_missing", "transport effect has no live Lab host"))?;
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
                            listener_id: listener_id.clone(),
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
                            listener_id: listener_id.clone(),
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
                            listener_id: listener_id.clone(),
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
        handle
            .submit_transport_event(CoreTransportEvent::DeliveryCompleted {
                operation_id: effect.operation_id,
                report,
            })
            .map_err(core_error)
    }

    fn process_host_events(&mut self) -> Result<bool, DesktopErrorDto> {
        let mut host_ids: Vec<NodeId> = self.hosts.keys().cloned().collect();
        host_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let mut progressed = false;
        for host_id in host_ids {
            loop {
                let event = {
                    let host = self.hosts.get_mut(&host_id).ok_or_else(|| {
                        live_error("host_missing", "Lab host disappeared while pumping")
                    })?;
                    match host.transport.recv_event(NONBLOCKING_RECV_BUDGET) {
                        Ok(event) => event,
                        Err(error) if error.kind == TransportErrorKind::Timeout => break,
                        Err(error) => {
                            return Err(transport_error("receive host event", &error));
                        }
                    }
                };
                progressed = true;
                let handle = self.actor(&host_id)?.handle.clone();
                let host = self.hosts.get_mut(&host_id).ok_or_else(|| {
                    live_error(
                        "host_missing",
                        "Lab host disappeared while processing event",
                    )
                })?;
                if let Some(message) = host
                    .processor
                    .process_for_lab(event, host.transport.as_ref(), &host.advertisement, &handle)
                    .map_err(|message| live_error("host_event_failed", &message))?
                {
                    return Err(live_error("host_event_rejected", &message));
                }
            }
        }
        Ok(progressed)
    }

    fn process_listener_events(&mut self) -> Result<bool, DesktopErrorDto> {
        let mut listener_ids: Vec<NodeId> = self.listeners.keys().cloned().collect();
        listener_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        let mut progressed = false;
        for listener_id in listener_ids {
            loop {
                let event = {
                    let listener = self.listeners.get(&listener_id).ok_or_else(|| {
                        live_error("listener_missing", "Lab listener disappeared while pumping")
                    })?;
                    match listener.transport.recv_event(NONBLOCKING_RECV_BUDGET) {
                        Ok(event) => event,
                        Err(error) if error.kind == TransportErrorKind::Timeout => break,
                        Err(error) => {
                            return Err(transport_error("receive listener event", &error));
                        }
                    }
                };
                progressed = true;
                self.apply_listener_event(&listener_id, event)?;
            }
        }
        Ok(progressed)
    }

    fn apply_listener_event(
        &mut self,
        listener_id: &NodeId,
        event: RuntimeTransportEvent,
    ) -> Result<(), DesktopErrorDto> {
        match event {
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Hello(_)),
                ..
            } => self
                .actor(listener_id)?
                .handle
                .submit_transport_event(CoreTransportEvent::AwaitingApproval)
                .map_err(core_error),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(value)),
                ..
            } => {
                self.actor(listener_id)?
                    .handle
                    .submit_transport_event(CoreTransportEvent::JoinApproved {
                        trusted_for_future: value.trusted_for_future,
                    })
                    .map_err(core_error)?;
                self.send_sync_probe(listener_id)
            }
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinRejection(value)),
                ..
            } => self
                .actor(listener_id)?
                .handle
                .submit_transport_event(CoreTransportEvent::JoinRejected {
                    reason: value.reason,
                })
                .map_err(core_error),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Disconnect(value)),
                ..
            } => self
                .actor(listener_id)?
                .handle
                .submit_transport_event(CoreTransportEvent::SessionEnded {
                    session_id: value.session_id,
                })
                .map_err(core_error),
            RuntimeTransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                frame: ProtocolFrame::SyncResponse(response),
                received_at,
                ..
            } => self.apply_sync_response(listener_id, response, received_at),
            RuntimeTransportEvent::Rejected { error, .. } => {
                Err(transport_error("listener received rejected frame", &error))
            }
            RuntimeTransportEvent::PeerDisconnected {
                error: Some(error), ..
            } => Err(transport_error("listener peer disconnected", &error)),
            RuntimeTransportEvent::PeerDisconnected { error: None, .. }
            | RuntimeTransportEvent::PeerAccepted { .. }
            | RuntimeTransportEvent::PeerAuthorized { .. }
            | RuntimeTransportEvent::FrameReceived { .. } => Ok(()),
        }
    }

    fn send_sync_probe(&mut self, listener_id: &NodeId) -> Result<(), DesktopErrorDto> {
        let (_handle, _device_id, node_clock) = self.actor_parts(listener_id)?;
        let listener = self
            .listeners
            .get_mut(listener_id)
            .ok_or_else(|| live_error("listener_missing", "sync probe has no live Lab listener"))?;
        let LiveListener { transport, sync } = listener;
        sync.send_probe(transport.as_ref(), node_clock.now())
            .map_err(|error| live_error("sync_probe_failed", &error))
    }

    fn apply_sync_response(
        &mut self,
        listener_id: &NodeId,
        response: silent_disco_core::protocol::SyncResponse,
        received_at: silent_disco_core::domain::MonotonicMillis,
    ) -> Result<(), DesktopErrorDto> {
        let (handle, device_id, _clock) = self.actor_parts(listener_id)?;
        let listener = self.listeners.get_mut(listener_id).ok_or_else(|| {
            live_error("listener_missing", "sync response has no live Lab listener")
        })?;
        let LiveListener { transport, sync } = listener;
        let (summary, report) = sync
            .observe_response(device_id.clone(), response, received_at)
            .map_err(|error| live_error("sync_response_rejected", &error))?;
        handle
            .submit_audio_event(AudioEvent::SynchronizationUpdated { device_id, summary })
            .map_err(core_error)?;
        transport
            .send_control(&report)
            .map_err(|error| transport_error("send synchronization report", &error))?;
        Ok(())
    }

    fn complete_platform(
        &self,
        node_id: &NodeId,
        operation_id: OperationId,
        completion: PlatformOperationCompletion,
    ) -> Result<(), DesktopErrorDto> {
        self.actor(node_id)?
            .handle
            .submit_platform_event(PlatformEvent::OperationSucceeded {
                operation_id,
                completion,
            })
            .map_err(core_error)
    }

    fn publish_advertisement(
        &self,
        host_id: &NodeId,
        advertisement: &SessionAdvertisement,
    ) -> Result<(), DesktopErrorDto> {
        let mut listener_ids: Vec<NodeId> = self.actors.keys().cloned().collect();
        listener_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for listener_id in listener_ids {
            if !self.has_link(host_id, &listener_id) {
                continue;
            }
            let actor = self.actor(&listener_id)?;
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
        host_id: &NodeId,
        advertisement: &SessionAdvertisement,
    ) -> Result<(), DesktopErrorDto> {
        let mut listener_ids: Vec<NodeId> = self.actors.keys().cloned().collect();
        listener_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for listener_id in listener_ids {
            if !self.has_link(host_id, &listener_id) {
                continue;
            }
            let actor = self.actor(&listener_id)?;
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
        .map_err(|error| live_error("error_shape_invalid", &error.to_string()))?;
        self.actor(node_id)?
            .handle
            .submit_platform_event(PlatformEvent::OperationFailed {
                operation_id,
                error: failure,
            })
            .map_err(core_error)
    }

    fn actor(&self, node_id: &NodeId) -> Result<&ActorEndpoint, DesktopErrorDto> {
        self.actors
            .get(node_id)
            .ok_or_else(|| live_error("unknown_node", "Lab live transport does not know this node"))
    }

    fn actor_parts(
        &self,
        node_id: &NodeId,
    ) -> Result<
        (
            CoreActorHandle,
            DeviceId,
            Arc<crate::lab::clock::LabNodeClock>,
        ),
        DesktopErrorDto,
    > {
        let actor = self.actor(node_id)?;
        Ok((
            actor.handle.clone(),
            actor.device_id.clone(),
            Arc::clone(&actor.clock),
        ))
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
