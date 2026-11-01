use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::protocol::{ControlMessage, JoinApproval, ProtocolFrame};
use crate::transport::test_support::{id_device, id_session, join_request, wait_for_control_from};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportErrorKind, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn legacy_socket_cannot_issue_durable_join_approval_even_through_core_transport() {
    let session = id_session("legacy-durable-trust-denied");
    let device = id_device("legacy-trust-listener");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind legacy socket host");
    let mut listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock,
        )
        .expect("connect listener");
    listener
        .send_control(&join_request(&session, &device, "Unverified listener"))
        .expect("send join request");
    wait_for_control_from(&mut host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let approval = |trusted_for_future| {
        ControlMessage::JoinApproval(JoinApproval {
            session_id: session.clone(),
            listener_id: device.clone(),
            trusted_for_future,
        })
    };
    let sent_before = host.counters().control_frames_sent;
    let error = host
        .send_pending_control(&device, &approval(true))
        .expect_err("legacy control socket cannot assert durable trust");
    assert_eq!(error.kind, TransportErrorKind::Unauthorized);
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before,
        "rejected approval must never reach the TCP writer"
    );
    host.send_pending_control(&device, &approval(false))
        .expect("session-only approval remains valid");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing session-only join approval");
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(ControlMessage::JoinApproval(received)),
            ..
        } = listener.recv_event(remaining).expect("receive approval")
        {
            assert_eq!(received.listener_id, device);
            assert!(!received.trusted_for_future);
            break;
        }
    }
    // Once authorized, neither the targeted nor broadcast API may bypass
    // the same v3 no-durable-trust rule used by pending admission.
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize session-only listener");
    crate::transport::test_support::wait_for_authorized(&mut host, &device);
    let sent_before = host.counters().control_frames_sent;
    for result in [
        host.send_control(&device, &approval(true)),
        host.broadcast_control(&approval(true)),
    ] {
        assert_eq!(
            result
                .expect_err("durable approval cannot bypass pending gate")
                .kind,
            TransportErrorKind::Unauthorized
        );
    }
    assert_eq!(
        host.counters().control_frames_sent,
        sent_before,
        "neither path may write a forged durable-trust approval"
    );
    host.send_control(&device, &approval(false))
        .expect("targeted session-only approval remains valid");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "missing targeted session-only approval"
        );
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(ControlMessage::JoinApproval(received)),
            ..
        } = listener
            .recv_event(remaining)
            .expect("receive targeted approval")
        {
            assert_eq!(received.listener_id, device);
            assert!(!received.trusted_for_future);
            break;
        }
    }
    listener.shutdown().expect("close listener");
    host.shutdown().expect("close host");
}
