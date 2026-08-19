//! `DesktopHostNetworkControl`'s binding lifecycle: construction, interface
//! bind/stop/shutdown, `Drop` safety, and the small monitor-delegation
//! methods. Playback stream control lives in [`super::playback_control`];
//! DTO translation lives in [`super::dto_bridge`].

use super::bind_selection::{
    BindPreference, SelectedAddress, parse_preference, select_address, validate_selected,
};
use super::dto_bridge::{monitor_status_dto, snapshot_from};
use super::interfaces::NetdevNetworkInterfaceProvider;
use super::{DesktopNetworkError, NetworkInterfaceProvider, StreamDiagnostics};
use crate::dto::DesktopErrorDto;
use crate::platform::audio_device::{
    AudioOutputBackend, CpalAudioOutputBackend, NullAudioOutputBackend,
};
use crate::platform::failure::DesktopPlatformFailure;
use crate::platform::host_transport::{ActiveHostSessionSnapshot, DesktopHostTransportRuntime};
use crate::platform::host_transport_events::DesktopHostTransportEventSink;
use crate::platform::mdns::{
    MdnsPublicationState, MdnsPublisher, MdnsSdPublisher, NullMdnsPublisher,
};
use crate::platform::monitor::{DesktopMonitorControl, MonitorStatus};
use crate::platform::network_dto::{
    MonitorStatusDto, NetworkInterfaceSnapshotDto, SetNetworkBindPreferenceRequest,
};
use crate::platform::playback_streamer::DesktopPlaybackStreamer;
use silent_disco_core::error::CoreError;
use silent_disco_core::runtime::{
    CoreActorHandle, NetworkEndpoint, SessionAdvertisement, TransportEffect,
};
use silent_disco_core::transport::{
    HostTransportConfig, SystemTransportClock, TransportClock, TransportFactory,
    production_transport_factory,
};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::platform) struct HostPorts {
    pub(in crate::platform) control: u16,
    pub(in crate::platform) sync: u16,
    pub(in crate::platform) audio: u16,
}

pub(super) struct ActiveBinding {
    pub(super) selected: SelectedAddress,
    pub(super) advertisement: SessionAdvertisement,
    pub(super) runtime: DesktopHostTransportRuntime,
    pub(super) playback: Option<DesktopPlaybackStreamer>,
    /// Live decode/packetizer queue-diagnostics readers for the current
    /// stream (Block 35.1 "decoder/source queues"/"packetizer"), taken
    /// before ownership of the underlying handles passes to the packetizer
    /// worker and the playback pump thread respectively. `None` whenever
    /// no stream has ever started for this binding.
    pub(super) stream_diagnostics: Option<StreamDiagnostics>,
    pub(super) mdns: MdnsPublicationState,
}

pub(super) struct NetworkState {
    preference: BindPreference,
    pub(super) active: Option<ActiveBinding>,
}

pub(crate) struct DesktopHostNetworkControl {
    provider: Arc<dyn NetworkInterfaceProvider>,
    transport_factory: Arc<dyn TransportFactory>,
    ports: HostPorts,
    mdns: Arc<dyn MdnsPublisher>,
    pub(in crate::platform) monitor: Arc<DesktopMonitorControl>,
    pub(super) state: Mutex<NetworkState>,
}

impl DesktopHostNetworkControl {
    #[must_use]
    pub(crate) fn production() -> Self {
        Self::with_components(
            Arc::new(NetdevNetworkInterfaceProvider),
            Arc::new(production_transport_factory()),
            HostPorts::default(),
        )
        .with_mdns_publisher(Arc::new(MdnsSdPublisher::new()))
        .with_monitor_backend(Arc::new(CpalAudioOutputBackend))
    }

    pub(in crate::platform) fn with_components(
        provider: Arc<dyn NetworkInterfaceProvider>,
        transport_factory: Arc<dyn TransportFactory>,
        ports: HostPorts,
    ) -> Self {
        Self {
            provider,
            transport_factory,
            ports,
            mdns: Arc::new(NullMdnsPublisher),
            monitor: DesktopMonitorControl::new(Arc::new(NullAudioOutputBackend)),
            state: Mutex::new(NetworkState {
                preference: BindPreference::Automatic,
                active: None,
            }),
        }
    }

    /// Replaces this control's mDNS publisher. Consuming/returning `Self`
    /// keeps `production()` a one-expression builder chain rather than a
    /// mutable local; test callers that want a custom fake use
    /// [`Self::with_components`] then this, same pattern.
    #[must_use]
    pub(in crate::platform) fn with_mdns_publisher(mut self, mdns: Arc<dyn MdnsPublisher>) -> Self {
        self.mdns = mdns;
        self
    }

    /// Replaces this control's local monitor audio output backend, same
    /// builder pattern as [`Self::with_mdns_publisher`].
    #[must_use]
    pub(super) fn with_monitor_backend(mut self, backend: Arc<dyn AudioOutputBackend>) -> Self {
        self.monitor = DesktopMonitorControl::new(backend);
        self
    }

    /// Sets the desktop host's local-monitor preference (34.2 "monitor
    /// enable is explicit"). Disabling takes effect immediately; enabling
    /// takes effect on the next stream start -- see `monitor.rs`'s module
    /// doc comment for why.
    pub(crate) fn set_monitor_enabled(&self, enabled: bool) -> Result<(), DesktopNetworkError> {
        self.monitor
            .set_enabled(enabled)
            .map_err(DesktopNetworkError::unavailable)
    }

    /// Current desktop monitor status, safe to surface as-is.
    #[must_use]
    pub(crate) fn monitor_status(&self) -> MonitorStatusDto {
        monitor_status_dto(&self.monitor.status())
    }

    /// Full monitor status including live render-callback telemetry
    /// (Block 35.1 "local monitor and render counters") -- unlike
    /// [`Self::monitor_status`], which returns the lean, frequently-polled
    /// [`MonitorStatusDto`] used by the host-session snapshot.
    #[must_use]
    pub(crate) fn monitor_status_full(&self) -> MonitorStatus {
        self.monitor.status()
    }

    /// Returns a bounded, classified interface snapshot and detects changes to an active bind.
    pub(crate) fn snapshot(&self) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let interfaces = self
            .provider
            .interfaces()
            .map_err(DesktopNetworkError::dto)?;
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        Ok(snapshot_from(
            &interfaces,
            &state.preference,
            state.active.as_ref(),
        ))
    }

    /// Replaces the bind preference after validating it against the current interface snapshot.
    pub(crate) fn set_preference(
        &self,
        request: &SetNetworkBindPreferenceRequest,
    ) -> Result<NetworkInterfaceSnapshotDto, DesktopErrorDto> {
        let interfaces = self
            .provider
            .interfaces()
            .map_err(DesktopNetworkError::dto)?;
        let preference = parse_preference(request).map_err(DesktopNetworkError::dto)?;
        if let BindPreference::Explicit { .. } = &preference {
            select_address(&interfaces, &preference).map_err(DesktopNetworkError::dto)?;
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        if state.active.is_some() {
            return Err(DesktopNetworkError::invalid_state(
                "network bind preference cannot change while a host endpoint is active",
            )
            .dto());
        }
        state.preference = preference;
        Ok(snapshot_from(&interfaces, &state.preference, None))
    }

    pub(in crate::platform) fn start_host(
        &self,
        advertisement: &SessionAdvertisement,
        handle: CoreActorHandle,
    ) -> Result<NetworkEndpoint, DesktopPlatformFailure> {
        self.start_host_with_sink(advertisement, Arc::new(handle))
            .map_err(|error| error.platform_failure())
    }

    #[cfg(test)]
    pub(in crate::platform) fn start_host_inner(
        &self,
        advertisement: &SessionAdvertisement,
    ) -> Result<NetworkEndpoint, DesktopNetworkError> {
        self.start_host_with_sink(
            advertisement,
            Arc::new(crate::platform::host_transport_events::TestTransportEventSink),
        )
    }

    fn start_host_with_sink(
        &self,
        advertisement: &SessionAdvertisement,
        sink: Arc<dyn DesktopHostTransportEventSink>,
    ) -> Result<NetworkEndpoint, DesktopNetworkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned())?;
        if state.active.is_some() {
            return Err(DesktopNetworkError::invalid_state(
                "desktop host network endpoint is already active",
            ));
        }
        let initial = self.provider.interfaces()?;
        let selected = select_address(&initial, &state.preference)?;
        let refreshed = self.provider.interfaces()?;
        validate_selected(&refreshed, &selected)?;

        let mut config = HostTransportConfig::loopback(advertisement.session_id.clone());
        config.bind_address = IpAddr::V4(selected.address);
        config.control_port = self.ports.control;
        config.sync_port = self.ports.sync;
        config.audio_port = self.ports.audio;
        let clock: Arc<dyn TransportClock> = Arc::new(SystemTransportClock::default());
        let node = self
            .transport_factory
            .bind_host(config, Arc::clone(&clock))
            .map_err(|error| DesktopNetworkError::transport(&error))?;
        let endpoint = node.endpoint();
        if endpoint.address != IpAddr::V4(selected.address) {
            let mut node = node;
            let cleanup = node.shutdown().err();
            return Err(DesktopNetworkError::endpoint_mismatch(cleanup.as_ref()));
        }
        let runtime = DesktopHostTransportRuntime::start(node, advertisement.clone(), sink, clock)?;
        // Publish only now that a real, already-bound endpoint exists
        // (30.2) -- `endpoint` above is what the transport actually
        // bound to, not a value computed ahead of the bind succeeding. A
        // publish failure is recorded, not propagated: the manual
        // connection payload stays fully functional regardless of mDNS's
        // fate (30.2 "retain manual endpoint as visibly available
        // alternative").
        let mdns = match self.mdns.publish(advertisement, endpoint) {
            Ok(registration) => MdnsPublicationState::Active(registration),
            Err(error) => MdnsPublicationState::Failed(error),
        };
        state.active = Some(ActiveBinding {
            selected,
            advertisement: advertisement.clone(),
            runtime,
            playback: None,
            stream_diagnostics: None,
            mdns,
        });
        Ok(endpoint)
    }

    pub(in crate::platform) fn stop_host(&self) -> Result<(), DesktopPlatformFailure> {
        self.stop_host_inner()
            .map_err(|error| error.platform_failure())
    }

    pub(in crate::platform) fn stop_host_inner(&self) -> Result<(), DesktopNetworkError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned())?;
        let Some(mut active) = state.active.take() else {
            return Err(DesktopNetworkError::invalid_state(
                "desktop host network endpoint is not active",
            ));
        };
        // Every shutdown step is attempted even when an earlier one
        // failed, exactly like the playback/runtime steps below -- a
        // failing withdrawal must not prevent the rest of teardown, and
        // must not be reported as a clean stop either (30.2 "withdraw on
        // session end and shutdown").
        let mdns_result = active.mdns.withdraw().map_err(|error| {
            DesktopNetworkError::invalid_state(format!("mDNS withdrawal failed: {error}"))
        });
        // The transport runtime is shut down even when the pump failed to stop
        // cleanly -- leaving it running would leak a bound socket -- but a
        // failing pump must not be reported as a clean host shutdown.
        let playback_result = match active.playback.take() {
            Some(playback) => {
                playback.request_stop();
                playback.join()
            }
            None => Ok(()),
        };
        active.runtime.shutdown()?;
        playback_result
            .map_err(|error| {
                DesktopNetworkError::invalid_state(format!(
                    "host shut down, but its playback pump did not stop cleanly: {}",
                    error.message
                ))
            })
            .and(mdns_result)
    }

    pub(crate) fn active_host_session(
        &self,
    ) -> Result<Option<ActiveHostSessionSnapshot>, DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_ref() else {
            return Ok(None);
        };
        let status = active.runtime.status().map_err(DesktopNetworkError::dto)?;
        Ok(Some(ActiveHostSessionSnapshot {
            advertisement: active.advertisement.clone(),
            endpoint: active.runtime.endpoint(),
            worker_running: status.running,
            last_error: status.last_error,
            observed_at_ms: active.runtime.observed_at().get(),
            broadcast: status.broadcast,
        }))
    }

    pub(crate) fn dispatch_transport_effect(
        &self,
        effect: TransportEffect,
    ) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().core_error(Some(operation_id.clone())))?;
        let Some(active) = state.active.as_ref() else {
            return Err(DesktopNetworkError::unavailable(
                "transport effect requires an active desktop host session",
            )
            .core_error(Some(operation_id)));
        };
        active.runtime.dispatch(effect)
    }

    pub(crate) fn shutdown(&self) -> Result<(), CoreError> {
        let active = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().core_error(None))?
            .active
            .is_some();
        let stop_result = if active {
            self.stop_host_inner()
        } else {
            Ok(())
        };
        // The daemon-level publisher is shut down unconditionally, even
        // when no binding was ever active -- distinct from the
        // per-publication `withdraw()` `stop_host_inner` already covers
        // above. A clean no-op when the daemon was never lazily created
        // (Block 36.2 mDNS daemon shutdown).
        let daemon_result = self.mdns.shutdown().map_err(|error| {
            DesktopNetworkError::invalid_state(format!("mDNS daemon shutdown failed: {error}"))
        });
        stop_result
            .and(daemon_result)
            .map_err(|error| error.core_error(None))
    }
}

impl Drop for DesktopHostNetworkControl {
    fn drop(&mut self) {
        let active = self
            .state
            .get_mut()
            .map_or(true, |state| state.active.is_some());
        assert!(
            !active || std::thread::panicking(),
            "DesktopHostNetworkControl dropped with an active transport"
        );
    }
}
