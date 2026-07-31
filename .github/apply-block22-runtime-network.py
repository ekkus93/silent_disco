from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    source = target.read_text()
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"integration anchor count {count} for {path}: {old[:140]!r}")
    target.write_text(source.replace(old, new, 1))


replace_once(
    "desktop/src-tauri/src/platform/mod.rs",
    "pub mod identity;\npub mod network;\n",
    "pub mod identity;\nmod host_join_projection;\nmod host_pending_handshake;\npub(crate) mod host_transport;\nmod host_transport_events;\npub mod network;\n",
)
replace_once(
    "desktop/src-tauri/src/platform/mod.rs",
    "#[cfg(test)]\nmod network_tests;\n",
    "#[cfg(test)]\nmod network_tests;\n#[cfg(test)]\nmod host_transport_tests;\n",
)
replace_once(
    "desktop/src-tauri/src/lib.rs",
    "mod host_commands;\n",
    "mod host_commands;\npub mod host_session_dto;\n",
)

replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "use super::failure::DesktopPlatformFailure;\n",
    "use super::failure::DesktopPlatformFailure;\nuse super::host_transport::{ActiveHostSessionSnapshot, DesktopHostTransportRuntime};\nuse super::host_transport_events::DesktopHostTransportEventSink;\n",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "use silent_disco_core::runtime::{NetworkEndpoint, SessionAdvertisement};\n",
    "use silent_disco_core::runtime::{CoreActorHandle, NetworkEndpoint, SessionAdvertisement};\n",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "    HostTransportConfig, HostTransportNode, SystemTransportClock, TransportFactory,\n",
    "    HostTransportConfig, SystemTransportClock, TransportFactory,\n",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "struct ActiveBinding {\n    selected: SelectedAddress,\n    endpoint: NetworkEndpoint,\n    node: Box<dyn HostTransportNode>,\n}\n",
    "struct ActiveBinding {\n    selected: SelectedAddress,\n    runtime: DesktopHostTransportRuntime,\n}\n",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    """    pub(super) fn start_host(
        &self,
        advertisement: &SessionAdvertisement,
    ) -> Result<NetworkEndpoint, DesktopPlatformFailure> {
        self.start_host_inner(advertisement)
            .map_err(|error| error.platform_failure())
    }

    pub(super) fn start_host_inner(
        &self,
        advertisement: &SessionAdvertisement,
    ) -> Result<NetworkEndpoint, DesktopNetworkError> {
""",
    """    pub(super) fn start_host(
        &self,
        advertisement: &SessionAdvertisement,
        handle: CoreActorHandle,
    ) -> Result<NetworkEndpoint, DesktopPlatformFailure> {
        self.start_host_with_sink(advertisement, Arc::new(handle))
            .map_err(|error| error.platform_failure())
    }

    #[cfg(test)]
    pub(super) fn start_host_inner(
        &self,
        advertisement: &SessionAdvertisement,
    ) -> Result<NetworkEndpoint, DesktopNetworkError> {
        self.start_host_with_sink(
            advertisement,
            Arc::new(super::host_transport_events::TestTransportEventSink),
        )
    }

    fn start_host_with_sink(
        &self,
        advertisement: &SessionAdvertisement,
        sink: Arc<dyn DesktopHostTransportEventSink>,
    ) -> Result<NetworkEndpoint, DesktopNetworkError> {
""",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    """        state.active = Some(ActiveBinding {
            selected,
            endpoint,
            node,
        });
""",
    """        let runtime = DesktopHostTransportRuntime::start(node, advertisement.clone(), sink)?;
        state.active = Some(ActiveBinding { selected, runtime });
""",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    "        let Some(mut active) = state.active.take() else {\n",
    "        let Some(active) = state.active.take() else {\n",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    """        active
            .node
            .shutdown()
            .map_err(|error| DesktopNetworkError::transport(&error))
    }

    pub(crate) fn shutdown(&self) -> Result<(), CoreError> {
""",
    """        active.runtime.shutdown()
    }

    pub(crate) fn active_host_session(
        &self,
    ) -> Result<Option<ActiveHostSessionSnapshot>, DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        state
            .active
            .as_ref()
            .map(|active| active.runtime.snapshot().map_err(DesktopNetworkError::dto))
            .transpose()
    }

    pub(crate) fn shutdown(&self) -> Result<(), CoreError> {
""",
)
replace_once(
    "desktop/src-tauri/src/platform/network.rs",
    """        address: binding.endpoint.address.to_string(),
        control_port: binding.endpoint.control_port,
        sync_port: binding.endpoint.sync_port,
        audio_port: binding.endpoint.audio_port,
""",
    """        address: binding.runtime.endpoint().address.to_string(),
        control_port: binding.runtime.endpoint().control_port,
        sync_port: binding.runtime.endpoint().sync_port,
        audio_port: binding.runtime.endpoint().audio_port,
""",
)
replace_once(
    "desktop/src-tauri/src/platform/network_tests.rs",
    """    fn recv_event(&mut self, _timeout: Duration) -> Result<TransportEvent, TransportError> {
        panic!("unused fake host operation")
    }
""",
    """    fn recv_event(&mut self, _timeout: Duration) -> Result<TransportEvent, TransportError> {
        Err(TransportError::new(
            silent_disco_core::transport::TransportErrorKind::Timeout,
            silent_disco_core::transport::TransportChannel::Runtime,
            "fake host receive timed out",
        ))
    }
""",
)

replace_once(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    """            Arc::new(DesktopPlatformAdapters::new_with_network(
                paths,
                Arc::clone(&network),
            )),
""",
    """            Arc::new(DesktopPlatformAdapters::new_with_network(
                paths,
                Arc::clone(&network),
                capability_handle.clone(),
            )),
""",
)
replace_once(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    "    network: Arc<DesktopHostNetworkControl>,\n}\n",
    "    network: Arc<DesktopHostNetworkControl>,\n    transport_events: Option<CoreActorHandle>,\n}\n",
)
replace_once(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    """    #[cfg(test)]
    pub(super) fn new(paths: DesktopProfilePaths) -> Self {
        Self::new_with_network(paths, Arc::new(DesktopHostNetworkControl::production()))
    }

    pub(super) fn new_with_network(
        paths: DesktopProfilePaths,
        network: Arc<DesktopHostNetworkControl>,
    ) -> Self {
        Self {
            paths,
            capabilities: desktop_capabilities(),
            network,
        }
    }
""",
    """    #[cfg(test)]
    pub(super) fn new(paths: DesktopProfilePaths) -> Self {
        Self {
            paths,
            capabilities: desktop_capabilities(),
            network: Arc::new(DesktopHostNetworkControl::production()),
            transport_events: None,
        }
    }

    pub(super) fn new_with_network(
        paths: DesktopProfilePaths,
        network: Arc<DesktopHostNetworkControl>,
        transport_events: CoreActorHandle,
    ) -> Self {
        Self {
            paths,
            capabilities: desktop_capabilities(),
            network,
            transport_events: Some(transport_events),
        }
    }
""",
)
replace_once(
    "desktop/src-tauri/src/platform/effect_runner.rs",
    "                .start_host(advertisement)\n",
    """                .start_host(
                    advertisement,
                    self.transport_events.clone().ok_or_else(|| {
                        DesktopPlatformFailure::new(
                            CoreErrorCode::WorkerStopped,
                            "desktop host transport event sink is unavailable",
                            ErrorSeverity::Error,
                            true,
                        )
                    })?,
                )
""",
)
