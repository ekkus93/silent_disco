use super::mdns::{MdnsPublishError, MdnsPublisher, MdnsRegistration};
use super::network::{
    AddressRecord, DesktopHostNetworkControl, InterfaceRecord, NetworkErrorKind,
    NetworkInterfaceProvider, TestHostPorts,
};
use super::network_dto::{NetworkAddressClassDto, SetNetworkBindPreferenceRequest};
use silent_disco_core::domain::{DeviceId, SessionId};
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame};
use silent_disco_core::runtime::{NetworkEndpoint, SessionAdvertisement};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,
    ListenerTransportNode, TransportClock, TransportCounters, TransportDelivery, TransportError,
    TransportEvent, TransportFactory, production_transport_factory,
};
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn interface(name: &str, index: u32, address: IpAddr) -> InterfaceRecord {
    InterfaceRecord {
        name: name.to_owned(),
        index,
        up: true,
        running: true,
        oper_up: true,
        loopback: false,
        point_to_point: false,
        tun: false,
        physical: true,
        default_route: false,
        addresses: vec![AddressRecord {
            address,
            prefix_length: if address.is_ipv4() { 24 } else { 64 },
        }],
    }
}

fn loopback() -> InterfaceRecord {
    InterfaceRecord {
        name: "lo".to_owned(),
        index: 1,
        up: true,
        running: true,
        oper_up: true,
        loopback: true,
        point_to_point: false,
        tun: false,
        physical: false,
        default_route: false,
        addresses: vec![AddressRecord {
            address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            prefix_length: 8,
        }],
    }
}

#[derive(Default)]
struct SequenceProvider {
    snapshots: Mutex<VecDeque<Vec<InterfaceRecord>>>,
    last: Mutex<Vec<InterfaceRecord>>,
}

impl SequenceProvider {
    fn new(snapshots: impl IntoIterator<Item = Vec<InterfaceRecord>>) -> Self {
        Self {
            snapshots: Mutex::new(snapshots.into_iter().collect()),
            last: Mutex::new(Vec::new()),
        }
    }
}

impl NetworkInterfaceProvider for SequenceProvider {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, super::network::DesktopNetworkError> {
        let mut snapshots = self.snapshots.lock().expect("snapshot queue");
        if let Some(next) = snapshots.pop_front() {
            *self.last.lock().expect("last snapshot") = next.clone();
            Ok(next)
        } else {
            Ok(self.last.lock().expect("last snapshot").clone())
        }
    }
}

struct FakeHostNode {
    endpoint: NetworkEndpoint,
    shutdown_called: Arc<AtomicBool>,
}

impl HostTransportNode for FakeHostNode {
    fn endpoint(&self) -> NetworkEndpoint {
        self.endpoint
    }

    fn authorize_peer(
        &self,
        _device_id: &DeviceId,
        _routes: ListenerDatagramRoutes,
    ) -> Result<(), TransportError> {
        panic!("unused fake host operation")
    }

    fn authorize_peer_ports(
        &self,
        _device_id: &DeviceId,
        _sync_port: u16,
        _audio_port: u16,
    ) -> Result<(), TransportError> {
        panic!("unused fake host operation")
    }

    fn disconnect_peer(&self, _device_id: &DeviceId) -> Result<(), TransportError> {
        panic!("unused fake host operation")
    }

    fn send_pending_control(
        &self,
        _device_id: &DeviceId,
        _message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        panic!("unused fake host operation")
    }

    fn send_control(
        &self,
        _device_id: &DeviceId,
        _message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        panic!("unused fake host operation")
    }

    fn broadcast_control(
        &self,
        _message: &ControlMessage,
    ) -> Result<TransportDelivery, TransportError> {
        panic!("unused fake host operation")
    }

    fn broadcast_sync(&self, _frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        panic!("unused fake host operation")
    }

    fn broadcast_audio(&self, _frame: &ProtocolFrame) -> Result<TransportDelivery, TransportError> {
        panic!("unused fake host operation")
    }

    fn recv_event(&mut self, _timeout: Duration) -> Result<TransportEvent, TransportError> {
        Err(TransportError::timeout(
            silent_disco_core::transport::TransportChannel::Runtime,
            "fake host receive timed out",
        ))
    }

    fn counters(&self) -> TransportCounters {
        TransportCounters::default()
    }

    fn shutdown(&mut self) -> Result<(), TransportError> {
        self.shutdown_called.store(true, Ordering::Release);
        Ok(())
    }
}

struct FakeTransportFactory {
    shutdown_called: Arc<AtomicBool>,
    seen_config: Mutex<Option<HostTransportConfig>>,
}

impl FakeTransportFactory {
    fn new(shutdown_called: Arc<AtomicBool>) -> Self {
        Self {
            shutdown_called,
            seen_config: Mutex::new(None),
        }
    }
}

impl TransportFactory for FakeTransportFactory {
    fn bind_host(
        &self,
        config: HostTransportConfig,
        _clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn HostTransportNode>, TransportError> {
        *self.seen_config.lock().expect("seen config") = Some(config.clone());
        let endpoint =
            NetworkEndpoint::new(config.bind_address, 41001, 41002, 41003).expect("fake endpoint");
        Ok(Box::new(FakeHostNode {
            endpoint,
            shutdown_called: Arc::clone(&self.shutdown_called),
        }))
    }

    fn connect_listener(
        &self,
        _config: ListenerTransportConfig,
        _clock: Arc<dyn TransportClock>,
    ) -> Result<Box<dyn ListenerTransportNode>, TransportError> {
        panic!("unused fake listener operation")
    }
}

/// Simulates 30.3's "daemon/multicast unavailable" scenario: the `mdns-sd`
/// daemon could not be created or reached (e.g. the multicast socket could
/// not be bound), so every publish attempt fails the same way a real
/// unavailable daemon would.
struct DaemonUnavailableMdnsPublisher;

impl MdnsPublisher for DaemonUnavailableMdnsPublisher {
    fn publish(
        &self,
        _advertisement: &SessionAdvertisement,
        _endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        Err(MdnsPublishError::DaemonUnavailable {
            message: "multicast socket could not be bound".to_owned(),
        })
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

/// A registration whose withdrawal always fails, standing in for 30.3's
/// "interface disappears" scenario: the bound interface vanished mid-session,
/// so the daemon can no longer send the withdrawal announcement over it.
struct VanishedInterfaceRegistration;

impl MdnsRegistration for VanishedInterfaceRegistration {
    fn withdraw(&self) -> Result<(), MdnsPublishError> {
        Err(MdnsPublishError::WithdrawFailed {
            message: "interface vanished before withdrawal could be sent".to_owned(),
        })
    }
}

struct VanishingInterfaceMdnsPublisher;

impl MdnsPublisher for VanishingInterfaceMdnsPublisher {
    fn publish(
        &self,
        _advertisement: &SessionAdvertisement,
        _endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        Ok(Box::new(VanishedInterfaceRegistration))
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

fn advertisement() -> SessionAdvertisement {
    SessionAdvertisement::new(
        SessionId::new("session-block21").expect("session ID"),
        DeviceId::new("desktop-block21").expect("device ID"),
        "Block 21",
        silent_disco_core::domain::ApprovalMode::Manual,
        2,
        None,
    )
    .expect("advertisement")
}

fn automatic_request() -> SetNetworkBindPreferenceRequest {
    SetNetworkBindPreferenceRequest {
        mode: "automatic".to_owned(),
        interface_name: None,
        address: None,
    }
}

fn explicit_request(name: &str, address: &str) -> SetNetworkBindPreferenceRequest {
    SetNetworkBindPreferenceRequest {
        mode: "explicit".to_owned(),
        interface_name: Some(name.to_owned()),
        address: Some(address.to_owned()),
    }
}

#[test]
fn loopback_only_is_visible_but_not_selectable() {
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([vec![loopback()]])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    let snapshot = control.snapshot().expect("network snapshot");
    assert_eq!(snapshot.candidates.len(), 1);
    assert_eq!(
        snapshot.candidates[0].classification,
        NetworkAddressClassDto::Loopback
    );
    assert!(!snapshot.candidates[0].selectable);
    assert!(snapshot.automatic_selection.is_none());
    assert!(
        snapshot
            .selection_error
            .expect("selection error")
            .contains("no active private-LAN")
    );
}

#[test]
fn one_private_lan_address_is_selected_and_bound_through_shared_factory() {
    let address = Ipv4Addr::new(192, 168, 50, 10);
    let records = vec![interface("enp1s0", 2, IpAddr::V4(address))];
    let provider = Arc::new(SequenceProvider::new([records.clone(), records]));
    let shutdown_called = Arc::new(AtomicBool::new(false));
    let factory = Arc::new(FakeTransportFactory::new(Arc::clone(&shutdown_called)));
    let control = DesktopHostNetworkControl::with_components(
        provider,
        factory.clone(),
        TestHostPorts::default(),
    );

    let endpoint = control
        .start_host_inner(&advertisement())
        .expect("bind host endpoint");
    assert_eq!(endpoint.address, IpAddr::V4(address));
    assert_eq!(
        factory
            .seen_config
            .lock()
            .expect("seen config")
            .as_ref()
            .expect("host config")
            .bind_address,
        IpAddr::V4(address)
    );
    let snapshot = control.snapshot().expect("bound snapshot");
    assert!(snapshot.active_binding_valid);
    assert_eq!(
        snapshot.active_binding.expect("active binding").address,
        address.to_string()
    );
    control.stop_host_inner().expect("stop host");
    assert!(shutdown_called.load(Ordering::Acquire));
}

/// 30.3 "daemon/multicast unavailable": a publish failure must never fail
/// the host start itself -- the manual connection payload has to stay fully
/// usable -- but it must also never be silently swallowed. It has to show
/// up, honestly, in the snapshot the UI reads.
#[test]
fn a_publish_failure_does_not_fail_host_start_but_is_visible_in_the_snapshot() {
    let address = Ipv4Addr::new(192, 168, 50, 11);
    let records = vec![interface("enp1s0", 2, IpAddr::V4(address))];
    let provider = Arc::new(SequenceProvider::new([records.clone(), records]));
    let factory = Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false))));
    let control =
        DesktopHostNetworkControl::with_components(provider, factory, TestHostPorts::default())
            .with_mdns_publisher(Arc::new(DaemonUnavailableMdnsPublisher));

    control
        .start_host_inner(&advertisement())
        .expect("host start succeeds despite the mDNS daemon being unavailable");

    let snapshot = control.snapshot().expect("bound snapshot");
    let binding = snapshot.active_binding.expect("active binding");
    assert!(!binding.mdns.active);
    assert!(
        binding
            .mdns
            .failure_reason
            .expect("failure reason")
            .contains("daemon unavailable")
    );

    control
        .stop_host_inner()
        .expect("stop host cleanly despite mDNS never having published");
}

/// 30.3 "interface disappears": withdrawal can fail if the bound interface
/// vanished mid-session, so the daemon can no longer send the withdrawal
/// announcement. `stop_host_inner` must still tear the rest of the host
/// down (socket, playback, runtime) and must still report the withdrawal
/// failure rather than claiming a clean stop.
#[test]
fn a_withdraw_failure_still_tears_down_the_host_but_is_reported_not_swallowed() {
    let address = Ipv4Addr::new(192, 168, 50, 12);
    let records = vec![interface("enp1s0", 2, IpAddr::V4(address))];
    let provider = Arc::new(SequenceProvider::new([records.clone(), records]));
    let shutdown_called = Arc::new(AtomicBool::new(false));
    let factory = Arc::new(FakeTransportFactory::new(Arc::clone(&shutdown_called)));
    let control =
        DesktopHostNetworkControl::with_components(provider, factory, TestHostPorts::default())
            .with_mdns_publisher(Arc::new(VanishingInterfaceMdnsPublisher));

    control
        .start_host_inner(&advertisement())
        .expect("bind host endpoint");
    let snapshot = control.snapshot().expect("bound snapshot");
    assert!(snapshot.active_binding.expect("active binding").mdns.active);

    let stop_result = control.stop_host_inner();
    assert!(
        stop_result
            .expect_err("a failed withdrawal must be reported, not swallowed")
            .to_string()
            .contains("mDNS withdrawal failed")
    );
    // The rest of teardown still happened even though withdrawal failed --
    // the socket must never be leaked just because mDNS could not be
    // cleanly withdrawn.
    assert!(shutdown_called.load(Ordering::Acquire));
}

#[test]
fn multiple_lan_addresses_require_explicit_selection_unless_one_is_default() {
    let first = interface("enp1s0", 2, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
    let second = interface("wlan0", 3, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 20)));
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([vec![first.clone(), second.clone()]])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    let snapshot = control.snapshot().expect("ambiguous snapshot");
    assert!(snapshot.requires_explicit_selection);
    assert!(snapshot.automatic_selection.is_none());

    control
        .set_preference(&explicit_request("wlan0", "10.0.0.20"))
        .expect("explicit preference");

    let mut default_first = first;
    default_first.default_route = true;
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([vec![default_first, second]])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    let snapshot = control.snapshot().expect("default selection");
    assert_eq!(
        snapshot
            .automatic_selection
            .expect("automatic address")
            .address,
        "192.168.1.10"
    );
}

#[test]
fn vpn_container_link_local_and_ipv6_candidates_are_classified_and_excluded() {
    let mut vpn = interface("wg0", 4, IpAddr::V4(Ipv4Addr::new(10, 8, 0, 2)));
    vpn.tun = true;
    vpn.physical = false;
    let mut container = interface("docker0", 5, IpAddr::V4(Ipv4Addr::new(172, 17, 0, 1)));
    container.physical = false;
    let link_local = interface("enp2s0", 6, IpAddr::V4(Ipv4Addr::new(169, 254, 10, 1)));
    let ipv6 = interface(
        "enp3s0",
        7,
        IpAddr::V6("fd00::10".parse::<Ipv6Addr>().expect("IPv6")),
    );
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([vec![
            vpn, container, link_local, ipv6,
        ]])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    let snapshot = control.snapshot().expect("classified snapshot");
    let classes = snapshot
        .candidates
        .iter()
        .map(|candidate| candidate.classification)
        .collect::<Vec<_>>();
    assert!(classes.contains(&NetworkAddressClassDto::Vpn));
    assert!(classes.contains(&NetworkAddressClassDto::Container));
    assert!(classes.contains(&NetworkAddressClassDto::LinkLocal));
    assert!(classes.contains(&NetworkAddressClassDto::PrivateLan));
    assert!(
        snapshot
            .candidates
            .iter()
            .all(|candidate| !candidate.selectable)
    );
}

#[test]
fn requested_address_disappearing_before_bind_is_a_visible_failure() {
    let address = IpAddr::V4(Ipv4Addr::new(192, 168, 10, 5));
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([
            vec![interface("enp1s0", 2, address)],
            vec![interface("enp1s0", 2, address)],
            Vec::new(),
        ])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    control
        .set_preference(&explicit_request("enp1s0", "192.168.10.5"))
        .expect("explicit preference");
    let error = control
        .start_host_inner(&advertisement())
        .expect_err("disappearing address must fail");
    assert_eq!(error.kind, NetworkErrorKind::Unavailable);
}

#[test]
fn invalid_and_ipv6_explicit_preferences_are_rejected() {
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([vec![interface(
            "enp1s0",
            2,
            IpAddr::V6("fd00::1".parse().expect("IPv6")),
        )]])),
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    assert!(
        control
            .set_preference(&explicit_request("enp1s0", "fd00::1"))
            .is_err()
    );
    assert!(
        control
            .set_preference(&SetNetworkBindPreferenceRequest {
                mode: "automatic".to_owned(),
                interface_name: Some("enp1s0".to_owned()),
                address: None,
            })
            .is_err()
    );
    control
        .set_preference(&automatic_request())
        .expect("restore automatic preference");
}

#[test]
fn active_interface_change_is_reported_in_snapshot() {
    let address = IpAddr::V4(Ipv4Addr::new(192, 168, 44, 2));
    let provider = Arc::new(SequenceProvider::new([
        vec![interface("enp1s0", 2, address)],
        vec![interface("enp1s0", 2, address)],
        Vec::new(),
    ]));
    let control = DesktopHostNetworkControl::with_components(
        provider,
        Arc::new(FakeTransportFactory::new(Arc::new(AtomicBool::new(false)))),
        TestHostPorts::default(),
    );
    control
        .start_host_inner(&advertisement())
        .expect("start host");
    let snapshot = control.snapshot().expect("changed snapshot");
    assert!(!snapshot.active_binding_valid);
    assert!(snapshot.interface_change.is_some());
    control.stop_host_inner().expect("stop host");
}

#[test]
fn port_in_use_and_partial_bind_cleanup_are_preserved_by_shared_transport() {
    // Ask production which interface it would bind, rather than re-deriving
    // it: a hand-rolled filter here accepted container bridges that
    // `classify` rejects, and picking one made this test fail.
    let Some((interface_name, interface_index, address)) =
        super::network::first_bindable_private_lan_address()
    else {
        eprintln!(
            "no private LAN interface on this CI host; policy coverage remains deterministic"
        );
        return;
    };
    let control_port = reserve_tcp_port(address);
    let sync_guard = std::net::UdpSocket::bind((address, 0)).expect("reserve sync port");
    let sync_port = sync_guard.local_addr().expect("sync address").port();
    let audio_port = reserve_udp_port(address);
    let record = interface(&interface_name, interface_index, IpAddr::V4(address));
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([vec![record.clone()], vec![record]])),
        Arc::new(production_transport_factory()),
        TestHostPorts {
            control: control_port,
            sync: sync_port,
            audio: audio_port,
        },
    );
    let error = control
        .start_host_inner(&advertisement())
        .expect_err("occupied synchronization port must fail");
    assert_eq!(
        error.kind,
        NetworkErrorKind::Transport,
        "occupied sync port must fail at the transport bind; got {:?}: {}",
        error.kind,
        error.message
    );
    std::net::TcpListener::bind((address, control_port))
        .expect("partial TCP bind must be released");
    std::net::UdpSocket::bind((address, audio_port))
        .expect("unreached audio port remains available");
}

fn reserve_tcp_port(address: Ipv4Addr) -> u16 {
    let socket = std::net::TcpListener::bind((address, 0)).expect("reserve TCP port");
    socket.local_addr().expect("TCP local address").port()
}

fn reserve_udp_port(address: Ipv4Addr) -> u16 {
    let socket = std::net::UdpSocket::bind((address, 0)).expect("reserve UDP port");
    socket.local_addr().expect("UDP local address").port()
}

/// Records whether the daemon-level `shutdown()` was ever invoked --
/// distinct from a per-publication `withdraw()` -- so Block 36.2's "mDNS
/// daemon shutdown, even when no binding was ever active" requirement is
/// directly observable.
#[derive(Default)]
struct RecordingMdnsPublisher {
    shutdown_called: AtomicBool,
}

impl MdnsPublisher for RecordingMdnsPublisher {
    fn publish(
        &self,
        _advertisement: &SessionAdvertisement,
        _endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        Ok(Box::new(NoopMdnsRegistration))
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        self.shutdown_called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct NoopMdnsRegistration;

impl MdnsRegistration for NoopMdnsRegistration {
    fn withdraw(&self) -> Result<(), MdnsPublishError> {
        Ok(())
    }
}

/// A daemon-level publisher whose `shutdown()` always fails, standing in
/// for a daemon that could not be cleanly torn down.
struct FailingShutdownMdnsPublisher;

impl MdnsPublisher for FailingShutdownMdnsPublisher {
    fn publish(
        &self,
        _advertisement: &SessionAdvertisement,
        _endpoint: NetworkEndpoint,
    ) -> Result<Box<dyn MdnsRegistration>, MdnsPublishError> {
        Ok(Box::new(NoopMdnsRegistration))
    }

    fn shutdown(&self) -> Result<(), MdnsPublishError> {
        Err(MdnsPublishError::DaemonUnavailable {
            message: "daemon refused to confirm shutdown".to_owned(),
        })
    }
}

/// Block 36.2 "withdraw mDNS" extends to the daemon itself, not just an
/// active publication: `shutdown()` must reach the daemon-level publisher
/// even when this control never bound a host session.
#[test]
fn network_control_shutdown_shuts_down_the_mdns_daemon_even_when_never_active() {
    let mdns = Arc::new(RecordingMdnsPublisher::default());
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([Vec::new()])),
        Arc::new(production_transport_factory()),
        TestHostPorts::default(),
    )
    .with_mdns_publisher(mdns.clone());

    control.shutdown().expect("inactive shutdown");

    assert!(
        mdns.shutdown_called.load(Ordering::SeqCst),
        "the daemon-level publisher must be shut down even without an active binding"
    );
}

/// A daemon shutdown failure must be reported, not silently swallowed.
#[test]
fn network_control_shutdown_reports_a_failing_mdns_daemon_shutdown() {
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([Vec::new()])),
        Arc::new(production_transport_factory()),
        TestHostPorts::default(),
    )
    .with_mdns_publisher(Arc::new(FailingShutdownMdnsPublisher));

    let error = control
        .shutdown()
        .expect_err("a failing daemon shutdown must be reported");
    assert!(error.message.contains("mDNS daemon shutdown failed"));
}

#[test]
fn network_control_shutdown_is_idempotent_when_inactive() {
    let control = DesktopHostNetworkControl::with_components(
        Arc::new(SequenceProvider::new([Vec::new()])),
        Arc::new(production_transport_factory()),
        TestHostPorts::default(),
    );
    control.shutdown().expect("inactive shutdown");
}
