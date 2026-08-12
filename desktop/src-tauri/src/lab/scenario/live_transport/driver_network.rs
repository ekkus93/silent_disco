impl LiveTransportDriver {
    fn start_advertising(
        &mut self,
        node_id: &NodeId,
        operation_id: OperationId,
        advertisement: &mut SessionAdvertisement,
    ) -> Result<(), DesktopErrorDto> {
        let profile = self.profile(node_id)?;
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
