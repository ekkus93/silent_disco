//! Real TCP admission and UDP synchronization isolation. Only the listener
//! that sent a clock probe receives its response; a retired or unknown route
//! cannot redirect another listener's clock/timestamp data.
use std::io::Write;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{DeviceId, MonotonicMillis};
use crate::protocol::{ControlMessage, ProtocolFrame, SyncRequest, SyncResponse, encode_frame};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame, wait_for_host_event,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerDatagramRoutes, ListenerTransportConfig,
    ListenerTransportNode, ManualTransportClock, SocketTransportFactory, TransportChannel,
    TransportErrorKind, TransportEvent, TransportFactory,
};

use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect real TCP and UDP listener");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("join request");
    wait_for_control_from(host, &device, |message| {
        matches!(message, crate::protocol::ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize datagram route");
    wait_for_authorized(host, &device);
    listener
}

fn expect_probe(host: &mut SocketHostTransport, device: &DeviceId, correlation: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "host missed the real UDP probe");
        if let TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization,
            frame: ProtocolFrame::SyncRequest(request),
            peer,
            ..
        } = host.recv_event(remaining).expect("host transport event")
        {
            assert_eq!(peer.device_id.as_ref(), Some(device));
            assert_eq!(request.correlation_id, correlation);
            break;
        }
    }
}

#[test]
fn real_udp_sync_replies_target_only_the_admitted_requester_and_retire_with_route() {
    let session = id_session("targeted-sync-two-listeners");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production host");
    let first_id = id_device("sync-first");
    let second_id = id_device("sync-second");
    let mut first = join(&mut host, &clock, &session, "sync-first");
    let mut second = join(&mut host, &clock, &session, "sync-second");

    let probe = SyncRequest {
        session_id: session.clone(),
        correlation_id: 101,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(200),
    };
    first
        .send_sync_request(&probe)
        .expect("send first UDP probe");
    expect_probe(&mut host, &first_id, 101);
    let second_received_before = second.counters().sync_datagrams_received;
    let response = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session.clone(),
        correlation_id: 101,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(200),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(205),
        t3_host_send_elapsed_ms: MonotonicMillis::new(207),
    });
    let sent_before = host.counters().sync_datagrams_sent;
    let delivery = host
        .send_sync_to_peer(&first_id, &response)
        .expect("send only to first listener");
    assert_eq!(delivery.report.intended_peers, 1);
    assert_eq!(delivery.report.successful_peers, 1);
    assert_eq!(host.counters().sync_datagrams_sent - sent_before, 1);
    wait_for_frame(first.as_mut(), TransportChannel::Synchronization, |frame| {
        frame == &response
    });
    // The second socket must never receive the first listener's correlation
    // or host timestamps, even though both are authorized in this session.
    match second.recv_event(Duration::from_millis(150)) {
        Err(error) if error.kind == TransportErrorKind::Timeout => {}
        Ok(TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization,
            ..
        }) => panic!("targeted reply leaked to another admitted listener"),
        Ok(_) => {}
        Err(error) => panic!("unexpected second listener error: {error}"),
    }
    assert_eq!(
        second.counters().sync_datagrams_received,
        second_received_before,
        "other listener must receive zero UDP synchronization datagrams"
    );

    assert!(
        host.send_sync_to_peer(&id_device("not-admitted"), &response)
            .is_err()
    );
    assert!(
        host.send_sync_to_peer(&first_id, &ProtocolFrame::SyncRequest(probe.clone()))
            .is_err(),
        "a targeted response must not accept a SyncRequest frame"
    );
    let mut wrong_session = response.clone();
    let ProtocolFrame::SyncResponse(ref mut reply) = wrong_session else {
        panic!("response fixture");
    };
    reply.session_id = id_session("other-session");
    assert!(host.send_sync_to_peer(&first_id, &wrong_session).is_err());

    host.disconnect_peer(&first_id).expect("retire first route");
    first.shutdown().expect("close first listener");
    assert!(
        host.send_sync_to_peer(&first_id, &response).is_err(),
        "retired route must not inherit another listener's clock reply"
    );
    let second_probe = SyncRequest {
        session_id: session.clone(),
        correlation_id: 202,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(300),
    };
    second
        .send_sync_request(&second_probe)
        .expect("send second UDP probe");
    expect_probe(&mut host, &second_id, 202);
    let second_response = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session.clone(),
        correlation_id: 202,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(300),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(303),
        t3_host_send_elapsed_ms: MonotonicMillis::new(305),
    });
    assert_eq!(
        host.send_sync_to_peer(&second_id, &second_response)
            .expect("second route remains authorized")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(
        second.as_mut(),
        TransportChannel::Synchronization,
        |frame| frame == &second_response,
    );
    second.shutdown().expect("close second listener");
    host.shutdown().expect("close host");
}

#[test]
fn admitted_udp_route_cannot_inject_host_sync_responses_or_keep_peer_alive_with_wrong_direction() {
    let session = id_session("host-sync-direction-real-udp");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind real TCP and UDP host");
    let mut healthy = join(&mut host, &clock, &session, "healthy-sync-listener");
    let adversary_id = id_device("sync-response-injector");

    // A raw TCP peer and its own UDP route bypass the listener API's
    // direction checks. The host must independently enforce protocol roles.
    let mut raw_control = TcpStream::connect(SocketAddr::new(
        host.endpoint().address,
        host.endpoint().control_port,
    ))
    .expect("connect raw TCP peer");
    raw_control
        .write_all(
            &encode_frame(&ProtocolFrame::Control(join_request(
                &session,
                &adversary_id,
                "Raw sync peer",
            )))
            .expect("encode raw join"),
        )
        .expect("send raw join");
    wait_for_control_from(&mut host, &adversary_id, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    let sync = UdpSocket::bind("127.0.0.1:0").expect("raw sync socket");
    let audio = UdpSocket::bind("127.0.0.1:0").expect("raw audio socket");
    host.authorize_peer(
        &adversary_id,
        ListenerDatagramRoutes {
            synchronization: sync.local_addr().expect("sync route"),
            audio: audio.local_addr().expect("audio route"),
        },
    )
    .expect("authorize raw route");
    wait_for_authorized(&mut host, &adversary_id);

    let forged = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session.clone(),
        correlation_id: 77,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(200),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(201),
        t3_host_send_elapsed_ms: MonotonicMillis::new(202),
    });
    let received_before = host.counters().sync_datagrams_received;
    let source = SocketAddr::new(host.endpoint().address, host.endpoint().sync_port);
    sync.send_to(
        &encode_frame(&forged).expect("encode forged response"),
        source,
    )
    .expect("inject response from authorized UDP port");
    let rejected = wait_for_host_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::Rejected {
                channel: TransportChannel::Synchronization,
                error,
                ..
            } if error.kind == TransportErrorKind::Protocol
        )
    });
    assert!(matches!(rejected, TransportEvent::Rejected { .. }));
    assert_eq!(
        host.counters().sync_datagrams_received,
        received_before,
        "host-only response must not count as a valid inbound probe"
    );

    // A legitimate request on that SAME admitted UDP route must still work:
    // reject the wrong frame direction, not the entire healthy connection.
    let probe = ProtocolFrame::SyncRequest(SyncRequest {
        session_id: session.clone(),
        correlation_id: 78,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(250),
    });
    sync.send_to(&encode_frame(&probe).expect("encode request"), source)
        .expect("send legitimate UDP probe");
    let observed = wait_for_host_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Synchronization,
                peer,
                frame: ProtocolFrame::SyncRequest(request),
                ..
            } if peer.device_id.as_ref() == Some(&adversary_id)
                && request.correlation_id == 78
        )
    });
    assert!(matches!(observed, TransportEvent::FrameReceived { .. }));
    assert_eq!(host.counters().sync_datagrams_received, received_before + 1);

    // An unrelated listener's PCM and authoritative controls must remain
    // intact after the invalid datagram and the valid clock probe.
    let start = stream_start(&session, 1_000);
    host.broadcast_control(&start).expect("authoritative Start");
    wait_for_frame(healthy.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(start.clone())
    });
    let pcm = audio_frame_at(&session, 0, 1_000);
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("PCM broadcast")
            .report
            .successful_peers,
        2
    );
    wait_for_frame(healthy.as_mut(), TransportChannel::Audio, |frame| {
        frame == &pcm
    });

    drop(raw_control);
    healthy.shutdown().expect("close healthy");
    host.shutdown().expect("close host");
}
