//! The listener side of the TLS control path, against a real host: a listener
//! that pins the host joins over TLS, receives its approval on the same channel,
//! sends UDP synchronization on its authorized route, and is seen to leave.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{DeviceId, MonotonicMillis, SessionId};
use crate::protocol::{ControlMessage, JoinApproval, ProtocolFrame, SyncRequest};
use crate::transport::test_support::{id_device, join_request};
use crate::transport::{
    HostTransportNode, ListenerTransportConfig, ListenerTransportNode, ManualTransportClock,
    SecureHostTarget, TransportChannel, TransportErrorKind, TransportEvent,
};

use super::SocketListenerTransport;
use super::secure_test_support::{host_with_secure, is_join_frame, key, spki_of, wait_for_event};

fn connect_tls_listener(
    endpoint: crate::runtime::NetworkEndpoint,
    host_spki: &[u8],
    session: &SessionId,
    device: &DeviceId,
    seed: u8,
) -> Result<SocketListenerTransport, crate::transport::TransportError> {
    let mut config = ListenerTransportConfig::loopback(session.clone(), device.clone(), endpoint);
    config.secure_host = Some(SecureHostTarget {
        host_spki: host_spki.to_vec(),
        signer: Arc::new(key(seed)),
    });
    SocketListenerTransport::connect(config, Arc::new(ManualTransportClock::new(0)))
}

fn wait_listener_event(
    listener: &SocketListenerTransport,
    mut wanted: impl FnMut(&TransportEvent) -> bool,
) -> TransportEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = listener.recv_event(remaining).expect("listener event");
        if wanted(&event) {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "expected listener event did not arrive"
        );
    }
}

#[test]
fn a_listener_pinned_to_the_host_joins_over_tls_and_receives_its_approval() {
    let session = SessionId::new("listener-tls-join").expect("session");
    let (mut host, host_spki) = host_with_secure(71, &session);
    let device = id_device("tls-listener-join");
    let listener = connect_tls_listener(host.endpoint(), &host_spki, &session, &device, 72)
        .expect("pinned listener connects");

    listener
        .send_control(&join_request(&session, &device, "TLS listener"))
        .expect("join sent");
    let joined = wait_for_event(&mut host, is_join_frame);
    let TransportEvent::FrameReceived { peer, .. } = joined else {
        panic!("join was not a frame event");
    };
    assert_eq!(peer.device_id.as_ref(), Some(&device));

    host.send_pending_control(
        &device,
        &ControlMessage::JoinApproval(JoinApproval {
            session_id: session.clone(),
            listener_id: device.clone(),
            trusted_for_future: false,
        }),
    )
    .expect("approval sent");
    let approval = wait_listener_event(&listener, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(_)),
                ..
            }
        )
    });
    let TransportEvent::FrameReceived { channel, .. } = approval else {
        panic!("approval was not a frame event");
    };
    assert_eq!(channel, TransportChannel::Control);
    host.shutdown().expect("shutdown");
}

#[test]
fn a_listener_pinned_to_another_host_key_never_connects() {
    let session = SessionId::new("listener-tls-pin").expect("session");
    let (host, _host_spki) = host_with_secure(73, &session);
    let other_spki = spki_of(74);
    let result = connect_tls_listener(
        host.endpoint(),
        &other_spki,
        &session,
        &id_device("tls-listener-wrong-pin"),
        75,
    );
    assert!(
        result.is_err(),
        "a listener pinned to another key connected"
    );
}

#[test]
fn a_tls_listener_sends_sync_on_its_authorized_route_and_the_host_sees_its_identity() {
    let session = SessionId::new("listener-tls-sync").expect("session");
    let (mut host, host_spki) = host_with_secure(76, &session);
    let device = id_device("tls-listener-sync");
    let listener = connect_tls_listener(host.endpoint(), &host_spki, &session, &device, 77)
        .expect("pinned listener connects");
    listener
        .send_control(&join_request(&session, &device, "TLS sync"))
        .expect("join sent");
    wait_for_event(&mut host, is_join_frame);
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize datagram route");
    crate::transport::test_support::wait_for_authorized(&mut host, &device);

    listener
        .send_sync_request(&SyncRequest {
            session_id: session.clone(),
            correlation_id: 501,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(10),
        })
        .expect("sync sent");
    let probe = wait_for_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                ..
            }
        )
    });
    let TransportEvent::FrameReceived { peer, frame, .. } = probe else {
        panic!("not a frame event");
    };
    assert_eq!(peer.device_id.as_ref(), Some(&device));
    assert!(
        matches!(frame, ProtocolFrame::SyncRequest(ref request) if request.correlation_id == 501)
    );
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_listener_shutdown_is_observed_by_the_host_as_a_departure() {
    let session = SessionId::new("listener-tls-leave").expect("session");
    let (mut host, host_spki) = host_with_secure(78, &session);
    let device = id_device("tls-listener-leave");
    let mut listener = connect_tls_listener(host.endpoint(), &host_spki, &session, &device, 79)
        .expect("pinned listener connects");
    listener
        .send_control(&join_request(&session, &device, "TLS leave"))
        .expect("join sent");
    wait_for_event(&mut host, is_join_frame);

    listener.shutdown().expect("listener shutdown");
    let departed = wait_for_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerDisconnected { .. })
    });
    let TransportEvent::PeerDisconnected { error, .. } = departed else {
        panic!("not a disconnect");
    };
    assert!(
        error
            .as_ref()
            .is_none_or(|error| error.kind != TransportErrorKind::Unauthorized),
        "a clean listener departure must not be reported as unauthorized"
    );
    host.shutdown().expect("shutdown");
}

#[test]
fn a_tls_listener_that_leaves_can_rejoin_with_the_same_device_and_be_approved_again() {
    let session = SessionId::new("listener-tls-rejoin").expect("session");
    let (mut host, host_spki) = host_with_secure(80, &session);
    let device = id_device("tls-listener-rejoin");

    let mut first = connect_tls_listener(host.endpoint(), &host_spki, &session, &device, 81)
        .expect("first connection");
    first
        .send_control(&join_request(&session, &device, "first"))
        .expect("first join sent");
    wait_for_event(&mut host, is_join_frame);
    first.shutdown().expect("first shutdown");
    wait_for_event(&mut host, |event| {
        matches!(event, TransportEvent::PeerDisconnected { .. })
    });

    let second = connect_tls_listener(host.endpoint(), &host_spki, &session, &device, 82)
        .expect("reconnect");
    second
        .send_control(&join_request(&session, &device, "second"))
        .expect("second join sent");
    let rejoined = wait_for_event(&mut host, is_join_frame);
    let TransportEvent::FrameReceived { peer, .. } = rejoined else {
        panic!("rejoin was not a frame event");
    };
    assert_eq!(peer.device_id.as_ref(), Some(&device));

    host.send_pending_control(
        &device,
        &ControlMessage::JoinApproval(JoinApproval {
            session_id: session.clone(),
            listener_id: device.clone(),
            trusted_for_future: false,
        }),
    )
    .expect("second approval sent");
    wait_listener_event(&second, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(_)),
                ..
            }
        )
    });
    host.shutdown().expect("shutdown");
}
