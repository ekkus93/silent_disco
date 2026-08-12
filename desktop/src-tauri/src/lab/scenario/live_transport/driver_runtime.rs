impl LiveTransportDriver {
    pub(super) fn new(
        lab: &LabRuntime,
        scenario: &Scenario,
        lab_node_ids: &HashMap<&str, LabNodeId>,
        mut effect_receivers: HashMap<NodeId, Receiver<CoreNotification>>,
    ) -> Result<Self, DesktopErrorDto> {
        let profiles = build_receive_profiles(scenario)?;
        let transport_trace = TransportTraceRecorder::new();
        let fault_controllers = build_fault_controllers(&profiles, &transport_trace);
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
            transport_trace,
            actors,
            hosts: HashMap::new(),
            listeners: HashMap::new(),
        })
    }

    pub(super) fn transport_trace(&self) -> Result<TransportTrace, TransportTraceError> {
        self.transport_trace.snapshot()
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
}
