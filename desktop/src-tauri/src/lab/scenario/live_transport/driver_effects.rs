impl LiveTransportDriver {
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
            RuntimeTransportEvent::PeerDisconnected { error: None, .. } => Err(live_error(
                "listener_peer_disconnected",
                "listener connection closed without a transport error",
            )),
            RuntimeTransportEvent::PeerAccepted { .. }
            | RuntimeTransportEvent::PeerAuthorized { .. } => Ok(()),
            RuntimeTransportEvent::FrameReceived { .. } => Err(live_error(
                "listener_frame_unsupported",
                "Lab listener received a transport frame outside the supported join/synchronization subset",
            )),
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
}
