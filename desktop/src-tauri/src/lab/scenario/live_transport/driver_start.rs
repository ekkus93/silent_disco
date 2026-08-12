impl LiveTransportDriver {
    pub(super) fn new(
        lab: &LabRuntime,
        scenario: &Scenario,
        lab_node_ids: &HashMap<&str, LabNodeId>,
        mut effect_receivers: HashMap<NodeId, Receiver<CoreNotification>>,
    ) -> Result<Self, DesktopErrorDto> {
        let profiles = build_receive_profiles(scenario)?;
        let fault_controllers = build_fault_controllers(&profiles);
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
            fault_controllers,
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
        if self.hosts.iter().any(|(host_id, host)| {
            host_id != node_id && host.advertisement.session_id == advertisement.session_id
        }) {
            return self.fail_platform(
                node_id,
                operation_id,
                "multiple Lab hosts produced the same core session ID; refusing ambiguous live routing",
            );
        }
        let (handle, _device_id, node_clock) = self.actor_parts(node_id)?;
        let controller = self.controller(node_id)?;
        let factory = LabLatencyTransportFactory::new_dynamic(
            VirtualTransportFactory::new(self.network.clone()),
            Arc::clone(&self.shared_clock),
            controller,
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
        let controller = self.controller(node_id)?;
        let factory = LabLatencyTransportFactory::new_dynamic(
            VirtualTransportFactory::new(self.network.clone()),
            Arc::clone(&self.shared_clock),
            controller,
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
}
