//! FFI must preserve the TCP generation when forwarding a terminal event.
use super::super::types::FfiHostTransportEvent;
use super::map_event;
use silent_disco_core::domain::{DeviceId, MonotonicMillis};
use silent_disco_core::transport::{TransportEvent, TransportPeer};

#[test]
fn native_disconnect_retains_exact_tcp_address_for_android_pending_routes() {
    let event = TransportEvent::PeerDisconnected {
        peer: TransportPeer {
            device_id: Some(DeviceId::new("duplicate-legacy-id").expect("valid ID")),
            control_address: "127.0.0.1:12345".parse().expect("TCP address"),
        },
        error: None,
        received_at: MonotonicMillis::new(42),
    };
    assert_eq!(
        map_event(event),
        Some(FfiHostTransportEvent::PeerDisconnectedAt {
            control_address: "127.0.0.1:12345".to_owned(),
            listener_id: Some("duplicate-legacy-id".to_owned()),
            message: None,
        }),
    );
}
