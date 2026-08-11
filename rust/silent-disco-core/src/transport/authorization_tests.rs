use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::{DeliverySeverity, MonotonicMillis};
use crate::protocol::{ControlMessage, Heartbeat, Hello, PROTOCOL_MAGIC};

use super::test_support::{
    id_device, id_session, join_request, wait_for_control_from, wait_for_rejection, wait_until,
};
use super::{
    HostTransportConfig, ListenerTransportConfig, SocketTransportFactory, SystemTransportClock,
    TransportErrorKind, TransportFactory,
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
    listener.shutdown().expect("listener should stop");
    host.shutdown()
        .expect("host should stop under queue pressure");
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
