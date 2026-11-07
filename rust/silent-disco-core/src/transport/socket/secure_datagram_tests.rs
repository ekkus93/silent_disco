//! UDP synchronization for a TLS-admitted listener. The authorized route is
//! the only source the host accepts for that listener: a datagram from any
//! other socket is refused even though the listener is joined over TLS.

use std::net::{SocketAddr, UdpSocket};

use crate::domain::{MonotonicMillis, SessionId};
use crate::protocol::{ProtocolFrame, SyncRequest, encode_frame};
use crate::transport::test_support::{id_device, wait_for_authorized};
use crate::transport::{
    HostTransportNode, ListenerDatagramRoutes, TransportChannel, TransportErrorKind, TransportEvent,
};

use super::secure_test_support::{host_with_secure, join_over_tls, wait_for_event};

fn loopback_socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").expect("loopback UDP socket")
}

fn probe(session: &SessionId, correlation_id: u64) -> ProtocolFrame {
    ProtocolFrame::SyncRequest(SyncRequest {
        session_id: session.clone(),
        correlation_id,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(100),
    })
}

#[test]
fn a_tls_joined_listener_sync_is_accepted_only_from_its_authorized_route() {
    let session = SessionId::new("datagram-tls").expect("session");
    let (mut host, host_spki) = host_with_secure(61, &session);
    let device = id_device("tls-sync-listener");
    let _listener = join_over_tls(&host, &host_spki, &session, &device, 62);
    wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(_),
                ..
            }
        )
    });

    let host_sync = SocketAddr::new(host.endpoint().address, host.endpoint().sync_port);
    let authorized = loopback_socket();
    let stranger = loopback_socket();
    let routes = ListenerDatagramRoutes {
        synchronization: authorized.local_addr().expect("address"),
        audio: loopback_socket().local_addr().expect("address"),
    };

    // Before authorization the host has no route for this listener.
    authorized
        .send_to(
            &encode_frame(&probe(&session, 400)).expect("encode"),
            host_sync,
        )
        .expect("send before authorization");
    let early = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Synchronization,
                ..
            }
        )
    });
    let TransportEvent::Rejected { error, .. } = early else {
        panic!("not a rejection");
    };
    assert_eq!(error.kind, TransportErrorKind::Unauthorized);

    host.authorize_peer(&device, routes).expect("authorize");
    wait_for_authorized(&mut host, &device);

    authorized
        .send_to(
            &encode_frame(&probe(&session, 401)).expect("encode"),
            host_sync,
        )
        .expect("send from the authorized route");
    let received = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                ..
            }
        )
    });
    let TransportEvent::FrameReceived { peer, frame, .. } = received else {
        panic!("not a frame");
    };
    assert_eq!(peer.device_id.as_ref(), Some(&device));
    assert!(
        matches!(frame, ProtocolFrame::SyncRequest(ref request) if request.correlation_id == 401),
        "the authorized probe was not delivered"
    );

    stranger
        .send_to(
            &encode_frame(&probe(&session, 402)).expect("encode"),
            host_sync,
        )
        .expect("send from a stranger");
    let refused = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Synchronization,
                ..
            }
        )
    });
    let TransportEvent::Rejected { error, .. } = refused else {
        panic!("not a rejection");
    };
    assert_eq!(error.kind, TransportErrorKind::Unauthorized);
    host.shutdown().expect("shutdown");
}
