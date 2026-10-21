//! Pending control frames share the late-join replay ordering gate.
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{ControlMessage, Hello, ProtocolFrame};
use crate::transport::test_support::{
    id_device, id_session, join_request, stream_start, wait_for_authorized, wait_for_control_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn pending_hello_cannot_overtake_or_follow_authorization_replay() {
    let session_id = id_session("pending-hello-replay-order");
    let device_id = id_device("pending-hello-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Pending listener"))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = stream_start(&session_id, 1_000);
    host.broadcast_control(&start).expect("cache stream start");
    let hello = ControlMessage::Hello(Hello {
        session_id,
        session_name: "Pending admission".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });

    // Hold the exact mutex used by authorization and live broadcasts. The
    // pending sender must remain blocked until it can acquire this guard.
    let delivery_gate = Arc::clone(&host.control_delivery);
    let guard = delivery_gate.lock().expect("ordering mutex");
    // Move the Send-only host into the sender thread. Borrowing it across
    // threads would incorrectly require its event receiver to be Sync.
    let mut host = thread::scope(|scope| {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let send_device_id = device_id.clone();
        let send_hello = hello.clone();
        scope.spawn(move || {
            ready_tx.send(()).expect("signal sender start");
            let result = host.send_pending_control(&send_device_id, &send_hello);
            assert!(
                done_tx.send((host, result)).is_ok(),
                "return host and pending delivery result"
            );
        });
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pending sender starts");
        assert!(
            matches!(
                done_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "pending control delivery must hold the replay ordering gate"
        );
        drop(guard);
        let (host, result) = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("pending send completes");
        result.expect("pending Hello delivered");
        host
    });

    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    let expected = [hello, start];
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut received = Vec::new();
    while received.len() < expected.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing ordered admission controls");
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(message),
            ..
        } = listener
            .recv_event(remaining)
            .expect("listener control event")
        {
            received.push(message);
        }
    }
    assert!(
        received == expected,
        "pending Hello must precede cached StreamStart on the TCP channel"
    );

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}

#[test]
fn pending_control_is_rejected_after_authorization_replay() {
    let session_id = id_session("post-authorization-pending");
    let device_id = id_device("post-authorization-peer");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session_id.clone()),
        clock.clone(),
    )
    .expect("host binds");
    let factory = SocketTransportFactory;
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("listener connects");
    listener
        .send_control(&join_request(&session_id, &device_id, "Pending listener"))
        .expect("join request");
    wait_for_control_from(&mut host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });

    let start = stream_start(&session_id, 1_000);
    host.broadcast_control(&start).expect("cache stream start");
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(&mut host, &device_id);

    // Authorization already replayed StreamStart. A delayed pending Hello
    // must not follow it and regress the listener to a pre-admission state.
    let hello = ControlMessage::Hello(Hello {
        session_id,
        session_name: "Stale pending admission".to_owned(),
        host_name: "Host".to_owned(),
        approval_required: true,
    });
    assert!(
        host.send_pending_control(&device_id, &hello).is_err(),
        "a pending-only message must not be delivered after authorization"
    );

    listener.shutdown().expect("listener shutdown");
    host.shutdown().expect("host shutdown");
}
