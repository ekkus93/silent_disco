use std::sync::Arc;
use std::time::Duration;

use crate::domain::{DeviceId, SessionId};

use super::{
    HostTransportConfig, ListenerTransportConfig, ManualTransportClock, TransportErrorKind,
    TransportEvent, TransportFactory, VirtualTransportFactory, VirtualTransportNetwork,
    VirtualUdpFaultConfig,
};

const EVENT_TIMEOUT: Duration = Duration::from_secs(1);

#[test]
fn disconnect_then_reconnect_obeys_the_virtual_clock_delay() {
    let session_id = SessionId::new("reconnect-delay").expect("session ID");
    let device_id = DeviceId::new("reconnect-delay-listener").expect("device ID");
    let factory = VirtualTransportFactory::new(VirtualTransportNetwork::default())
        .with_udp_faults(VirtualUdpFaultConfig::default())
        .with_reconnect_delay(500);
    let clock = Arc::new(ManualTransportClock::new(10_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("virtual host should bind");
    let listener_config = ListenerTransportConfig::loopback(
        session_id,
        device_id.clone(),
        host.endpoint(),
    );
    let mut listener = factory
        .connect_listener(listener_config.clone(), clock.clone())
        .expect("initial listener should connect");

    // Drain the host-side accepted event so the post-reconnect assertion is
    // not satisfied by the original connection.
    assert!(matches!(
        host.recv_event(EVENT_TIMEOUT)
            .expect("host should observe the initial peer"),
        TransportEvent::PeerAccepted { .. }
    ));

    host.disconnect_peer(&device_id)
        .expect("host should disconnect the listener");
    assert!(matches!(
        listener
            .recv_event(EVENT_TIMEOUT)
            .expect("listener should observe the disconnect"),
        TransportEvent::PeerDisconnected { .. }
    ));
    listener.shutdown().expect("disconnected listener should stop");

    let Err(too_early) = factory.connect_listener(listener_config.clone(), clock.clone()) else {
        panic!("reconnect before the virtual deadline must fail");
    };
    assert_eq!(too_early.kind, TransportErrorKind::Connect);
    assert!(too_early.message.contains("delayed until 10500ms"));

    clock.advance(499);
    let Err(still_early) = factory.connect_listener(listener_config.clone(), clock.clone()) else {
        panic!("reconnect one millisecond before the deadline must still fail");
    };
    assert_eq!(still_early.kind, TransportErrorKind::Connect);

    clock.advance(1);
    let mut reconnected = factory
        .connect_listener(listener_config, clock)
        .expect("reconnect at the exact virtual deadline should succeed");
    assert!(matches!(
        host.recv_event(EVENT_TIMEOUT)
            .expect("host should observe the reconnected peer"),
        TransportEvent::PeerAccepted { .. }
    ));

    reconnected.shutdown().expect("reconnected listener should stop");
    host.shutdown().expect("host should stop");
}

#[test]
fn zero_reconnect_delay_does_not_invent_a_backoff() {
    let session_id = SessionId::new("reconnect-zero").expect("session ID");
    let device_id = DeviceId::new("reconnect-zero-listener").expect("device ID");
    let factory = VirtualTransportFactory::new(VirtualTransportNetwork::default())
        .with_reconnect_delay(0);
    let clock = Arc::new(ManualTransportClock::new(3_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("virtual host should bind");
    let config = ListenerTransportConfig::loopback(session_id, device_id, host.endpoint());
    let mut first = factory
        .connect_listener(config.clone(), clock.clone())
        .expect("initial listener should connect");
    first.shutdown().expect("initial listener should stop");
    let mut second = factory
        .connect_listener(config, clock)
        .expect("zero-delay reconnect should succeed immediately");
    second.shutdown().expect("second listener should stop");
    host.shutdown().expect("host should stop");
}
