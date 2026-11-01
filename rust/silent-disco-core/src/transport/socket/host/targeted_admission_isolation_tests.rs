//! Real TCP/UDP admission routing: a targeted decision must not be sent to
//! another claimed listener or broadcast. Legacy v3 is not authenticated.
use std::sync::Arc;

use crate::protocol::{
    ControlMessage, Disconnect, JoinApproval, JoinRejection, ProtocolFrame, ResyncNotice,
};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportErrorKind,
    TransportFactory,
};
use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let id = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect real TCP/UDP listener");
    listener
        .send_control(&join_request(session, &id, name))
        .expect("send join");
    wait_for_control_from(host, &id, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    listener
}

#[test]
fn misdirected_admission_and_terminal_controls_preserve_independent_pcm_routes() {
    let session = id_session("targeted-admission-isolation");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let first_id = id_device("first-listener");
    let second_id = id_device("second-listener");
    let mut first = join(&mut host, &clock, &session, "first-listener");
    let mut second = join(&mut host, &clock, &session, "second-listener");
    let approval = ControlMessage::JoinApproval(JoinApproval {
        session_id: session.clone(),
        listener_id: second_id.clone(),
        trusted_for_future: false,
    });
    let rejection = ControlMessage::JoinRejection(JoinRejection {
        session_id: session.clone(),
        listener_id: second_id.clone(),
        reason: "Denied".to_owned(),
    });
    let disconnect = ControlMessage::Disconnect(Disconnect {
        session_id: session.clone(),
        listener_id: second_id.clone(),
        reason: "Disconnected".to_owned(),
    });
    let resync = ControlMessage::ResyncNotice(ResyncNotice {
        session_id: session.clone(),
        listener_id: second_id.clone(),
        reason: "Rebuild clock".to_owned(),
    });

    let before = host.counters().control_frames_sent;
    for control in [&approval, &rejection, &disconnect] {
        assert_eq!(
            host.send_pending_control(&first_id, control)
                .expect_err("wrong pending listener must be refused")
                .kind,
            TransportErrorKind::Unauthorized,
        );
    }
    assert_eq!(host.counters().control_frames_sent, before);
    host.send_pending_control(&second_id, &approval)
        .expect("correct session-only approval");
    wait_for_frame(second.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(approval.clone())
    });
    for (id, listener) in [(&first_id, &first), (&second_id, &second)] {
        host.authorize_peer(id, listener.local_routes())
            .expect("authorize matching UDP route");
    }
    wait_for_authorized(&mut host, &first_id);
    wait_for_authorized(&mut host, &second_id);
    let before = host.counters().control_frames_sent;
    for control in [&approval, &rejection, &disconnect, &resync] {
        assert_eq!(
            host.send_control(&first_id, control)
                .expect_err("wrong authorized listener must be refused")
                .kind,
            TransportErrorKind::Unauthorized,
        );
        assert_eq!(
            host.broadcast_control(control)
                .expect_err("listener-specific control cannot broadcast")
                .kind,
            TransportErrorKind::Unauthorized,
        );
    }
    assert_eq!(
        host.send_disconnect(&first_id, &disconnect)
            .expect_err("wrong disconnect cannot retire another route")
            .kind,
        TransportErrorKind::Unauthorized,
    );
    assert_eq!(host.counters().control_frames_sent, before);
    assert_eq!(host.authorized_routes().expect("routes").len(), 2);

    host.send_control(&second_id, &resync)
        .expect("correct targeted resync");
    wait_for_frame(second.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(resync.clone())
    });
    let start = stream_start(&session, 1_000);
    assert_eq!(
        host.broadcast_control(&start)
            .expect("broadcast Start")
            .report
            .successful_peers,
        2,
    );
    for listener in [&mut first, &mut second] {
        wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(start.clone())
        });
    }
    let pcm = audio_frame_at(&session, 0, 1_000);
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("independent UDP routes remain active")
            .report
            .successful_peers,
        2,
    );
    for listener in [&mut first, &mut second] {
        wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
            frame == &pcm
        });
    }
    first.shutdown().expect("close first");
    second.shutdown().expect("close second");
    host.shutdown().expect("close host");
}
