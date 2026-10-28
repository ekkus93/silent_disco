use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::{DeliverySeverity, MonotonicMillis};
use crate::protocol::{ControlMessage, Heartbeat, Hello, PROTOCOL_MAGIC, ProtocolFrame};

use super::test_support::{
    id_device, id_session, join_request, stream_start, stream_stop, wait_for_control_from,
    wait_for_frame, wait_for_host_event, wait_for_rejection, wait_until,
};
use super::{
    HostTransportConfig, ListenerTransportConfig, SocketTransportFactory, SystemTransportClock,
    TransportChannel, TransportErrorKind, TransportEvent, TransportFactory,
};

#[test]
fn socket_runtime_rejects_unauthorized_control_and_malformed_headers() {
    let session_id = id_session("rejection-session");
    let factory = SocketTransportFactory;
    let clock = Arc::new(SystemTransportClock::default());
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("host should bind");
    let endpoint = host.endpoint();
    let device_id = id_device("unauthorized-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), endpoint),
            clock,
        )
        .expect("listener should connect");
    listener
        .send_control(&join_request(
            &session_id,
            &device_id,
            "Unauthorized Listener",
        ))
        .expect("join request is the one pre-authorization control message");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    listener
        .send_control(&ControlMessage::Heartbeat(Heartbeat {
            session_id: session_id.clone(),
            listener_id: device_id.clone(),
            sent_at_elapsed_ms: MonotonicMillis::new(5),
        }))
        .expect("TCP write may succeed before host authorization check");
    wait_for_rejection(&mut *host, TransportErrorKind::Unauthorized);

    let address = SocketAddr::new(endpoint.address, endpoint.control_port);
    let mut raw = TcpStream::connect(address).expect("raw malformed client should connect");
    raw.write_all(&invalid_version_header())
        .expect("raw malformed header should be written");
    drop(raw.shutdown(Shutdown::Both));
    wait_for_rejection(&mut *host, TransportErrorKind::Protocol);

    let mut oversized = TcpStream::connect(address).expect("raw oversized client should connect");
    oversized
        .write_all(&oversized_control_header())
        .expect("oversized header should be written");
    drop(oversized.shutdown(Shutdown::Both));
    wait_for_rejection(&mut *host, TransportErrorKind::Protocol);

    let mut truncated = TcpStream::connect(address).expect("raw truncated client should connect");
    truncated
        .write_all(&invalid_version_header()[..8])
        .expect("partial header should be written");
    drop(truncated.shutdown(Shutdown::Both));
    wait_for_rejection(&mut *host, TransportErrorKind::Protocol);

    let counters = host.counters();
    assert!(counters.unauthorized_frames >= 1);
    assert!(counters.malformed_frames + counters.oversized_frames >= 3);
    listener.shutdown().expect("listener should stop");
    host.shutdown().expect("host should stop");
}

#[test]
fn bounded_event_queue_reports_pressure_and_zero_peer_delivery_is_not_success() {
    let session_id = id_session("pressure-session");
    let factory = SocketTransportFactory;
    let mut config = HostTransportConfig::loopback(session_id.clone());
    config.event_queue_capacity = 1;
    let clock = Arc::new(SystemTransportClock::default());
    let mut host = factory
        .bind_host(config, clock.clone())
        .expect("host should bind with a one-event queue");
    let zero = host
        .broadcast_control(&ControlMessage::Hello(Hello {
            session_id: session_id.clone(),
            session_name: "Pressure Session".to_owned(),
            host_name: "Pressure Host".to_owned(),
            approval_required: false,
        }))
        .expect("zero-peer broadcast still returns explicit accounting");
    assert_eq!(zero.report.intended_peers, 0);
    assert_eq!(zero.report.severity, DeliverySeverity::ZeroPeers);

    let device_id = id_device("pressure-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("pressure listener should connect");
    listener
        .send_control(&join_request(&session_id, &device_id, "Pressure Listener"))
        .expect("join write should complete");
    wait_until(Duration::from_secs(2), || {
        host.counters().queue_overflows > 0
    });
    assert!(host.counters().queue_overflows > 0);
    assert!(host.counters().delivery_failures > 0);

    // Real TCP loopback proof: a join rejected by the full host event queue
    // must be answered on the wire. The listener must never wait forever
    // for a manual approval that the host actor cannot possibly receive.
    let rejection = wait_for_frame(&mut *listener, TransportChannel::Control, |frame| {
        matches!(
            frame,
            ProtocolFrame::Control(ControlMessage::JoinRejection(_))
        )
    });
    match rejection {
        TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::JoinRejection(value)),
            ..
        } => {
            assert_eq!(value.listener_id, device_id);
            assert!(value.reason.contains("Host busy"));
        }
        _ => panic!("expected explicit busy rejection"),
    }

    // The only queued host event before admission was PeerAccepted. No
    // JoinRequest may leak into the actor after the refusal was delivered.
    for _ in 0..4 {
        if let Ok(TransportEvent::FrameReceived {
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            ..
        }) = host.recv_event(Duration::from_millis(30))
        {
            panic!("refused join was also delivered to the host actor");
        }
    }
    listener.shutdown().expect("listener should stop");
    host.shutdown()
        .expect("host should stop under queue pressure");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end real-socket pressure, refusal, recovery and Stop scenario"
)]
fn real_socket_multi_peer_pressure_refuses_busy_joins_then_recovers_control_delivery() {
    let session_id = id_session("multi-peer-pressure-session");
    let factory = SocketTransportFactory;
    let clock = Arc::new(SystemTransportClock::default());
    let mut config = HostTransportConfig::loopback(session_id.clone());
    config.event_queue_capacity = 2;
    let mut host = factory
        .bind_host(config, clock.clone())
        .expect("host binds with two-event capacity");
    let endpoint = host.endpoint();
    let first_id = id_device("pressure-first");
    let second_id = id_device("pressure-second");
    let mut first = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), first_id.clone(), endpoint),
            clock.clone(),
        )
        .expect("first listener connects");
    let mut second = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), second_id.clone(), endpoint),
            clock.clone(),
        )
        .expect("second listener connects");

    // The two PeerAccepted notifications occupy both slots before either
    // client sends a join. Both joins must get explicit wire-level refusals,
    // never a phantom pending approval in the host actor.
    wait_until(Duration::from_secs(2), || {
        host.counters().accepted_control_connections >= 2
    });
    first
        .send_control(&join_request(&session_id, &first_id, "First"))
        .expect("first join writes");
    second
        .send_control(&join_request(&session_id, &second_id, "Second"))
        .expect("second join writes");
    for listener in [&mut *first, &mut *second] {
        let rejected = wait_for_frame(listener, TransportChannel::Control, |frame| {
            matches!(
                frame,
                ProtocolFrame::Control(ControlMessage::JoinRejection(_))
            )
        });
        assert!(
            matches!(
                rejected,
                TransportEvent::FrameReceived {
                    frame: ProtocolFrame::Control(ControlMessage::JoinRejection(value)),
                    ..
                } if value.reason.contains("Host busy")
            ),
            "every saturated join must be explicitly refused"
        );
    }
    wait_until(Duration::from_secs(2), || {
        host.counters().closed_control_connections >= 2
    });
    assert!(host.counters().delivery_failures >= 2);
    while let Ok(event) = host.recv_event(Duration::from_millis(100)) {
        assert!(
            !matches!(
                event,
                TransportEvent::FrameReceived {
                    frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
                    ..
                }
            ),
            "a refused join must never appear later in the host actor"
        );
    }
    first.shutdown().expect("first listener shutdown");
    second.shutdown().expect("second listener shutdown");

    // Once the consumer drains the queue, a fresh real TCP peer can join
    // and receive authoritative playback controls despite later telemetry
    // saturation. Congestion must not permanently poison admission.
    let recovered_id = id_device("pressure-recovered");
    let mut recovered = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), recovered_id.clone(), endpoint),
            clock,
        )
        .expect("recovered listener connects");
    drop(wait_for_host_event(&mut *host, |event| {
        matches!(event, TransportEvent::PeerAccepted { .. })
    }));
    recovered
        .send_control(&join_request(&session_id, &recovered_id, "Recovered"))
        .expect("recovered join writes");
    wait_for_control_from(&mut *host, &recovered_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&recovered_id, recovered.local_routes())
        .expect("recovered peer authorizes");
    drop(wait_for_host_event(&mut *host, |event| {
        matches!(event, TransportEvent::PeerAuthorized { .. })
    }));

    for index in 0..64 {
        recovered
            .send_control(&ControlMessage::Heartbeat(Heartbeat {
                session_id: session_id.clone(),
                listener_id: recovered_id.clone(),
                sent_at_elapsed_ms: MonotonicMillis::new(index),
            }))
            .expect("heartbeat writes under pressure");
    }
    wait_until(Duration::from_secs(2), || {
        host.counters().queue_overflows >= 3
    });
    host.broadcast_control(&stream_start(&session_id, 1_000))
        .expect("start reaches authorized peer despite telemetry pressure");
    drop(wait_for_frame(
        &mut *recovered,
        TransportChannel::Control,
        |frame| {
            matches!(
                frame,
                ProtocolFrame::Control(ControlMessage::StreamStart(_))
            )
        },
    ));
    host.broadcast_control(&stream_stop(&session_id, 2_000))
        .expect("Stop reaches authorized peer despite telemetry pressure");
    drop(wait_for_frame(
        &mut *recovered,
        TransportChannel::Control,
        |frame| matches!(frame, ProtocolFrame::Control(ControlMessage::Stop(_))),
    ));
    recovered.shutdown().expect("recovered listener shutdown");
    host.shutdown()
        .expect("host shutdown after pressure recovery");
}

fn invalid_version_header() -> [u8; 16] {
    let mut header = [0_u8; 16];
    header[..4].copy_from_slice(&PROTOCOL_MAGIC);
    header[4..6].copy_from_slice(&99_u16.to_be_bytes());
    header[6..8].copy_from_slice(&1_u16.to_be_bytes());
    header[10..12].copy_from_slice(&16_u16.to_be_bytes());
    header
}

fn oversized_control_header() -> [u8; 16] {
    let mut header = [0_u8; 16];
    header[..4].copy_from_slice(&PROTOCOL_MAGIC);
    header[4..6].copy_from_slice(&2_u16.to_be_bytes());
    header[6..8].copy_from_slice(&1_u16.to_be_bytes());
    header[10..12].copy_from_slice(&16_u16.to_be_bytes());
    header[12..16].copy_from_slice(&65_537_u32.to_be_bytes());
    header
}
