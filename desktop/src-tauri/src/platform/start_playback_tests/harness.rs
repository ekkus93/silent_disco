//! Real actor/transport driving helpers shared by every test in this suite
//! (automated and manual): standing up a desktop host session, joining and
//! approving a listener, and polling the actor/transport for expected
//! state without racing its asynchronous effect processing.

use crate::platform::network::{
    AddressRecord, DesktopHostNetworkControl, InterfaceRecord, TestHostPorts,
};
use silent_disco_core::domain::{AppRole, ApprovalMode, DeviceId};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame, StreamStart, SyncResponse,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, CoreSnapshot, HostDraftPatch, InviteCodePatch,
    PlatformEffectRequest, PlatformEvent, PlatformOperationCompletion, SnapshotRevision,
    TransportEffect,
};
use silent_disco_core::transport::{
    ListenerTransportConfig, ListenerTransportNode, SystemTransportClock, TransportChannel,
    TransportEvent, TransportFactory, production_transport_factory,
};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

pub(super) const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Drives a real actor through role selection, host draft, and session
/// creation, then binds a real desktop host transport on the given local
/// interface address and completes the advertising handshake.
#[allow(clippy::type_complexity)]
pub(super) fn start_host_session(
    descriptor: AudioSourceDescriptor,
    interface_name: String,
    interface_index: u32,
    address: Ipv4Addr,
) -> (
    CoreActorRuntime,
    silent_disco_core::runtime::CoreActorHandle,
    Receiver<CoreNotification>,
    silent_disco_core::runtime::SessionAdvertisement,
    Arc<DesktopHostNetworkControl>,
    silent_disco_core::runtime::NetworkEndpoint,
) {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-playback-host").expect("host id")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    next_snapshot(&receiver, 0);
    submit(
        &handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(&receiver, 1);
    submit(
        &handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Streaming playback host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(&receiver, 2);
    submit(&handle, 2, CoreCommand::CreateHostSession);
    next_snapshot(&receiver, 3);
    let advertisement_effect = next_effect(&receiver);
    let PlatformEffectRequest::StartAdvertising(advertisement) = advertisement_effect.request
    else {
        panic!("expected start advertising effect");
    };

    let network = Arc::new(DesktopHostNetworkControl::with_components(
        Arc::new(FixedInterfaceProvider::new(
            interface_name,
            interface_index,
            address,
        )),
        Arc::new(production_transport_factory()),
        TestHostPorts::default(),
    ));
    let endpoint = network
        .start_host(&advertisement, handle.clone())
        .expect("start desktop host transport");
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertisement_effect.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising completion");
    next_snapshot(&receiver, 4);
    (actor, handle, receiver, advertisement, network, endpoint)
}

/// Connects a real loopback-bound listener, joins, and drives the approval
/// through a real `CoreCommand::ApproveJoin` -> `TransportEffect` ->
/// `authorize_peer_ports` round trip -- exercising the same wiring a real
/// listener's ports depend on before any sync/audio frame can reach it.
pub(super) fn join_and_approve_listener(
    address: Ipv4Addr,
    endpoint: silent_disco_core::runtime::NetworkEndpoint,
    advertisement: &silent_disco_core::runtime::SessionAdvertisement,
    handle: &silent_disco_core::runtime::CoreActorHandle,
    receiver: &Receiver<CoreNotification>,
    network: &DesktopHostNetworkControl,
) -> Box<dyn ListenerTransportNode> {
    let listener_id = DeviceId::new("desktop-playback-listener").expect("listener id");
    let mut listener = production_transport_factory()
        .connect_listener(
            ListenerTransportConfig {
                local_address: IpAddr::V4(address),
                ..ListenerTransportConfig::loopback(
                    advertisement.session_id.clone(),
                    listener_id.clone(),
                    endpoint,
                )
            },
            Arc::new(SystemTransportClock::default()),
        )
        .expect("listener connects to desktop host");
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: advertisement.session_id.clone(),
            device: DeviceIdentity {
                device_id: listener_id.clone(),
                display_name: "Streaming listener".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("send join request");
    wait_for_hello(&mut *listener);

    let joined = wait_snapshot(handle, |snapshot| {
        !snapshot.pending_join_requests.is_empty()
    });
    let request_id = joined.pending_join_requests[0].request_id.clone();
    submit(
        handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    listener
}

struct FixedInterfaceProvider {
    record: InterfaceRecord,
}

impl FixedInterfaceProvider {
    fn new(name: String, index: u32, address: Ipv4Addr) -> Self {
        Self {
            record: InterfaceRecord {
                name,
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
                    address: IpAddr::V4(address),
                    prefix_length: 24,
                }],
            },
        }
    }
}

impl crate::platform::network::NetworkInterfaceProvider for FixedInterfaceProvider {
    fn interfaces(
        &self,
    ) -> Result<Vec<InterfaceRecord>, crate::platform::network::DesktopNetworkError> {
        Ok(vec![self.record.clone()])
    }
}

/// Finds a real, currently-active private-LAN IPv4 interface on this machine,
/// mirroring `network_tests.rs`'s bind-conflict test -- a real production
/// socket bind requires an address genuinely assigned to a local interface,
/// so this cannot be faked the way `network_tests.rs`'s simulated-transport
/// tests fake interface records.
/// The interface production would bind, not a re-derived guess.
///
/// This used to hand-roll the filter, which accepted more than production's
/// `classify` does -- notably container bridges, which are private, up, and
/// physical by every predicate used here. Requiring `interface.default` made
/// it pick the right one on this machine by luck rather than by rule; the
/// same filter without that clause, in `network_tests.rs`, picked a Docker
/// bridge often enough to fail three runs in four. Delegating means a device
/// test can never advertise an address a phone cannot reach.
pub(super) fn real_private_lan_address() -> Option<(String, u32, Ipv4Addr)> {
    crate::platform::network::first_bindable_private_lan_address()
}

pub(super) fn submit(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    revision: u64,
    command: CoreCommand,
) {
    handle
        .submit_command(
            CoreCommandRequest::new(SnapshotRevision::new(revision), command).expect("command"),
        )
        .expect("submit command");
}

fn next_snapshot(receiver: &Receiver<CoreNotification>, minimum: u64) -> CoreSnapshot {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot)) if snapshot.revision.get() >= minimum => {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for snapshot"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn next_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::PlatformEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

pub(super) fn next_transport_effect(receiver: &Receiver<CoreNotification>) -> TransportEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for transport effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

pub(super) fn wait_snapshot(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
) -> CoreSnapshot {
    wait_snapshot_for(handle, predicate, TEST_TIMEOUT)
}

/// As [`wait_snapshot`], but with an explicit timeout instead of the fast
/// loopback-test default. Real devices/emulators are measurably slower and
/// more congested than the in-process loopback listener the automated
/// suite uses -- a manual run against a real Android emulator observed
/// `queue_overflows=930` under sustained playback and a stop-transition
/// that took longer than `TEST_TIMEOUT` (10s) to land, even though it did
/// land moments later. Manual device tests should use a longer timeout
/// here rather than papering over that by loosening the fast tests' shared
/// constant.
pub(super) fn wait_snapshot_for(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
    timeout: Duration,
) -> CoreSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for actor state; observed playback_state={:?} \
             host_lifecycle={:?} revision={} pending_joins={} listeners={}",
            snapshot.playback_state,
            snapshot.host_lifecycle,
            snapshot.revision.get(),
            snapshot.pending_join_requests.len(),
            snapshot.listeners.len(),
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_hello(listener: &mut dyn ListenerTransportNode) {
    wait_for_control(listener, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
}

pub(super) fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    matches_expected: impl Fn(&ControlMessage) -> bool,
) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(message),
                ..
            }) if matches_expected(&message) => return,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for expected control message"
        );
    }
}

/// Like [`wait_for_control`], but returns the matched `StreamStart` payload
/// instead of discarding it -- callers that need to inspect
/// `host_start_time_ms` (e.g. confirming a resume's re-anchor actually
/// shifted it) can't do that with `wait_for_control` alone.
pub(super) fn wait_for_stream_start(listener: &mut dyn ListenerTransportNode) -> StreamStart {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::StreamStart(start)),
                ..
            }) => return start,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a StreamStart control message"
        );
    }
}

pub(super) fn wait_for_audio(
    listener: &mut dyn ListenerTransportNode,
) -> silent_disco_core::protocol::AudioDatagram {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Audio,
                frame: ProtocolFrame::Audio(datagram),
                ..
            }) => return datagram,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for an audio datagram"
        );
    }
}

pub(super) fn wait_for_sync_response(
    listener: &mut dyn ListenerTransportNode,
    correlation_id: u64,
) -> SyncResponse {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                frame: ProtocolFrame::SyncResponse(response),
                ..
            }) if response.correlation_id == correlation_id => return response,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a sync response"
        );
    }
}
