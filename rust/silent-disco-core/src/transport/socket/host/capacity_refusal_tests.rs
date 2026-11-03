//! Real TCP/UDP admission-capacity refusal and recovery. These legacy-v3
//! tests establish bounded visible failure, not authenticated transport.
use std::sync::Arc;
use std::time::Duration;

use crate::protocol::{ControlMessage, ProtocolFrame};
use crate::transport::test_support::{
    id_device, id_session, join_request, stream_start, wait_for_authorized, wait_for_control_from,
    wait_for_frame, wait_for_host_event, wait_until,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ManualTransportClock,
    SocketTransportFactory, TransportChannel, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

#[test]
fn full_host_visibly_refuses_real_join_without_starving_playback_and_recovers() {
    let session = id_session("capacity-refusal-recovery");
    let first_id = id_device("capacity-listener-first");
    let refused_id = id_device("capacity-listener-refused");
    let recovered_id = id_device("capacity-listener-recovered");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut config = HostTransportConfig::loopback(session.clone());
    config.max_peers = 1;
    config.operation_timeout = Duration::from_secs(1);
    let mut host = SocketHostTransport::bind(config, clock.clone()).expect("bind host");
    let factory = SocketTransportFactory;

    let mut first = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), first_id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("first TCP peer");
    first
        .send_control(&join_request(&session, &first_id, "Current listener"))
        .expect("first join");
    wait_for_control_from(&mut host, &first_id, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&first_id, first.local_routes())
        .expect("authorize first listener");
    wait_for_authorized(&mut host, &first_id);

    let mut refused = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), refused_id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("capacity-limited TCP peer");
    refused
        .send_control(&join_request(&session, &refused_id, "Rejected listener"))
        .expect("capacity join");
    wait_for_frame(refused.as_mut(), TransportChannel::Control, |frame| {
        matches!(
            frame,
            ProtocolFrame::Control(ControlMessage::JoinRejection(rejection))
                if rejection.listener_id == refused_id
                    && rejection.reason.contains("capacity")
        )
    });
    assert_eq!(host.counters().accepted_control_connections, 1);
    assert_eq!(host.counters().delivery_failures, 1);
    assert_eq!(host.counters().busy_admission_refusals, 1);

    // Repeated real TCP joins while the slot remains occupied must not
    // accumulate admitted peers or strand an existing playback connection.
    // Every attempt waits for its own refusal before starting the next one,
    // so this also exercises worker-permit reuse and finished-handle reaping.
    for attempt in 0..24 {
        let churn_id = id_device(&format!("capacity-churn-{attempt}"));
        let mut churn = factory
            .connect_listener(
                ListenerTransportConfig::loopback(
                    session.clone(),
                    churn_id.clone(),
                    host.endpoint(),
                ),
                clock.clone(),
            )
            .expect("connect excess TCP listener");
        churn
            .send_control(&join_request(&session, &churn_id, "Excess listener"))
            .expect("send excess join");
        wait_for_frame(churn.as_mut(), TransportChannel::Control, |frame| {
            matches!(
                frame,
                ProtocolFrame::Control(ControlMessage::JoinRejection(rejection))
                    if rejection.listener_id == churn_id
                        && rejection.reason.contains("capacity")
            )
        });
        drop(churn.shutdown());
    }
    assert_eq!(host.counters().accepted_control_connections, 1);
    assert_eq!(host.counters().delivery_failures, 25);
    assert_eq!(host.counters().busy_admission_refusals, 25);
    assert!(
        host.peers.lock().expect("peer registry").len() == 1,
        "refused socket must never consume a registered peer slot"
    );

    let start = stream_start(&session, 1_000);
    host.broadcast_control(&start)
        .expect("existing authorized peer still receives controls");
    wait_for_frame(first.as_mut(), TransportChannel::Control, |frame| {
        matches!(
            frame,
            ProtocolFrame::Control(ControlMessage::StreamStart(_))
        )
    });

    drop(refused.shutdown());
    first.shutdown().expect("disconnect original listener");
    drop(wait_for_host_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::PeerDisconnected { peer, .. }
                if peer.device_id.as_ref() == Some(&first_id)
        )
    }));
    wait_until(Duration::from_secs(5), || {
        host.peers.lock().expect("peer registry").is_empty()
    });

    let mut recovered = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session.clone(),
                recovered_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("capacity restored for fresh listener");
    recovered
        .send_control(&join_request(&session, &recovered_id, "Recovered listener"))
        .expect("recovered join");
    wait_for_control_from(&mut host, &recovered_id, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&recovered_id, recovered.local_routes())
        .expect("recovered listener authorized");
    wait_for_authorized(&mut host, &recovered_id);
    assert_eq!(host.counters().accepted_control_connections, 2);
    recovered.shutdown().expect("recovered peer shutdown");
    host.shutdown().expect("host shutdown");
}
