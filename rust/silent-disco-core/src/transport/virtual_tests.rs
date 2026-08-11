use std::sync::Arc;

use crate::domain::{DeliverySeverity, MonotonicMillis};
use crate::protocol::{ControlMessage, SyncRequest};

use super::test_support::{
    EVENT_TIMEOUT, audio_frame, id_device, id_session, join_request, wait_for_authorized,
    wait_for_control_from, wait_for_frame, wait_for_frame_from, wait_for_listener_event,
};
use super::{
    HostTransportConfig, ListenerTransportConfig, ManualTransportClock, TransportChannel,
    TransportEvent, TransportFactory, VirtualTransportFactory, VirtualTransportNetwork,
};

#[test]
fn virtual_transport_is_explicit_isolated_and_uses_injected_clock() {
    let network_a = VirtualTransportNetwork::default();
    let network_b = VirtualTransportNetwork::default();
    let factory_a = VirtualTransportFactory::new(network_a);
    let factory_b = VirtualTransportFactory::new(network_b);
    let session_a = id_session("virtual-a");
    let session_b = id_session("virtual-b");
    let clock_a = Arc::new(ManualTransportClock::new(1_000));
    let clock_b = Arc::new(ManualTransportClock::new(9_000));
    let mut host_a = factory_a
        .bind_host(
            HostTransportConfig::loopback(session_a.clone()),
            clock_a.clone(),
        )
        .expect("first virtual host should bind");
    let mut host_b = factory_b
        .bind_host(HostTransportConfig::loopback(session_b.clone()), clock_b)
        .expect("second isolated virtual host should bind the same synthetic ports");
    assert_eq!(host_a.endpoint(), host_b.endpoint());

    let device_a = id_device("virtual-listener-a");
    let mut listener_a = factory_a
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_a.clone(),
                device_a.clone(),
                host_a.endpoint(),
            ),
            clock_a.clone(),
        )
        .expect("virtual listener should connect only inside its network");
    let accepted = host_a
        .recv_event(EVENT_TIMEOUT)
        .expect("virtual host should receive accepted event");
    assert!(matches!(
        accepted,
        TransportEvent::PeerAccepted {
            received_at,
            ..
        } if received_at == MonotonicMillis::new(1_000)
    ));
    listener_a
        .send_control(&join_request(&session_a, &device_a, "Virtual Listener"))
        .expect("virtual join should exercise canonical protocol round trip");
    wait_for_control_from(&mut *host_a, &device_a, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host_a
        .authorize_peer(&device_a, listener_a.local_routes())
        .expect("virtual peer should authorize against exact routes");
    wait_for_authorized(&mut *host_a, &device_a);
    clock_a.advance(25);
    listener_a
        .send_sync_request(&SyncRequest {
            session_id: session_a.clone(),
            correlation_id: 77,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1_025),
        })
        .expect("authorized virtual listener should send sync request");
    let event = wait_for_frame_from(
        &mut *host_a,
        TransportChannel::Synchronization,
        &device_a,
        |_| true,
    );
    assert!(matches!(
        event,
        TransportEvent::FrameReceived {
            received_at,
            ..
        } if received_at == MonotonicMillis::new(1_025)
    ));

    assert_eq!(
        host_b
            .broadcast_audio(&audio_frame(&session_b, 1))
            .expect("isolated host with no listeners returns explicit zero-peer report")
            .report
            .severity,
        DeliverySeverity::ZeroPeers
    );
    listener_a.shutdown().expect("virtual listener should stop");
    host_a.shutdown().expect("first virtual host should stop");
    host_b.shutdown().expect("second virtual host should stop");
}

/// `received_at` records when the *recipient* observed an event, not when
/// the sender produced it -- a host-broadcast frame delivered to a
/// listener must be stamped with the listener's own clock, and a
/// host-initiated disconnect delivered to a listener likewise. Proven by
/// giving the host and listener genuinely independent clocks and moving
/// only the listener's forward: the stamped time must track it exactly,
/// even though the host's own clock never advances.
#[test]
fn virtual_transport_stamps_delivered_events_with_the_recipients_own_clock() {
    let network = VirtualTransportNetwork::default();
    let factory = VirtualTransportFactory::new(network);
    let session_id = id_session("virtual-recipient-clock");
    let host_clock = Arc::new(ManualTransportClock::new(1_000));
    let listener_clock = Arc::new(ManualTransportClock::new(50_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            host_clock,
        )
        .expect("virtual host should bind");

    let device_id = id_device("virtual-recipient-clock-listener");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            listener_clock.clone(),
        )
        .expect("virtual listener should connect with its own clock");

    listener
        .send_control(&join_request(
            &session_id,
            &device_id,
            "Recipient Clock Listener",
        ))
        .expect("join request should send");
    wait_for_control_from(&mut *host, &device_id, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("virtual peer should authorize");
    wait_for_authorized(&mut *host, &device_id);

    listener_clock.advance(2_500);

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("broadcast audio to the authorized listener");
    let event = wait_for_frame(&mut *listener, TransportChannel::Audio, |_| true);
    assert!(matches!(
        event,
        TransportEvent::FrameReceived { received_at, .. }
            if received_at == MonotonicMillis::new(52_500)
    ));

    host.disconnect_peer(&device_id)
        .expect("host should be able to disconnect the listener");
    let disconnect_event = wait_for_listener_event(&mut *listener, |event| {
        matches!(event, TransportEvent::PeerDisconnected { .. })
    });
    assert!(matches!(
        disconnect_event,
        TransportEvent::PeerDisconnected { received_at, .. }
            if received_at == MonotonicMillis::new(52_500)
    ));

    listener.shutdown().expect("virtual listener should stop");
    host.shutdown().expect("virtual host should stop");
}
