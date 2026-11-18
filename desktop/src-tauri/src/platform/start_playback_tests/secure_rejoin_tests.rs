//! The v4 reconnect device run (handoff §48) through the real desktop host
//! runtime and actor: a listener rejoins with the same key, then the host
//! stops playback and shuts down. On the phone, neither the Stop nor the
//! close arrived after the rejoin; the core transport alone handles this
//! sequence correctly (`quic_reconnect_tests`), so this checks the layer
//! above it.

use super::fixtures::stage_long_source;
use super::harness::{
    TEST_TIMEOUT, real_private_lan_address, start_host_session_configured, submit, wait_for_audio,
    wait_for_control, wait_for_stream_start, wait_snapshot,
};
use crate::platform::invitation_identity::tests_support::fixed_identity_for_tests;
use crate::platform::network::DesktopHostNetworkControl;
use crate::platform::start_playback;
use p256::ecdsa::SigningKey;
use p256::pkcs8::EncodePublicKey;
use silent_disco_core::domain::DeviceId;
use silent_disco_core::protocol::{ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame};
use silent_disco_core::runtime::{
    CoreActorHandle, CoreCommand, CoreNotification, NetworkEndpoint, SessionAdvertisement,
    TransportEffect,
};
use silent_disco_core::transport::secure_identity::device_id_for_listener_key;
use silent_disco_core::transport::{
    ListenerTransportConfig, ListenerTransportNode, QuicTransportFactory, SecureHostTarget,
    SystemTransportClock, TransportErrorKind, TransportEvent, TransportFactory,
};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// The phone keeps one Keystore key across connections.
const PHONE_KEY: u8 = 70;
/// The phone fell back to the 8 s QUIC idle timeout; a real close is prompt.
const PROMPT_CLOSE: Duration = Duration::from_secs(2);

struct V4Host {
    actor: silent_disco_core::runtime::CoreActorRuntime,
    handle: CoreActorHandle,
    receiver: Receiver<CoreNotification>,
    advertisement: SessionAdvertisement,
    network: Arc<DesktopHostNetworkControl>,
    endpoint: NetworkEndpoint,
    host_spki: Vec<u8>,
    address: Ipv4Addr,
    registry: crate::platform::file_picker::SelectedSourceRegistry,
    _temp: TempDir,
}

fn start_v4_host() -> Option<V4Host> {
    let Some((interface_name, interface_index, address)) = real_private_lan_address() else {
        eprintln!("no private LAN interface on this host; skipping");
        return None;
    };
    let temp = TempDir::new().expect("temp");
    let (descriptor, registry) = stage_long_source(&temp);
    let tls = fixed_identity_for_tests(23)
        .host_tls_identity()
        .expect("host TLS identity");
    let host_spki = tls.spki.clone();
    let (actor, handle, receiver, advertisement, network, endpoint) =
        start_host_session_configured(descriptor, interface_name, interface_index, address, |n| {
            n.set_host_tls_identity(tls).expect("TLS identity");
            n.set_secure_transport(true).expect("secure transport on");
        });
    Some(V4Host {
        actor,
        handle,
        receiver,
        advertisement,
        network,
        endpoint,
        host_spki,
        address,
        registry,
        _temp: temp,
    })
}

/// Connects a v4 listener holding `PHONE_KEY`, joins, and approves it
/// through the real actor, as the desktop does for the phone.
fn join_and_approve_v4(host: &V4Host) -> Box<dyn ListenerTransportNode> {
    let key = SigningKey::from_slice(&[PHONE_KEY; 32]).expect("listener key");
    let spki = key
        .verifying_key()
        .to_public_key_der()
        .expect("spki")
        .as_bytes()
        .to_vec();
    let device: DeviceId = device_id_for_listener_key(&spki);
    let mut config = ListenerTransportConfig::loopback(
        host.advertisement.session_id.clone(),
        device.clone(),
        host.endpoint,
    );
    config.local_address = IpAddr::V4(host.address);
    config.secure_host = Some(SecureHostTarget {
        host_spki: host.host_spki.clone(),
        signer: Arc::new(key),
    });
    let mut listener = QuicTransportFactory
        .connect_listener(config, Arc::new(SystemTransportClock::default()))
        .expect("v4 listener connects to the desktop host");
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: host.advertisement.session_id.clone(),
            device: DeviceIdentity {
                device_id: device,
                display_name: "Rejoining listener".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("join request");
    let effect = approve_pending_join(host);
    host.network
        .dispatch_transport_effect(effect)
        .expect("dispatch approval");
    wait_for_control(&mut *listener, |m| {
        matches!(m, ControlMessage::JoinApproval(_))
    });
    wait_snapshot(&host.handle, |s| s.listeners.len() == 1);
    listener
}

/// Approves the pending join and returns its transport effect. While
/// playback runs, the stream keeps advancing the snapshot revision, so an
/// approval sampled a moment earlier can be refused as stale (the actor's
/// optimistic revision check); resubmit with a fresh revision until the
/// approval produces its effect.
fn approve_pending_join(host: &V4Host) -> TransportEffect {
    let deadline = Instant::now() + TEST_TIMEOUT;
    while Instant::now() < deadline {
        let joined = wait_snapshot(&host.handle, |s| !s.pending_join_requests.is_empty());
        submit(
            &host.handle,
            joined.revision.get(),
            CoreCommand::ApproveJoin {
                request_id: joined.pending_join_requests[0].request_id.clone(),
                remember_for_future: false,
            },
        );
        let attempt = Instant::now() + Duration::from_millis(500);
        while let Some(left) = attempt.checked_duration_since(Instant::now()) {
            match host.receiver.recv_timeout(left) {
                Ok(CoreNotification::TransportEffect(effect)) => return effect,
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
            }
        }
    }
    panic!("the join was never approved within {TEST_TIMEOUT:?}");
}

/// How long the listener takes to report the host's disconnect.
fn time_to_disconnect(listener: &mut dyn ListenerTransportNode) -> Duration {
    let started = Instant::now();
    loop {
        match listener.recv_event(Duration::from_secs(12)) {
            Ok(TransportEvent::PeerDisconnected { .. }) => return started.elapsed(),
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {
                panic!("no disconnect within 12 s")
            }
            Err(error) => panic!("listener failed: {error}"),
        }
    }
}

fn stop_and_shut_down(host: V4Host, mut listener: Box<dyn ListenerTransportNode>) {
    host.network.stop_playback().expect("stop playback");
    wait_for_control(&mut *listener, |m| matches!(m, ControlMessage::Stop(_)));
    host.network.shutdown().expect("host shutdown");
    let waited = time_to_disconnect(&mut *listener);
    assert!(
        waited < PROMPT_CLOSE,
        "the rejoined listener saw the host close after {waited:?}"
    );
    host.actor.shutdown().expect("actor shutdown");
}

#[test]
fn a_v4_listener_that_rejoins_after_disconnecting_receives_stop_and_a_prompt_close() {
    let Some(host) = start_v4_host() else { return };
    let mut first = join_and_approve_v4(&host);
    start_playback::start(&host.handle, &host.network, &host.registry).expect("start playback");
    wait_for_control(&mut *first, |m| matches!(m, ControlMessage::StreamStart(_)));

    // As in the device run, the old connection is gone before the rejoin.
    first.shutdown().expect("first connection closes");
    wait_snapshot(&host.handle, |s| s.listeners.is_empty());

    let mut second = join_and_approve_v4(&host);
    wait_for_control(&mut *second, |m| {
        matches!(m, ControlMessage::StreamStart(_))
    });
    stop_and_shut_down(host, second);
}

#[test]
fn a_v4_listener_that_rejoins_before_its_old_connection_ends_receives_stop_and_a_prompt_close() {
    let Some(host) = start_v4_host() else { return };
    let mut first = join_and_approve_v4(&host);
    start_playback::start(&host.handle, &host.network, &host.registry).expect("start playback");
    wait_for_control(&mut *first, |m| matches!(m, ControlMessage::StreamStart(_)));

    let mut second = join_and_approve_v4(&host);
    wait_for_control(&mut *second, |m| {
        matches!(m, ControlMessage::StreamStart(_))
    });
    stop_and_shut_down(host, second);
    drop(first);
}

/// Reads the listener until the host disconnects and reports whether a
/// Stop arrived first.
fn stop_arrives_before_disconnect(listener: &mut dyn ListenerTransportNode) -> bool {
    let mut stopped = false;
    loop {
        match listener.recv_event(Duration::from_secs(12)) {
            Ok(TransportEvent::PeerDisconnected { .. }) => return stopped,
            Ok(TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::Stop(_)),
                ..
            }) => stopped = true,
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {
                panic!("no disconnect within 12 s")
            }
            Err(error) => panic!("listener failed: {error}"),
        }
    }
}

/// The A54 run 11 defect: an operator who shuts the host down straight after
/// stopping left the Stop in the desktop broadcast queue, so the phone only
/// saw the connection close.
#[test]
fn a_host_shut_down_straight_after_stopping_still_delivers_the_stop() {
    let Some(host) = start_v4_host() else { return };
    let mut listener = join_and_approve_v4(&host);
    start_playback::start(&host.handle, &host.network, &host.registry).expect("start playback");
    wait_for_control(&mut *listener, |m| {
        matches!(m, ControlMessage::StreamStart(_))
    });

    host.network.stop_playback().expect("stop playback");
    host.network.shutdown().expect("host shutdown");

    assert!(
        stop_arrives_before_disconnect(&mut *listener),
        "the listener saw the host close without receiving Stop"
    );
    host.actor.shutdown().expect("actor shutdown");
}

/// Stop then Play again over v4 through the real desktop host (remediation
/// TODO §1, stop/replay; until now covered only over v3): the listener gets
/// the Stop for the first stream, then a fresh `StreamStart` whose audio
/// restarts at sequence 0 and never belongs to the stopped stream.
#[test]
fn a_v4_listener_gets_the_stop_then_a_fresh_stream_when_the_host_plays_again() {
    let Some(host) = start_v4_host() else { return };
    let mut listener = join_and_approve_v4(&host);
    start_playback::start(&host.handle, &host.network, &host.registry).expect("start playback");
    let first = wait_for_stream_start(&mut *listener);
    assert_eq!(wait_for_audio(&mut *listener).stream_id, first.stream_id);

    host.network.stop_playback().expect("stop playback");
    wait_snapshot(&host.handle, |s| {
        s.playback_state == silent_disco_core::domain::PlaybackState::Stopped
    });
    wait_for_control(
        &mut *listener,
        |m| matches!(m, ControlMessage::Stop(stop) if stop.stream_id == first.stream_id),
    );
    // Drain what was already in flight before the Stop; the next StreamStart
    // is the boundary after which no old audio may appear.
    while listener.recv_event(Duration::from_millis(25)).is_ok() {}

    start_playback::start(&host.handle, &host.network, &host.registry).expect("play again");
    let second = wait_for_stream_start(&mut *listener);
    assert_ne!(
        second.stream_id, first.stream_id,
        "playing again starts a new stream"
    );
    let audio = wait_for_audio(&mut *listener);
    assert_eq!(
        audio.stream_id, second.stream_id,
        "no audio from the stopped stream"
    );
    assert_eq!(
        audio.sequence.get(),
        0,
        "the new stream starts at sequence 0"
    );
    stop_and_shut_down(host, listener);
}

/// Owner decision 2026-11-18 (remediation TODO §1): revoking a connected
/// listener disconnects it at once. Through the real actor and the desktop's
/// v4 transport, the actor turns the revocation into a delivery-first
/// disconnect and the listener receives it with the revoked reason.
#[test]
fn a_revoked_v4_listener_is_disconnected_with_the_revoked_reason() {
    let Some(host) = start_v4_host() else { return };
    let mut listener = join_and_approve_v4(&host);
    let device = host.handle.current_snapshot().expect("snapshot").listeners[0]
        .device_id
        .clone();

    let revision = host
        .handle
        .current_snapshot()
        .expect("snapshot")
        .revision
        .get();
    submit(
        &host.handle,
        revision,
        CoreCommand::ListenerRevoked {
            listener_id: device.clone(),
        },
    );
    let effect = loop {
        match host.receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => break effect,
            Ok(_) => {}
            Err(error) => panic!("no disconnect effect: {error}"),
        }
    };
    host.network
        .dispatch_transport_effect(effect)
        .expect("dispatch disconnect");
    wait_for_control(
        &mut *listener,
        |m| matches!(m, ControlMessage::Disconnect(d) if d.listener_id == device && d.reason == "host_revoked_listener"),
    );
    host.network.shutdown().expect("host shutdown");
    host.actor.shutdown().expect("actor shutdown");
}
