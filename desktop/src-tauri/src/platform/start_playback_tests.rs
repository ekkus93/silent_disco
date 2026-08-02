use super::file_picker::{AudioContainer, InspectedAudioSource, SelectedSourceRegistry};
use super::network::{AddressRecord, DesktopHostNetworkControl, InterfaceRecord, TestHostPorts};
use super::start_playback;
use silent_disco_core::domain::{AppRole, ApprovalMode, DeviceId, MonotonicMillis};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame, SyncRequest, SyncResponse,
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
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn desktop_host_streams_real_audio_and_answers_sync_requests() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!(
            "no private LAN interface on this CI host; streaming playback coverage remains deterministic"
        );
        return;
    };

    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_source(&temp);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor, interface_name, interface_index, address);

    let mut listener = join_and_approve_listener(
        address,
        endpoint,
        &advertisement,
        &handle,
        &receiver,
        &network,
    );

    start_playback::start(&handle, &network, &registry).expect("start playback");

    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::StreamStart(_))
    });
    let first_audio = wait_for_audio(&mut *listener);
    assert_eq!(first_audio.session_id, advertisement.session_id);
    assert!(!first_audio.payload.is_empty());

    let correlation_id = 7;
    listener
        .send_sync_request(&SyncRequest {
            session_id: advertisement.session_id.clone(),
            correlation_id,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(0),
        })
        .expect("send sync request");
    let sync_response = wait_for_sync_response(&mut *listener, correlation_id);
    assert_eq!(sync_response.session_id, advertisement.session_id);
    assert!(
        sync_response.t3_host_send_elapsed_ms.get()
            >= sync_response.t2_host_receive_elapsed_ms.get()
    );

    network.stop_playback().expect("stop playback");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::Stop(_))
    });

    listener.shutdown().expect("listener shutdown");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
}

fn stage_source(temp: &TempDir) -> (AudioSourceDescriptor, SelectedSourceRegistry) {
    let source_path = temp.path().join("source.wav");
    fs::write(&source_path, pcm_wav()).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(
        "desktop-block-playback-source",
        "source.wav",
        Some(byte_length),
        None,
    )
    .expect("descriptor");
    let registry = SelectedSourceRegistry::new();
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register staged source");
    (descriptor, registry)
}

/// Drives a real actor through role selection, host draft, and session
/// creation, then binds a real desktop host transport on the given local
/// interface address and completes the advertising handshake.
#[allow(clippy::type_complexity)]
fn start_host_session(
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
fn join_and_approve_listener(
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

/// Not part of the automated suite: binds a real desktop host on this
/// machine's real LAN address, prints a real connection payload, and waits
/// for an actual external listener (e.g. a phone on the same Wi-Fi network,
/// pasting the printed payload into the app's "Connect manually" screen) to
/// join before streaming a first long "song" (a 300Hz tone), then switching
/// mid-session to a second, audibly distinct "song" (a 900Hz tone) -- the
/// same stop -> update draft -> start sequence a real user changing tracks
/// would trigger, including a fresh stream ID for the second song. Run
/// explicitly with:
/// `cargo +1.97.1 test --manifest-path desktop/src-tauri/Cargo.toml manual_real_android_listener -- --ignored --nocapture`
#[test]
#[ignore = "requires a real external listener device on the same LAN, driven manually"]
fn manual_real_android_listener_plays_a_song_change() {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        panic!("no private LAN interface available for the manual device test");
    };
    let temp = TempDir::new().expect("temp");
    let registry = SelectedSourceRegistry::new();
    let descriptor_a = stage_tone_source(&temp, &registry, "song-a", 300.0, 40);
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session(descriptor_a, interface_name, interface_index, address);

    eprintln!(
        "=== paste this connection payload into the Android app's Connect manually screen ==="
    );
    eprintln!(
        "{{\"hostAddress\":\"{address}\",\"controlPort\":{},\"syncPort\":{},\"audioPort\":{},\"sessionId\":\"{}\",\"protocolVersion\":{},\"inviteCodeRequired\":false,\"expiresAtMs\":null}}",
        endpoint.control_port,
        endpoint.sync_port,
        endpoint.audio_port,
        advertisement.session_id.as_str(),
        advertisement.protocol_version,
    );
    eprintln!("waiting up to 8 minutes for a real join request...");

    let deadline = Instant::now() + Duration::from_mins(8);
    let joined = loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if !snapshot.pending_join_requests.is_empty() {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a real join request"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let request = &joined.pending_join_requests[0];
    eprintln!(
        "real join request received from device_id={} display_name={}",
        request.device_id.as_str(),
        request.display_name
    );
    let request_id = request.request_id.clone();
    submit(
        &handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(&receiver);
    network
        .dispatch_transport_effect(approval_effect)
        .expect("dispatch join approval");
    eprintln!("approved and authorized.");

    eprintln!("=== song 1/2: \"song-a\", a 300Hz tone -- starting playback ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    eprintln!("song-a playing for 40s...");
    std::thread::sleep(Duration::from_secs(40));

    eprintln!("=== switching songs: stopping song-a ===");
    network.stop_playback().expect("stop playback");
    wait_snapshot(&handle, |snapshot| {
        snapshot.playback_state == silent_disco_core::domain::PlaybackState::Stopped
    });

    let descriptor_b = stage_tone_source(&temp, &registry, "song-b", 900.0, 40);
    let current = handle.current_snapshot().expect("current snapshot");
    submit(
        &handle,
        current.revision.get(),
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: None,
            approval_mode: None,
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(descriptor_b.clone()),
            remember_approved_devices: None,
        }),
    );
    wait_snapshot(&handle, |snapshot| {
        snapshot
            .host_draft
            .audio_source
            .as_ref()
            .is_some_and(|source| source.source_id == descriptor_b.source_id)
    });

    eprintln!("=== song 2/2: \"song-b\", a 900Hz tone -- starting playback ===");
    start_playback::start(&handle, &network, &registry).expect("start playback");
    eprintln!("song-b playing for 40s...");
    std::thread::sleep(Duration::from_secs(40));

    eprintln!("stopping playback...");
    network.stop_playback().expect("stop playback");
    network.shutdown().expect("stop desktop host transport");
    actor.shutdown().expect("actor shutdown");
    eprintln!("done.");
}

fn stage_tone_source(
    temp: &TempDir,
    registry: &SelectedSourceRegistry,
    source_id: &str,
    frequency_hz: f64,
    seconds: u32,
) -> AudioSourceDescriptor {
    let source_path = temp.path().join(format!("{source_id}.wav"));
    fs::write(&source_path, tone_pcm_wav(frequency_hz, seconds)).expect("write source");
    let canonical_path = fs::canonicalize(&source_path).expect("canonical source");
    let byte_length = fs::metadata(&canonical_path).expect("metadata").len();
    let descriptor = AudioSourceDescriptor::new(
        format!("desktop-block-playback-manual-{source_id}"),
        format!("{source_id}.wav"),
        Some(byte_length),
        None,
    )
    .expect("descriptor");
    registry
        .replace(InspectedAudioSource::from_staged(
            descriptor.clone(),
            canonical_path,
            AudioContainer::Wav,
        ))
        .expect("register staged source");
    descriptor
}

fn tone_pcm_wav(frequency_hz: f64, seconds: u32) -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let frame_count = sample_rate * seconds;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frame_count {
        let time = f64::from(index) / f64::from(sample_rate);
        let sample = (time * frequency_hz * std::f64::consts::TAU).sin() * 12_000.0;
        #[allow(clippy::cast_possible_truncation)]
        let sample = sample as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn pcm_wav() -> Vec<u8> {
    let sample_rate = 44_100_u32;
    let channels = 1_u16;
    let frame_count = 4_410_u32;
    let data_bytes = frame_count * 2;
    let mut bytes = Vec::with_capacity(usize::try_from(data_bytes + 44).expect("capacity"));
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frame_count {
        let sample = if index % 64 < 32 {
            8_000_i16
        } else {
            -8_000_i16
        };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// Finds a real, currently-active private-LAN IPv4 interface on this machine,
/// mirroring `network_tests.rs`'s bind-conflict test -- a real production
/// socket bind requires an address genuinely assigned to a local interface,
/// so this cannot be faked the way `network_tests.rs`'s simulated-transport
/// tests fake interface records.
fn real_private_lan_address() -> Option<(String, u32, Ipv4Addr)> {
    let interfaces = netdev::get_interfaces();
    let system_interface = interfaces.into_iter().find(|interface| {
        interface.is_up()
            && (interface.is_running() || interface.is_oper_up())
            && !interface.is_loopback()
            && !interface.is_tun()
            && !interface.is_point_to_point()
            && interface.default
            && interface.ipv4_addrs().iter().any(Ipv4Addr::is_private)
    })?;
    let address = system_interface
        .ipv4_addrs()
        .into_iter()
        .find(Ipv4Addr::is_private)?;
    Some((system_interface.name, system_interface.index, address))
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

impl super::network::NetworkInterfaceProvider for FixedInterfaceProvider {
    fn interfaces(&self) -> Result<Vec<InterfaceRecord>, super::network::DesktopNetworkError> {
        Ok(vec![self.record.clone()])
    }
}

fn submit(
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

fn next_transport_effect(receiver: &Receiver<CoreNotification>) -> TransportEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for transport effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn wait_snapshot(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&CoreSnapshot) -> bool,
) -> CoreSnapshot {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for actor state"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_hello(listener: &mut dyn ListenerTransportNode) {
    wait_for_control(listener, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
}

fn wait_for_control(
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

fn wait_for_audio(
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

fn wait_for_sync_response(
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
