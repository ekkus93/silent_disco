use super::audio_device::{CpalAudioOutputBackend, NullAudioOutputBackend};
use super::failure::DesktopPlatformFailure;
use super::host_transport::{ActiveHostSessionSnapshot, DesktopHostTransportRuntime};
use super::host_transport_events::DesktopHostTransportEventSink;
use super::mdns::{MdnsPublicationState, MdnsPublisher, MdnsSdPublisher, NullMdnsPublisher};
use super::monitor::{DesktopMonitorControl, MonitorStatus};
use super::network_dto::{
    MdnsStatusDto, MonitorStatusDto, NetworkAddressCandidateDto, NetworkAddressClassDto,
    NetworkBindPreferenceDto, NetworkBindingDto, NetworkInterfaceSnapshotDto,
    SetNetworkBindPreferenceRequest,
};
pub(super) use super::network_error::{DesktopNetworkError, NetworkErrorKind};
use super::playback_streamer::DesktopPlaybackStreamer;
use crate::dto::DesktopErrorDto;
use netdev::Interface;
use silent_disco_core::domain::{MonotonicMillis, PlaybackState};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{ControlMessage, Pause, ProtocolFrame, StreamStart};
use silent_disco_core::runtime::{
    AudioEvent, CoreActorHandle, NetworkEndpoint, SessionAdvertisement, TransportEffect,
};
use silent_disco_core::transport::{
    HostTransportConfig, SystemTransportClock, TransportClock, TransportFactory,
    production_transport_factory,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

const MAX_INTERFACE_RECORDS: usize = 256;
const MAX_ADDRESS_RECORDS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct InterfaceRecord {
    pub name: String,
    pub index: u32,
    pub up: bool,
    pub running: bool,
    pub oper_up: bool,
    pub loopback: bool,
    pub point_to_point: bool,
    pub tun: bool,
    pub physical: bool,
    pub default_route: bool,
    pub addresses: Vec<AddressRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AddressRecord {
    pub address: IpAddr,
    pub prefix_length: u8,
}

pub(super) trait NetworkInterfaceProvider: Send + Sync + 'static {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, DesktopNetworkError>;
}

#[derive(Debug, Default)]
struct NetdevNetworkInterfaceProvider;

impl NetworkInterfaceProvider for NetdevNetworkInterfaceProvider {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, DesktopNetworkError> {
        normalize_interfaces(netdev::get_interfaces())
    }
}

fn normalize_interfaces(
    interfaces: Vec<Interface>,
) -> Result<Vec<InterfaceRecord>, DesktopNetworkError> {
    if interfaces.len() > MAX_INTERFACE_RECORDS {
        return Err(DesktopNetworkError::resource_limit(
            "desktop network interface count exceeds the supported limit",
        ));
    }
    let mut address_count = 0usize;
    interfaces
        .into_iter()
        .map(|interface| {
            let up = interface.is_up();
            let running = interface.is_running();
            let oper_up = interface.is_oper_up();
            let loopback = interface.is_loopback();
            let point_to_point = interface.is_point_to_point();
            let tun = interface.is_tun();
            let physical = interface.is_physical();
            let default_route = interface.default;
            let index = interface.index;
            let name = interface.name.clone();
            let mut addresses = Vec::with_capacity(interface.ipv4.len() + interface.ipv6.len());
            for network in &interface.ipv4 {
                addresses.push(AddressRecord {
                    address: IpAddr::V4(network.addr()),
                    prefix_length: network.prefix_len(),
                });
            }
            for network in &interface.ipv6 {
                addresses.push(AddressRecord {
                    address: IpAddr::V6(network.addr()),
                    prefix_length: network.prefix_len(),
                });
            }
            address_count = address_count.saturating_add(addresses.len());
            if address_count > MAX_ADDRESS_RECORDS {
                return Err(DesktopNetworkError::resource_limit(
                    "desktop network address count exceeds the supported limit",
                ));
            }
            Ok(InterfaceRecord {
                name,
                index,
                up,
                running,
                oper_up,
                loopback,
                point_to_point,
                tun,
                physical,
                default_route,
                addresses,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BindPreference {
    Automatic,
    Explicit {
        interface_name: String,
        address: Ipv4Addr,
    },
}

impl BindPreference {
    fn dto(&self) -> NetworkBindPreferenceDto {
        match self {
            Self::Automatic => NetworkBindPreferenceDto {
                mode: "automatic".to_owned(),
                interface_name: None,
                address: None,
            },
            Self::Explicit {
                interface_name,
                address,
            } => NetworkBindPreferenceDto {
                mode: "explicit".to_owned(),
                interface_name: Some(interface_name.clone()),
                address: Some(address.to_string()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedAddress {
    interface_name: String,
    interface_index: u32,
    address: Ipv4Addr,
    default_route: bool,
    physical: bool,
    prefix_length: u8,
}

impl SelectedAddress {
    fn dto(&self) -> NetworkAddressCandidateDto {
        NetworkAddressCandidateDto {
            interface_name: self.interface_name.clone(),
            interface_index: self.interface_index,
            address: self.address.to_string(),
            prefix_length: self.prefix_length,
            classification: NetworkAddressClassDto::PrivateLan,
            is_default_route: self.default_route,
            is_active: true,
            is_physical: self.physical,
            selectable: true,
            rejection_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct HostPorts {
    pub(super) control: u16,
    pub(super) sync: u16,
    pub(super) audio: u16,
}

struct ActiveBinding {
    selected: SelectedAddress,
    advertisement: SessionAdvertisement,
    runtime: DesktopHostTransportRuntime,
    playback: Option<DesktopPlaybackStreamer>,
    mdns: MdnsPublicationState,
}

struct NetworkState {
    preference: BindPreference,
    active: Option<ActiveBinding>,
}

pub(crate) struct DesktopHostNetworkControl {
    provider: Arc<dyn NetworkInterfaceProvider>,
    transport_factory: Arc<dyn TransportFactory>,
    ports: HostPorts,
    mdns: Arc<dyn MdnsPublisher>,
    pub(super) monitor: Arc<DesktopMonitorControl>,
    state: Mutex<NetworkState>,
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

    pub(super) fn with_components(
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
    pub(super) fn with_mdns_publisher(mut self, mdns: Arc<dyn MdnsPublisher>) -> Self {
        self.mdns = mdns;
        self
    }

    /// Replaces this control's local monitor audio output backend, same
    /// builder pattern as [`Self::with_mdns_publisher`].
    #[must_use]
    pub(super) fn with_monitor_backend(
        mut self,
        backend: Arc<dyn super::audio_device::AudioOutputBackend>,
    ) -> Self {
        self.monitor = DesktopMonitorControl::new(backend);
        self
    }

    /// Sets the desktop host's local-monitor preference (34.2 "monitor
    /// enable is explicit"). Disabling takes effect immediately; enabling
    /// takes effect on the next stream start -- see `monitor.rs`'s module
    /// doc comment for why.
    pub(crate) fn set_monitor_enabled(&self, enabled: bool) {
        self.monitor.set_enabled(enabled);
    }

    /// Current desktop monitor status, safe to surface as-is.
    #[must_use]
    pub(crate) fn monitor_status(&self) -> MonitorStatusDto {
        monitor_status_dto(&self.monitor.status())
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

    pub(super) fn start_host(
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
            mdns,
        });
        Ok(endpoint)
    }

    pub(super) fn stop_host(&self) -> Result<(), DesktopPlatformFailure> {
        self.stop_host_inner()
            .map_err(|error| error.platform_failure())
    }

    pub(super) fn stop_host_inner(&self) -> Result<(), DesktopNetworkError> {
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

    /// Resolves the current staged/decoded/packetized source into an active
    /// playback stream, transitioning the actor to `Playing` and starting
    /// the real-time broadcast pump. See [`DesktopPlaybackStreamer::start`].
    ///
    /// # Errors
    ///
    /// Returns a structured error when no host session is active, playback
    /// is already active and still running, or the actor rejects the
    /// `Playing` transition.
    pub(crate) fn start_playback(
        self: &Arc<Self>,
        packetizer: silent_disco_core::audio::StreamingPacketizeHandle,
        session_id: silent_disco_core::domain::SessionId,
        stream_id: silent_disco_core::domain::StreamId,
        handle: CoreActorHandle,
    ) -> Result<(), DesktopErrorDto> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| DesktopNetworkError::poisoned().dto())?;
            let Some(active) = state.active.as_mut() else {
                return Err(DesktopNetworkError::unavailable(
                    "starting playback requires an active desktop host session",
                )
                .dto());
            };
            match &active.playback {
                Some(playback) if !playback.is_finished() => {
                    return Err(DesktopNetworkError::invalid_state(
                        "playback is already active for this host session",
                    )
                    .dto());
                }
                _ => {
                    // A previous stream that ended by failing must surface here
                    // rather than being buried by starting the next one.
                    if let Some(finished) = active.playback.take() {
                        finished.join()?;
                    }
                }
            }
        }
        let streamer = DesktopPlaybackStreamer::start(
            packetizer,
            session_id,
            stream_id,
            Arc::clone(self),
            handle,
        )?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "desktop host session ended while playback was starting",
            )
            .dto());
        };
        active.playback = Some(streamer);
        Ok(())
    }

    /// Pauses the active playback stream after a validated actor transition.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no playback is active or the actor
    /// rejects the `Paused` transition (e.g. not currently playing).
    pub(crate) fn pause_playback(&self) -> Result<(), DesktopErrorDto> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "pausing playback requires an active desktop host session",
            )
            .dto());
        };
        let Some(playback) = active.playback.as_ref() else {
            return Err(DesktopNetworkError::invalid_state("no playback is active").dto());
        };
        playback
            .handle
            .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
            .map_err(DesktopErrorDto::from)?;
        let host_pause_time_ms = active.runtime.observed_at();
        active
            .runtime
            .broadcast_frame(ProtocolFrame::Control(ControlMessage::Pause(Pause {
                session_id: playback.session_id.clone(),
                stream_id: playback.stream_id.clone(),
                host_pause_time_ms,
            })))
            .map_err(DesktopNetworkError::dto)?;
        playback
            .paused_at_ms
            .store(host_pause_time_ms.get(), Ordering::Release);
        playback.paused.store(true, Ordering::Release);
        Ok(())
    }

    /// Resumes the active, paused playback stream after a validated actor
    /// transition, re-broadcasting `StreamStart` with a presentation anchor
    /// shifted forward by this pause's duration, so a listener that missed
    /// frames while paused re-anchors its own presentation-time expectations
    /// to match the same shift the pump now applies to every subsequent
    /// audio frame (see [`super::playback_streamer`]'s pause-offset
    /// accounting). Reusing the same `stream_id` lets the listener apply
    /// this in place rather than tearing down and reopening its audio
    /// engine.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no playback is active or the actor
    /// rejects the `Playing` transition (e.g. not currently paused).
    pub(crate) fn resume_playback(&self) -> Result<(), DesktopErrorDto> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "resuming playback requires an active desktop host session",
            )
            .dto());
        };
        let Some(playback) = active.playback.as_ref() else {
            return Err(DesktopNetworkError::invalid_state("no playback is active").dto());
        };
        playback
            .handle
            .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .map_err(DesktopErrorDto::from)?;

        // A resume issued while already playing is a stale/duplicate command
        // today's actor checks still accept -- it must stay a pure no-op
        // beyond the transition above. `paused_at_ms` is still its "never
        // paused" sentinel of zero in that case, so computing an offset from
        // it would fabricate a bogus multi-<x>-long "pause" out of nothing
        // and corrupt the anchor instead of fixing it; broadcasting here
        // would also contend with the pump's own real-time frames on the
        // same bounded queue for no reason.
        if !playback.paused.swap(false, Ordering::AcqRel) {
            return Ok(());
        }

        let now = active.runtime.observed_at();
        let paused_at_ms = playback.paused_at_ms.swap(0, Ordering::AcqRel);
        let elapsed_ms = now.get().saturating_sub(paused_at_ms);
        let total_offset_ms = playback
            .accumulated_pause_offset_ms
            .fetch_add(elapsed_ms, Ordering::AcqRel)
            .saturating_add(elapsed_ms);
        let reanchored_start = StreamStart {
            host_start_time_ms: MonotonicMillis::new(
                playback
                    .stream_start
                    .host_start_time_ms
                    .get()
                    .saturating_add(total_offset_ms),
            ),
            ..playback.stream_start.clone()
        };
        active
            .runtime
            .broadcast_frame(ProtocolFrame::Control(ControlMessage::StreamStart(
                reanchored_start,
            )))
            .map_err(DesktopNetworkError::dto)?;

        Ok(())
    }

    /// Signals the active playback stream to stop and blocks until its pump
    /// thread performs the `Stop` broadcast, the `Stopped` actor transition,
    /// and exits.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no playback is active.
    pub(crate) fn stop_playback(&self) -> Result<(), DesktopErrorDto> {
        let playback = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| DesktopNetworkError::poisoned().dto())?;
            let Some(active) = state.active.as_mut() else {
                return Err(DesktopNetworkError::unavailable(
                    "stopping playback requires an active desktop host session",
                )
                .dto());
            };
            active
                .playback
                .take()
                .ok_or_else(|| DesktopNetworkError::invalid_state("no playback is active").dto())?
        };
        playback.request_stop();
        playback.join()
    }

    /// Returns the transport worker's current monotonic time, the same
    /// clock basis used for sync responses -- callers computing a playback
    /// timestamp (e.g. `host_start_time_ms`) must use this, not a fresh
    /// clock, so presentation times remain comparable to sync samples.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no host session is active.
    pub(crate) fn transport_now(&self) -> Result<MonotonicMillis, DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_ref() else {
            return Err(DesktopNetworkError::unavailable(
                "desktop host network endpoint is not active",
            )
            .dto());
        };
        Ok(active.runtime.observed_at())
    }

    /// Enqueues one control/sync/audio frame for the host transport worker
    /// to broadcast. Used by the playback pump thread, which is never
    /// already holding this control's state lock.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no host session is active or the
    /// worker's broadcast queue is full/unavailable.
    pub(crate) fn broadcast_playback_frame(
        &self,
        frame: ProtocolFrame,
    ) -> Result<(), DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_ref() else {
            return Err(DesktopNetworkError::unavailable(
                "desktop host network endpoint is not active",
            )
            .dto());
        };
        active
            .runtime
            .broadcast_frame(frame)
            .map_err(DesktopNetworkError::dto)
    }

    /// Reports whether a playback stream is already running for the active
    /// host session, without mutating any actor or network state. Callers
    /// starting a new stream must consult this *before* submitting any
    /// actor transition (e.g. `Buffering`) -- otherwise a duplicate/stale
    /// Start command still visibly corrupts the authoritative snapshot with
    /// an `Error` state even though the real, already-running stream is
    /// untouched and the duplicate is correctly rejected underneath.
    pub(crate) fn playback_is_active(&self) -> Result<bool, DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        Ok(state
            .active
            .as_ref()
            .and_then(|active| active.playback.as_ref())
            .is_some_and(|playback| !playback.is_finished()))
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
        if !active {
            return Ok(());
        }
        self.stop_host_inner()
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

fn snapshot_from(
    interfaces: &[InterfaceRecord],
    preference: &BindPreference,
    active: Option<&ActiveBinding>,
) -> NetworkInterfaceSnapshotDto {
    let mut candidates = address_candidates(interfaces);
    candidates.sort_by(|left, right| {
        left.interface_name
            .cmp(&right.interface_name)
            .then(left.interface_index.cmp(&right.interface_index))
            .then(left.address.cmp(&right.address))
    });
    let automatic = select_address(interfaces, &BindPreference::Automatic);
    let (automatic_selection, requires_explicit_selection) = match &automatic {
        Ok(selected) => (Some(selected.dto()), false),
        Err(error) if error.kind == NetworkErrorKind::Ambiguous => (None, true),
        Err(_) => (None, false),
    };
    let resolved = select_address(interfaces, preference);
    let resolved_selection = resolved.as_ref().ok().map(SelectedAddress::dto);
    let selection_error = resolved.err().map(|error| error.message);
    let active_binding = active.map(|binding| NetworkBindingDto {
        interface_name: binding.selected.interface_name.clone(),
        address: binding.runtime.endpoint().address.to_string(),
        control_port: binding.runtime.endpoint().control_port,
        sync_port: binding.runtime.endpoint().sync_port,
        audio_port: binding.runtime.endpoint().audio_port,
        mdns: mdns_status_dto(&binding.mdns),
    });
    let active_binding_valid =
        active.is_none_or(|binding| validate_selected(interfaces, &binding.selected).is_ok());
    let interface_change = if active.is_some() && !active_binding_valid {
        Some("the active network interface or address is no longer available".to_owned())
    } else {
        None
    };
    NetworkInterfaceSnapshotDto {
        preference: preference.dto(),
        candidates,
        automatic_selection,
        resolved_selection,
        requires_explicit_selection,
        selection_error,
        active_binding,
        active_binding_valid,
        interface_change,
    }
}

fn mdns_status_dto(mdns: &MdnsPublicationState) -> MdnsStatusDto {
    match mdns {
        MdnsPublicationState::Active(_) => MdnsStatusDto {
            active: true,
            failure_reason: None,
        },
        MdnsPublicationState::Failed(error) => MdnsStatusDto {
            active: false,
            failure_reason: Some(error.to_string()),
        },
    }
}

fn monitor_status_dto(status: &MonitorStatus) -> MonitorStatusDto {
    MonitorStatusDto {
        enabled: status.enabled,
        active: status.active,
        failure_reason: status.failure_reason.clone(),
    }
}

fn address_candidates(interfaces: &[InterfaceRecord]) -> Vec<NetworkAddressCandidateDto> {
    interfaces
        .iter()
        .flat_map(|interface| {
            interface.addresses.iter().map(move |address| {
                let class = classify(interface, address.address);
                let active = is_active(interface);
                let selectable = active
                    && class == NetworkAddressClassDto::PrivateLan
                    && address.address.is_ipv4();
                let rejection_reason = if selectable {
                    None
                } else if !active {
                    Some("interface is not active".to_owned())
                } else if address.address.is_ipv6() {
                    Some(
                        "IPv6 host binding is not enabled in the initial desktop LAN baseline"
                            .to_owned(),
                    )
                } else {
                    Some(
                        match class {
                            NetworkAddressClassDto::Loopback => {
                                "loopback addresses are not advertised"
                            }
                            NetworkAddressClassDto::LinkLocal => {
                                "link-local addresses are not advertised"
                            }
                            NetworkAddressClassDto::Vpn => {
                                "VPN interfaces require a later explicit policy"
                            }
                            NetworkAddressClassDto::Container => {
                                "container interfaces are not advertised"
                            }
                            NetworkAddressClassDto::Other => "address is not a private LAN address",
                            NetworkAddressClassDto::PrivateLan => "address is not selectable",
                        }
                        .to_owned(),
                    )
                };
                NetworkAddressCandidateDto {
                    interface_name: interface.name.clone(),
                    interface_index: interface.index,
                    address: address.address.to_string(),
                    prefix_length: address.prefix_length,
                    classification: class,
                    is_default_route: interface.default_route,
                    is_active: active,
                    is_physical: interface.physical,
                    selectable,
                    rejection_reason,
                }
            })
        })
        .collect()
}

fn select_address(
    interfaces: &[InterfaceRecord],
    preference: &BindPreference,
) -> Result<SelectedAddress, DesktopNetworkError> {
    let mut selectable = interfaces
        .iter()
        .flat_map(|interface| {
            interface.addresses.iter().filter_map(move |address| {
                let IpAddr::V4(ipv4) = address.address else {
                    return None;
                };
                (is_active(interface)
                    && classify(interface, address.address) == NetworkAddressClassDto::PrivateLan)
                    .then(|| SelectedAddress {
                        interface_name: interface.name.clone(),
                        interface_index: interface.index,
                        address: ipv4,
                        default_route: interface.default_route,
                        physical: interface.physical,
                        prefix_length: address.prefix_length,
                    })
            })
        })
        .collect::<Vec<_>>();
    selectable.sort_by(|left, right| {
        right
            .default_route
            .cmp(&left.default_route)
            .then(left.interface_index.cmp(&right.interface_index))
            .then(left.interface_name.cmp(&right.interface_name))
            .then(left.address.octets().cmp(&right.address.octets()))
    });
    match preference {
        BindPreference::Explicit {
            interface_name,
            address,
        } => selectable
            .into_iter()
            .find(|candidate| {
                &candidate.interface_name == interface_name && &candidate.address == address
            })
            .ok_or_else(|| {
                DesktopNetworkError::unavailable(
                    "the requested private-LAN interface address is unavailable",
                )
            }),
        BindPreference::Automatic => match selectable.as_slice() {
            [] => Err(DesktopNetworkError::unavailable(
                "no active private-LAN IPv4 address is available for the desktop host",
            )),
            [single] => Ok(single.clone()),
            many => {
                let defaults = many
                    .iter()
                    .filter(|candidate| candidate.default_route)
                    .collect::<Vec<_>>();
                match defaults.as_slice() {
                    [single] => Ok((*single).clone()),
                    _ => Err(DesktopNetworkError::ambiguous(
                        "multiple private-LAN addresses are eligible; select one explicitly",
                    )),
                }
            }
        },
    }
}

fn validate_selected(
    interfaces: &[InterfaceRecord],
    selected: &SelectedAddress,
) -> Result<(), DesktopNetworkError> {
    let preference = BindPreference::Explicit {
        interface_name: selected.interface_name.clone(),
        address: selected.address,
    };
    select_address(interfaces, &preference).map(|_| ())
}

fn parse_preference(
    request: &SetNetworkBindPreferenceRequest,
) -> Result<BindPreference, DesktopNetworkError> {
    match request.mode.as_str() {
        "automatic" if request.interface_name.is_none() && request.address.is_none() => {
            Ok(BindPreference::Automatic)
        }
        "explicit" => {
            let interface_name = request.interface_name.as_deref().ok_or_else(|| {
                DesktopNetworkError::invalid_argument(
                    "explicit network preference requires an interface name",
                )
            })?;
            if interface_name.is_empty()
                || interface_name.len() > 128
                || interface_name.trim() != interface_name
            {
                return Err(DesktopNetworkError::invalid_argument(
                    "network interface name is invalid",
                ));
            }
            let address = request.address.as_deref().ok_or_else(|| {
                DesktopNetworkError::invalid_argument(
                    "explicit network preference requires an IPv4 address",
                )
            })?;
            let address = address.parse::<Ipv4Addr>().map_err(|_| {
                DesktopNetworkError::invalid_argument(
                    "explicit network preference address must be canonical IPv4",
                )
            })?;
            Ok(BindPreference::Explicit {
                interface_name: interface_name.to_owned(),
                address,
            })
        }
        _ => Err(DesktopNetworkError::invalid_argument(
            "network preference must be automatic or a complete explicit selection",
        )),
    }
}

/// The interface production would actually bind to, for tests that need a
/// real LAN address.
///
/// Tests used to hand-roll this filter -- "up, not loopback, not tun, not
/// point-to-point, has a private IPv4" -- which accepts strictly more than
/// production does: `classify` additionally excludes VPN and *container*
/// interfaces. On a host with Docker bridges (172.x, private, up, physical
/// by every one of those predicates) the two disagreed, and `netdev`'s
/// ordering decided which interface a test picked. That is what made
/// `port_in_use_and_partial_bind_cleanup_are_preserved_by_shared_transport`
/// fail roughly three runs in four with "no active private-LAN IPv4 address
/// is available": the test had handed production a bridge, and production
/// correctly refused it.
///
/// Going through `normalize_interfaces` and `select_address` means the answer
/// is production's own, so the two cannot drift apart again.
#[cfg(test)]
pub(super) fn first_bindable_private_lan_address() -> Option<(String, u32, std::net::Ipv4Addr)> {
    let records = normalize_interfaces(netdev::get_interfaces()).ok()?;
    let selected = select_address(&records, &BindPreference::Automatic).ok()?;
    Some((
        selected.interface_name,
        selected.interface_index,
        selected.address,
    ))
}

fn is_active(interface: &InterfaceRecord) -> bool {
    interface.up && (interface.running || interface.oper_up)
}

fn classify(interface: &InterfaceRecord, address: IpAddr) -> NetworkAddressClassDto {
    if interface.loopback || address.is_loopback() {
        return NetworkAddressClassDto::Loopback;
    }
    if is_link_local(address) {
        return NetworkAddressClassDto::LinkLocal;
    }
    if is_vpn(interface) {
        return NetworkAddressClassDto::Vpn;
    }
    if is_container(interface) {
        return NetworkAddressClassDto::Container;
    }
    if is_private_lan(address) {
        return NetworkAddressClassDto::PrivateLan;
    }
    NetworkAddressClassDto::Other
}

fn is_link_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_link_local(),
        IpAddr::V6(address) => address.is_unicast_link_local(),
    }
}

fn is_private_lan(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_private()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_broadcast()
        }
        IpAddr::V6(address) => is_unique_local(address),
    }
}

fn is_unique_local(address: Ipv6Addr) -> bool {
    address.octets()[0] & 0xfe == 0xfc
}

fn is_vpn(interface: &InterfaceRecord) -> bool {
    if interface.tun || interface.point_to_point {
        return true;
    }
    let name = interface.name.to_ascii_lowercase();
    [
        "tun",
        "tap",
        "wg",
        "tailscale",
        "utun",
        "ppp",
        "ipsec",
        "zerotier",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn is_container(interface: &InterfaceRecord) -> bool {
    let name = interface.name.to_ascii_lowercase();
    [
        "docker", "br-", "veth", "podman", "cni", "virbr", "lxc", "lxd", "flannel",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
pub(super) use HostPorts as TestHostPorts;
