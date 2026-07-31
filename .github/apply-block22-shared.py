from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"anchor count {count} for {path}: {old[:120]!r}")
    target.write_text(source.replace(old, new, 1))


# 1. Make the pending-control path explicit at the shared transport boundary.
replace_once(
    "rust/silent-disco-core/src/transport/boundary.rs",
    """    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError>;\n    fn send_control(\n""",
    """    fn disconnect_peer(&self, device_id: &DeviceId) -> Result<(), TransportError>;\n    /// Sends one control message to an identified TCP peer before datagram authorization.\n    ///\n    /// This is intentionally narrower than `send_control`: it supports the pre-approval\n    /// `Hello` exchange without granting synchronization or audio routes.\n    fn send_pending_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError>;\n    fn send_control(\n""",
)

# 2. Socket host lookup and targeted delivery for an identified pending TCP peer.
replace_once(
    "rust/silent-disco-core/src/transport/socket/host.rs",
    """    fn peer_for_device(&self, device_id: &DeviceId) -> Result<Arc<PeerState>, TransportError> {\n        let routes = self.routes.lock().map_err(|_| {\n""",
    """    fn pending_peer_for_device(\n        &self,\n        device_id: &DeviceId,\n    ) -> Result<Arc<PeerState>, TransportError> {\n        let peers = self.peers.lock().map_err(|_| {\n            TransportError::new(\n                TransportErrorKind::WorkerPanicked,\n                TransportChannel::Runtime,\n                \"peer registry is poisoned\",\n            )\n        })?;\n        peers\n            .values()\n            .find(|peer| {\n                peer.active.load(Ordering::Acquire)\n                    && peer.device_id().as_ref() == Some(device_id)\n            })\n            .cloned()\n            .ok_or_else(|| {\n                TransportError::new(\n                    TransportErrorKind::PeerNotFound,\n                    TransportChannel::Control,\n                    \"identified pending control peer is not connected\",\n                )\n            })\n    }\n\n    fn peer_for_device(&self, device_id: &DeviceId) -> Result<Arc<PeerState>, TransportError> {\n        let routes = self.routes.lock().map_err(|_| {\n""",
)
replace_once(
    "rust/silent-disco-core/src/transport/socket/host.rs",
    """    fn send_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
    """    fn send_pending_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        if message.session_id() != &self.session_id {\n            return Err(TransportError::new(\n                TransportErrorKind::Unauthorized,\n                TransportChannel::Control,\n                \"outbound pending control message belongs to a different session\",\n            ));\n        }\n        let peer = self.pending_peer_for_device(device_id)?;\n        let bytes = encode_frame(&ProtocolFrame::Control(message.clone()))\n            .map_err(|error| TransportError::protocol(TransportChannel::Control, &error))?;\n        let result = peer.sender.send(bytes);\n        self.record_peer_result(&peer, &result);\n        match result {\n            Ok(written) => {\n                TransportDelivery::new(1, 1, 0, u64::try_from(written).unwrap_or(u64::MAX))\n            }\n            Err(error) => {\n                self.counters.delivery_failure();\n                Err(error)\n            }\n        }\n    }\n\n    fn send_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
)

# 3. Virtual transport mirrors pending TCP control without granting authorization.
replace_once(
    "rust/silent-disco-core/src/transport/virtual_transport.rs",
    """    fn deliver_control(\n        &self,\n        target: Option<&DeviceId>,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
    """    fn deliver_control(\n        &self,\n        target: Option<&DeviceId>,\n        message: &ControlMessage,\n        authorized_only: bool,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
)
replace_once(
    "rust/silent-disco-core/src/transport/virtual_transport.rs",
    """            .filter(|(device_id, listener)| {\n                listener.authorized && target.is_none_or(|target| target == *device_id)\n            })\n""",
    """            .filter(|(device_id, listener)| {\n                (!authorized_only || listener.authorized)\n                    && target.is_none_or(|target| target == *device_id)\n            })\n""",
)
replace_once(
    "rust/silent-disco-core/src/transport/virtual_transport.rs",
    """                \"virtual authorized peer is not connected\",\n""",
    """                if authorized_only {\n                    \"virtual authorized peer is not connected\"\n                } else {\n                    \"virtual identified pending peer is not connected\"\n                },\n""",
)
replace_once(
    "rust/silent-disco-core/src/transport/virtual_transport.rs",
    """    fn send_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        self.deliver_control(Some(device_id), message)\n    }\n\n    fn broadcast_control(\n        &self,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        self.deliver_control(None, message)\n    }\n""",
    """    fn send_pending_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        self.deliver_control(Some(device_id), message, false)\n    }\n\n    fn send_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        self.deliver_control(Some(device_id), message, true)\n    }\n\n    fn broadcast_control(\n        &self,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        self.deliver_control(None, message, true)\n    }\n""",
)

# 4. Fault wrapper delegates the new non-UDP operation unchanged.
replace_once(
    "rust/silent-disco-core/src/transport/virtual_fault.rs",
    """    fn send_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
    """    fn send_pending_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        self.inner.send_pending_control(device_id, message)\n    }\n\n    fn send_control(\n        &self,\n        device_id: &DeviceId,\n        message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
)

# 5. Desktop Block 21 fake remains exhaustive over the shared trait.
replace_once(
    "desktop/src-tauri/src/platform/network_tests.rs",
    """    fn send_control(\n        &self,\n        _device_id: &DeviceId,\n        _message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
    """    fn send_pending_control(\n        &self,\n        _device_id: &DeviceId,\n        _message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n        panic!(\"unused fake host operation\")\n    }\n\n    fn send_control(\n        &self,\n        _device_id: &DeviceId,\n        _message: &ControlMessage,\n    ) -> Result<TransportDelivery, TransportError> {\n""",
)

# 6. A disconnect before approval removes the authoritative pending request.
replace_once(
    "rust/silent-disco-core/src/runtime/actor_runtime/state/transport.rs",
    """        let previous = self.snapshot.listeners.len();\n        self.snapshot\n            .listeners\n            .retain(|listener| &listener.device_id != device_id);\n        if previous != self.snapshot.listeners.len()\n            && self.snapshot.listeners.is_empty()\n            && self.snapshot.host_lifecycle == HostLifecycle::Ready\n        {\n            self.snapshot.host_lifecycle = HostLifecycle::WaitingForListeners;\n        }\n        if let Some(error) = error {\n            self.snapshot.last_error = Some(error.clone());\n            return Ok(ApplyOutcome {\n                notifications: vec![CoreNotification::Error(error)],\n                changed: true,\n                stop_requested: false,\n            });\n        }\n        if previous == self.snapshot.listeners.len() {\n            return Ok(ApplyOutcome::default());\n        }\n        Ok(ApplyOutcome::changed())\n""",
    """        let previous_listeners = self.snapshot.listeners.len();\n        let previous_requests = self.snapshot.pending_join_requests.len();\n        self.snapshot\n            .listeners\n            .retain(|listener| &listener.device_id != device_id);\n        self.snapshot\n            .pending_join_requests\n            .retain(|request| &request.device_id != device_id);\n        let listener_removed = previous_listeners != self.snapshot.listeners.len();\n        let request_removed = previous_requests != self.snapshot.pending_join_requests.len();\n        if listener_removed\n            && self.snapshot.listeners.is_empty()\n            && self.snapshot.host_lifecycle == HostLifecycle::Ready\n        {\n            self.snapshot.host_lifecycle = HostLifecycle::WaitingForListeners;\n        }\n        if let Some(error) = error {\n            self.snapshot.last_error = Some(error.clone());\n            return Ok(ApplyOutcome {\n                notifications: vec![CoreNotification::Error(error)],\n                changed: true,\n                stop_requested: false,\n            });\n        }\n        if !listener_removed && !request_removed {\n            return Ok(ApplyOutcome::default());\n        }\n        Ok(ApplyOutcome::changed())\n""",
)

# 7. Focused socket proof: Hello is delivered before UDP authorization, with no audio claim.
transport_tests = Path("rust/silent-disco-core/src/transport/tests.rs")
source = transport_tests.read_text()
anchor = """#[test]\nfn socket_runtime_completes_multi_listener_join_sync_and_audio_exchange() {\n"""
if source.count(anchor) != 1:
    raise SystemExit("transport test insertion anchor not found exactly once")
new_test = """#[test]\nfn pending_control_peer_receives_hello_before_datagram_authorization() {\n    let session_id = id_session(\"manual-endpoint-session\");\n    let device_id = id_device(\"manual-endpoint-listener\");\n    let factory = SocketTransportFactory;\n    let clock = Arc::new(SystemTransportClock::default());\n    let mut host = factory\n        .bind_host(\n            HostTransportConfig::loopback(session_id.clone()),\n            clock.clone(),\n        )\n        .expect(\"manual endpoint host should bind\");\n    let mut listener = factory\n        .connect_listener(\n            ListenerTransportConfig::loopback(\n                session_id.clone(),\n                device_id.clone(),\n                host.endpoint(),\n            ),\n            clock,\n        )\n        .expect(\"manual endpoint listener should connect\");\n\n    listener\n        .send_control(&join_request(\n            &session_id,\n            &device_id,\n            \"Manual Endpoint Listener\",\n        ))\n        .expect(\"join request should reach the host\");\n    wait_for_control_from(&mut *host, &device_id, |message| {\n        matches!(message, ControlMessage::JoinRequest(_))\n    });\n\n    let hello = ControlMessage::Hello(Hello {\n        session_id: session_id.clone(),\n        session_name: \"Manual Endpoint Session\".to_owned(),\n        host_name: \"Desktop Host\".to_owned(),\n        approval_required: true,\n    });\n    let delivery = host\n        .send_pending_control(&device_id, &hello)\n        .expect(\"identified pending peer should receive TCP Hello\");\n    assert_eq!(delivery.report.intended_peers, 1);\n    assert_eq!(delivery.report.successful_peers, 1);\n    wait_for_frame(&mut *listener, TransportChannel::Control, |frame| {\n        frame == &ProtocolFrame::Control(hello.clone())\n    });\n\n    assert_eq!(host.counters().audio_datagrams_sent, 0);\n    assert_eq!(listener.counters().audio_datagrams_received, 0);\n    listener.shutdown().expect(\"listener should stop\");\n    host.shutdown().expect(\"host should stop\");\n}\n\n"""
transport_tests.write_text(source.replace(anchor, new_test + anchor, 1))
