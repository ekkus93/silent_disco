use std::sync::Arc;

use crate::domain::MonotonicMillis;
use crate::protocol::{ProtocolFrame, encode_frame};

use super::test_support::{EVENT_TIMEOUT, audio_frame, id_device, id_session};
use super::{
    HostTransportConfig, ListenerTransportConfig, ManualTransportClock, TransportChannel,
    TransportErrorKind, TransportEvent, TransportFactory, VirtualTransportFactory,
    VirtualTransportNetwork,
};

/// Block 39.1 wire-boundary proof: virtual protocol traffic is queued as
/// canonical encoded bytes plus transport metadata. The recipient performs
/// the production `decode_frame` inside `recv_event`; a valid frame arrives
/// intact, while a checksum-corrupted payload fails at receive time with the
/// same protocol classification as the real socket transport.
#[test]
fn virtual_listener_decodes_raw_wire_bytes_at_receive_time() {
    let network = VirtualTransportNetwork::default();
    let factory = VirtualTransportFactory::new(network.clone());
    let session_id = id_session("virtual-raw-wire");
    let host_clock = Arc::new(ManualTransportClock::new(1_000));
    let listener_clock = Arc::new(ManualTransportClock::new(9_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            host_clock,
        )
        .expect("virtual host should bind");
    let device_id = id_device("virtual-raw-wire-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            listener_clock,
        )
        .expect("virtual listener should connect");

    let frame = audio_frame(&session_id, 41);
    let bytes = encode_frame(&frame).expect("audio frame should encode");
    network
        .inject_listener_wire_frame_for_test(
            host.endpoint(),
            &device_id,
            TransportChannel::Audio,
            bytes,
        )
        .expect("raw virtual wire frame should enqueue");
    let received = listener
        .recv_event(EVENT_TIMEOUT)
        .expect("recipient should decode the raw wire frame");
    assert!(matches!(
        received,
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            ref peer,
            frame: ProtocolFrame::Audio(ref datagram),
            received_at,
        } if datagram.sequence.get() == 41
            && received_at == MonotonicMillis::new(9_000)
            && peer.control_address.ip() == host.endpoint().address
            && peer.control_address.port() == host.endpoint().control_port
    ));

    let mut corrupted = encode_frame(&frame).expect("audio frame should encode again");
    *corrupted
        .last_mut()
        .expect("encoded audio frame should contain payload bytes") ^= 0xFF;
    network
        .inject_listener_wire_frame_for_test(
            host.endpoint(),
            &device_id,
            TransportChannel::Audio,
            corrupted,
        )
        .expect("corrupted bytes should still cross the virtual wire");
    let Err(decode_error) = listener.recv_event(EVENT_TIMEOUT) else {
        panic!("production receive decode must reject corrupted bytes");
    };
    assert_eq!(decode_error.kind, TransportErrorKind::Protocol);

    listener.shutdown().expect("virtual listener should stop");
    host.shutdown().expect("virtual host should stop");
}
