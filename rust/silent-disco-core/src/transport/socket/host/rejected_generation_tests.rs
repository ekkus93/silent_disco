//! A rejected legacy TCP generation must not be resurrected by delayed
//! approval or receive a later authorized generation's playback data.
use std::sync::Arc;
use std::time::Duration;

use crate::protocol::{ControlMessage, JoinRejection, ProtocolFrame};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportErrorKind, TransportFactory,
};
use super::SocketHostTransport;

#[test]
fn rejected_generation_cannot_be_approved_later_or_receive_replayed_pcm() {
    let session = id_session("rejected-generation-playback-isolation");
    let device = id_device("rejected-generation-id");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production host");
    let mut rejected = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect first TCP generation");
    rejected
        .send_control(&join_request(&session, &device, "Rejected generation"))
        .expect("send initial join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    let denial = ControlMessage::JoinRejection(JoinRejection {
        session_id: session.clone(),
        listener_id: device.clone(),
        reason: "Host declined this attempt".to_owned(),
    });
    assert_eq!(
        host.send_pending_control(&device, &denial)
            .expect("deliver explicit rejection")
            .report
            .successful_peers,
        1,
    );
    wait_for_frame(rejected.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(denial.clone())
    });
    assert!(
        host.authorize_peer(&device, rejected.local_routes()).is_err(),
        "a delayed approval must not revive the rejected TCP generation",
    );
    assert!(
        host.send_pending_control(&device, &denial).is_err(),
        "a rejected generation must not receive another admission decision",
    );

    // Reconnecting with the same unverified legacy ID requires a fresh
    // explicit decision, and must not inherit the rejected TCP route.
    let mut accepted = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("connect replacement generation");
    accepted
        .send_control(&join_request(&session, &device, "New generation"))
        .expect("send new join");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, accepted.local_routes())
        .expect("authorize only the fresh generation");
    wait_for_authorized(&mut host, &device);

    let start = stream_start(&session, 1_000);
    assert_eq!(
        host.broadcast_control(&start)
            .expect("send authoritative Start")
            .report
            .successful_peers,
        1,
    );
    wait_for_frame(accepted.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(start.clone())
    });
    let pcm = audio_frame_at(&session, 0, 1_000);
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("send production UDP PCM")
            .report
            .successful_peers,
        1,
    );
    wait_for_frame(accepted.as_mut(), TransportChannel::Audio, |frame| {
        frame == &pcm
    });
    let error = rejected
        .recv_event(Duration::from_millis(200))
        .expect_err("rejected socket must not receive Start or UDP PCM");
    assert_eq!(error.kind, TransportErrorKind::Timeout);
    assert_eq!(host.routes.lock().expect("routes").len(), 1);

    rejected.shutdown().expect("close rejected socket");
    accepted.shutdown().expect("close approved socket");
    host.shutdown().expect("close host");
}
