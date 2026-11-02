//! A failed authoritative listener TCP write must retire its generation.
//! Real TCP/UDP loopback coverage; no physical-device claim.
use super::super::host::SocketHostTransport;
use super::SocketListenerTransport;
use crate::domain::{DeviceId, MonotonicMillis, SessionId};
use crate::protocol::{ControlMessage, ProtocolFrame, ResyncNotice, SyncRequest};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame, wait_for_host_event,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, TransportChannel, TransportErrorKind, TransportEvent,
};
use std::net::Shutdown;
use std::sync::Arc;
use std::sync::atomic::Ordering;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &SessionId,
    name: &str,
) -> SocketListenerTransport {
    let device = id_device(name);
    let listener = SocketListenerTransport::connect(
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

fn sync_request(session: &SessionId) -> SyncRequest {
    SyncRequest {
        session_id: session.clone(),
        correlation_id: 42,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(0),
    }
}

#[test]
fn failed_listener_tcp_writer_retires_udp_sync_and_allows_independent_reconnect() {
    let session = id_session("listener-writer-retirement");
    let failed_id: DeviceId = id_device("failed-listener");
    let healthy_id = id_device("healthy-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production host");
    let mut failed = join(&mut host, &clock, &session, "failed-listener");
    let mut healthy = join(&mut host, &clock, &session, "healthy-listener");

    let start = stream_start(&session, 1_000);
    assert_eq!(
        host.broadcast_control(&start)
            .expect("start both")
            .report
            .successful_peers,
        2
    );
    for listener in [&mut failed as &mut dyn ListenerTransportNode, &mut healthy] {
        wait_for_frame(listener, TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(start.clone())
        });
    }

    // Half-close the actual listener TCP writer, not a mocked sender. Any
    // failed control delivery must terminate this admission generation.
    failed
        .control_stream
        .lock()
        .expect("listener control stream")
        .shutdown(Shutdown::Write)
        .expect("half-close listener TCP writer");
    let resync = ControlMessage::ResyncNotice(ResyncNotice {
        session_id: session.clone(),
        listener_id: failed_id.clone(),
        reason: "request timeline recovery".to_owned(),
    });
    assert!(
        failed.send_control(&resync).is_err(),
        "failed TCP writer must report its error"
    );
    assert!(
        failed.stop.load(Ordering::Acquire),
        "failed control delivery must retire the entire listener generation"
    );
    assert_eq!(
        failed
            .send_control(&resync)
            .expect_err("retired control")
            .kind,
        TransportErrorKind::ShuttingDown
    );
    assert_eq!(
        failed
            .send_sync_request(&sync_request(&session))
            .expect_err("retired UDP sync")
            .kind,
        TransportErrorKind::ShuttingDown
    );

    // The old TCP reader eventually removes its authorized UDP route.
    drop(wait_for_host_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerDisconnected { peer, .. }
            if peer.device_id.as_ref() == Some(&failed_id))
    }));
    let routes = host.authorized_routes().expect("authorized routes");
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].0, healthy_id);
    let pcm = audio_frame_at(&session, 0, 1_000);
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("healthy PCM")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(&mut healthy, TransportChannel::Audio, |frame| frame == &pcm);

    failed.shutdown().expect("close failed generation");
    let mut replacement = join(&mut host, &clock, &session, "failed-listener");
    wait_for_frame(&mut replacement, TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(start.clone())
    });
    assert_eq!(
        host.authorized_routes().expect("reconnected routes").len(),
        2
    );
    replacement.shutdown().expect("close replacement");
    healthy.shutdown().expect("close healthy");
    host.shutdown().expect("close host");
}

#[test]
fn explicitly_closed_listener_cannot_send_control_or_udp_sync() {
    let session = id_session("closed-listener-sends");
    let device = id_device("closed-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let mut listener = SocketListenerTransport::connect(
        ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
        clock,
    )
    .expect("connect listener");
    listener.shutdown().expect("close listener");
    assert_eq!(
        listener
            .send_control(&join_request(&session, &device, "closed-listener"))
            .expect_err("closed TCP send")
            .kind,
        TransportErrorKind::ShuttingDown
    );
    assert_eq!(
        listener
            .send_sync_request(&sync_request(&session))
            .expect_err("closed UDP sync")
            .kind,
        TransportErrorKind::ShuttingDown
    );
    host.shutdown().expect("close host");
}
