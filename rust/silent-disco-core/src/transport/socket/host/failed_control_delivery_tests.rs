//! Production TCP writer failures must revoke the corresponding admission
//! or audio route. These tests half-close the host's actual TCP writer socket
//! rather than calling the retirement helper directly.
use std::net::Shutdown;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::domain::{DeviceId, SessionId};
use crate::protocol::{
    ControlMessage, Disconnect, Hello, JoinApproval, JoinRejection, ProtocolFrame, ResyncNotice,
};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, stream_stop,
    wait_for_authorized, wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportFactory,
};

use super::SocketHostTransport;
use super::peer::PeerState;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect real TCP/UDP listener");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("send join");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(host, &device);
    listener
}

fn fail_next_tcp_write(peer: &PeerState) {
    peer.shutdown_stream
        .lock()
        .expect("TCP writer socket")
        .shutdown(Shutdown::Write)
        .expect("half-close host TCP writer");
    assert!(peer.active.load(Ordering::Acquire));
}

fn expect_only_healthy_route(host: &SocketHostTransport, healthy: &DeviceId) {
    let routes = host.authorized_routes().expect("authorized routes");
    assert_eq!(routes.len(), 1, "failed TCP writer must lose UDP access");
    assert_eq!(&routes[0].0, healthy);
}

#[test]
fn failed_targeted_stop_retires_only_that_peer_and_preserves_other_pcm_route() {
    let session = id_session("targeted-stop-writer-failure");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let failed_id = id_device("targeted-failed");
    let healthy_id = id_device("targeted-healthy");
    let mut failed = join(&mut host, &clock, &session, "targeted-failed");
    let mut healthy = join(&mut host, &clock, &session, "targeted-healthy");
    let start = stream_start(&session, 1_000);
    host.broadcast_control(&start).expect("start both");
    for listener in [&mut failed, &mut healthy] {
        wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(start.clone())
        });
    }
    let failed_peer = host.peer_for_device(&failed_id).expect("failed route");
    fail_next_tcp_write(&failed_peer);
    assert!(
        host.send_control(&failed_id, &stream_stop(&session, 1_100))
            .is_err(),
        "the actual TCP writer must report failed targeted Stop"
    );
    assert!(!failed_peer.active.load(Ordering::Acquire));
    expect_only_healthy_route(&host, &healthy_id);

    // The unaffected listener continues to receive controls and UDP.
    let pcm = audio_frame_at(&session, 0, 1_000);
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("healthy UDP")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(healthy.as_mut(), TransportChannel::Audio, |frame| {
        frame == &pcm
    });
    let stop = stream_stop(&session, 1_200);
    assert_eq!(
        host.broadcast_control(&stop)
            .expect("healthy Stop")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(healthy.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(stop.clone())
    });
    assert_eq!(
        host.broadcast_audio(&audio_frame_at(&session, 1, 1_005))
            .expect("retired PCM")
            .report
            .intended_peers,
        0
    );
    failed.shutdown().expect("close failed listener");
    let mut replacement = join(&mut host, &clock, &session, "targeted-failed");
    assert!(!failed_peer.active.load(Ordering::Acquire));
    assert_eq!(host.authorized_routes().expect("replacement").len(), 2);
    replacement.shutdown().expect("close replacement");
    healthy.shutdown().expect("close healthy");
    host.shutdown().expect("close host");
}

#[test]
fn failed_recovery_snapshot_retires_udp_route_and_reconnect_replays_start() {
    let session = id_session("recovery-writer-failure");
    let device = id_device("recovering-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let mut old = join(&mut host, &clock, &session, "recovering-peer");
    let start = stream_start(&session, 1_000);
    host.broadcast_control(&start).expect("start");
    wait_for_frame(old.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(start.clone())
    });
    let peer = host.peer_for_device(&device).expect("old peer");
    fail_next_tcp_write(&peer);
    assert!(
        host.replay_cached_playback_to_peer(&device).is_err(),
        "failed recovery write must be visible to the caller"
    );
    assert!(!peer.active.load(Ordering::Acquire));
    assert!(host.authorized_routes().expect("routes").is_empty());

    old.shutdown().expect("old shutdown");
    let mut replacement = join(&mut host, &clock, &session, "recovering-peer");
    wait_for_frame(replacement.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(start.clone())
    });
    assert_eq!(host.authorized_routes().expect("routes").len(), 1);
    replacement.shutdown().expect("replacement shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn failed_pending_hello_cannot_become_a_ghost_approval_after_reconnect() {
    let session = id_session("pending-writer-failure");
    let device = id_device("pending-failed");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let mut old = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect pending peer");
    old.send_control(&join_request(&session, &device, "pending-failed"))
        .expect("join request");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    let peer = host.pending_peer_for_device(&device).expect("pending peer");
    fail_next_tcp_write(&peer);
    let hello = ControlMessage::Hello(Hello {
        session_id: session.clone(),
        session_name: "Pending approval".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });
    assert!(host.send_pending_control(&device, &hello).is_err());
    assert!(peer.admission_denied.load(Ordering::Acquire));
    assert!(!peer.active.load(Ordering::Acquire));
    assert!(
        host.authorize_peer(&device, old.local_routes()).is_err(),
        "failed admission control cannot later authorize a stale socket"
    );

    old.shutdown().expect("old pending shutdown");
    let mut replacement = join(&mut host, &clock, &session, "pending-failed");
    assert_eq!(host.authorized_routes().expect("routes").len(), 1);
    replacement.shutdown().expect("replacement shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn failed_admission_and_resync_writes_revoke_udp_and_allow_independent_reconnect() {
    // These are actual TCP writer failures, not calls to the retirement
    // helper. A missed admission or recovery control must never leave a
    // listener eligible for UDP while a healthy peer continues playback.
    for kind in ["hello", "approval", "rejection", "disconnect", "resync"] {
        let session = id_session(&format!("failed-authoritative-{kind}"));
        let clock = Arc::new(ManualTransportClock::new(0));
        let mut host = SocketHostTransport::bind(
            HostTransportConfig::loopback(session.clone()),
            clock.clone(),
        )
        .expect("bind host");
        let failed_name = format!("failed-{kind}");
        let healthy_name = format!("healthy-{kind}");
        let failed_id = id_device(&failed_name);
        let healthy_id = id_device(&healthy_name);
        let mut failed = join(&mut host, &clock, &session, &failed_name);
        let mut healthy = join(&mut host, &clock, &session, &healthy_name);
        let start = stream_start(&session, 1_000);
        host.broadcast_control(&start).expect("start both listeners");
        for listener in [&mut failed, &mut healthy] {
            wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
                frame == &ProtocolFrame::Control(start.clone())
            });
        }

        let control = match kind {
            "hello" => ControlMessage::Hello(Hello {
                session_id: session.clone(),
                session_name: "Recovery".to_owned(),
                host_name: "Host".to_owned(),
                approval_required: false,
            }),
            "approval" => ControlMessage::JoinApproval(JoinApproval {
                session_id: session.clone(),
                listener_id: failed_id.clone(),
                trusted_for_future: false,
            }),
            "rejection" => ControlMessage::JoinRejection(JoinRejection {
                session_id: session.clone(),
                listener_id: failed_id.clone(),
                reason: "Rejected".to_owned(),
            }),
            "disconnect" => ControlMessage::Disconnect(Disconnect {
                session_id: session.clone(),
                listener_id: failed_id.clone(),
                reason: "Disconnected".to_owned(),
            }),
            "resync" => ControlMessage::ResyncNotice(ResyncNotice {
                session_id: session.clone(),
                listener_id: failed_id.clone(),
                reason: "Rebuild timeline".to_owned(),
            }),
            _ => unreachable!("enumerated authoritative controls"),
        };
        let failed_peer = host.peer_for_device(&failed_id).expect("failed peer");
        fail_next_tcp_write(&failed_peer);
        assert!(
            host.send_control(&failed_id, &control).is_err(),
            "{kind}: writer failure must be reported"
        );
        assert!(
            !failed_peer.active.load(Ordering::Acquire),
            "{kind}: missed authoritative control must retire TCP and UDP"
        );
        expect_only_healthy_route(&host, &healthy_id);

        let pcm = audio_frame_at(&session, 0, 1_000);
        assert_eq!(
            host.broadcast_audio(&pcm)
                .expect("healthy PCM")
                .report
                .successful_peers,
            1,
            "{kind}: healthy listener remains eligible"
        );
        wait_for_frame(healthy.as_mut(), TransportChannel::Audio, |frame| {
            frame == &pcm
        });
        failed.shutdown().expect("close failed listener");
        let mut replacement = join(&mut host, &clock, &session, &failed_name);
        wait_for_frame(replacement.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(start.clone())
        });
        assert_eq!(host.authorized_routes().expect("routes").len(), 2);
        replacement.shutdown().expect("close replacement");
        healthy.shutdown().expect("close healthy listener");
        host.shutdown().expect("close host");
    }
}
