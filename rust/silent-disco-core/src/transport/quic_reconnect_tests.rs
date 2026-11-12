//! A listener that rejoins with the same key, then the host stops and shuts
//! down. Reproduces the sequence of the v4 reconnect device run (handoff
//! §48), where the phone saw neither the Stop nor the close after a rejoin.

use std::time::{Duration, Instant};

use super::quic_listener_transport::QuicListenerTransport;
use super::quic_listener_transport_tests::{admitted, host, identity, session};
use super::test_support::{stream_start, stream_stop, wait_for_host_event};
use super::{HostTransportNode, ListenerTransportNode, TransportEvent};
use crate::protocol::{ControlMessage, ProtocolFrame};

/// Same key in both connections, as the phone keeps its Keystore key.
const PHONE: u8 = 60;
/// How quickly a host shutdown must reach a connected listener. The QUIC
/// idle timeout (8 s) is what the phone fell back to in the device run.
const PROMPT_CLOSE: Duration = Duration::from_secs(2);

/// The next control message, skipping datagrams.
fn next_control_message(listener: &QuicListenerTransport) -> ControlMessage {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "no control message in time");
        if let Ok(TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(message),
            ..
        }) = listener.recv_event(remaining)
        {
            return message;
        }
    }
}

/// Waits for the listener's `PeerDisconnected`, returning how long it took.
fn time_to_disconnect(listener: &QuicListenerTransport) -> Duration {
    let started = Instant::now();
    loop {
        match listener.recv_event(Duration::from_secs(12)) {
            Ok(TransportEvent::PeerDisconnected { .. }) => return started.elapsed(),
            Ok(_) => {}
            Err(error) => panic!("no disconnect: {error}"),
        }
    }
}

fn stop_and_shut_down(
    mut host: super::quic_host_transport::QuicHostTransport,
    listener: &QuicListenerTransport,
) {
    let delivery = host
        .broadcast_control(&stream_stop(&session(), 9_000))
        .expect("stop broadcast");
    assert_eq!(
        (
            delivery.report.intended_peers,
            delivery.report.successful_peers
        ),
        (1, 1),
        "the Stop goes to exactly the rejoined connection"
    );
    assert!(
        matches!(next_control_message(listener), ControlMessage::Stop(_)),
        "the rejoined listener receives the Stop"
    );
    host.shutdown().expect("host shutdown");
    let waited = time_to_disconnect(listener);
    assert!(
        waited < PROMPT_CLOSE,
        "the rejoined listener saw the host close after {waited:?}, not promptly"
    );
}

#[test]
fn a_listener_that_rejoins_after_its_old_connection_closed_gets_the_stop_and_the_close() {
    let identity = identity();
    let mut host = host(&identity);
    let (mut first, _) = admitted(&mut host, &identity, PHONE);
    host.broadcast_control(&stream_start(&session(), 2_000))
        .expect("start");
    assert!(matches!(
        next_control_message(&first),
        ControlMessage::StreamStart(_)
    ));

    // The device run: the old connection is gone before the rejoin.
    first.shutdown().expect("first connection closes");
    wait_for_host_event(&mut host, |e| {
        matches!(e, TransportEvent::PeerDisconnected { .. })
    });

    let (second, _) = admitted(&mut host, &identity, PHONE);
    assert!(
        matches!(
            next_control_message(&second),
            ControlMessage::StreamStart(_)
        ),
        "the rejoined listener receives the cached StreamStart"
    );
    stop_and_shut_down(host, &second);
}

#[test]
fn a_listener_that_rejoins_while_its_old_connection_is_still_authorized_gets_the_stop_and_the_close()
 {
    let identity = identity();
    let mut host = host(&identity);
    let (first, _) = admitted(&mut host, &identity, PHONE);
    host.broadcast_control(&stream_start(&session(), 2_000))
        .expect("start");
    assert!(matches!(
        next_control_message(&first),
        ControlMessage::StreamStart(_)
    ));

    // The old connection is still authorized when the same key rejoins,
    // so the host must move the listener to the new connection.
    let (second, _) = admitted(&mut host, &identity, PHONE);
    assert!(matches!(
        next_control_message(&second),
        ControlMessage::StreamStart(_)
    ));
    stop_and_shut_down(host, &second);
    drop(first);
}
