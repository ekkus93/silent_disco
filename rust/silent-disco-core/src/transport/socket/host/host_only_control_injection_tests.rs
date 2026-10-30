//! Production TCP/UDP negative acceptance: an admitted listener cannot inject
//! host-only playback controls into another listener's authoritative timeline.
//! This is a legacy-v3 fail-closed validation test, NOT TLS authentication.
use std::sync::Arc;

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, Stop, TrackBoundary};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame, wait_for_host_event,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportErrorKind,
    TransportEvent, TransportFactory,
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
        .expect("connect real listener");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("join");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize");
    wait_for_authorized(host, &device);
    listener
}

#[test]
fn forged_listener_stop_is_rejected_without_disrupting_other_peer_pcm_or_replay() {
    let session = id_session("forged-host-stop-on-legacy-control");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP/UDP host");
    let attacker_id = id_device("injecting-listener");
    let mut attacker = join(&mut host, &clock, &session, "injecting-listener");
    let mut healthy = join(&mut host, &clock, &session, "healthy-listener");

    let start = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(ref started) = start else {
        panic!("StreamStart fixture");
    };
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        first_sample_index: SampleIndex::new(48_000),
        entry_id: "next".to_owned(),
        display_name: "Next song".to_owned(),
    });
    for control in [&start, &boundary] {
        assert_eq!(
            host.broadcast_control(control)
                .expect("broadcast authoritative control")
                .report
                .successful_peers,
            2,
        );
        for listener in [&mut attacker, &mut healthy] {
            wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
                frame == &ProtocolFrame::Control(control.clone())
            });
        }
    }

    // This frame has a valid session and stream, but arrives from a listener
    // TCP route. It must never be projected as a host Stop, even though the
    // sender was admitted for receiving audio and sync responses.
    let forged = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_050),
    });
    attacker
        .send_control(&forged)
        .expect("send syntactically valid but unauthorized Stop");
    let event = wait_for_host_event(&mut host, |event| {
        matches!(
            event,
            TransportEvent::PeerDisconnected {
                peer,
                error: Some(error),
                ..
            } if peer.device_id.as_ref() == Some(&attacker_id)
                && error.kind == TransportErrorKind::Unauthorized
        )
    });
    assert!(matches!(
        event,
        TransportEvent::PeerDisconnected {
            error: Some(ref error),
            ..
        } if error.kind == TransportErrorKind::Unauthorized
    ));
    assert!(host.counters().unauthorized_frames > 0);
    assert_eq!(
        host.routes.lock().expect("routes").len(),
        1,
        "invalid sender's UDP route must be retired"
    );
    let snapshot = host.playback_controls.lock().expect("cache").snapshot();
    assert_eq!(
        snapshot.len(),
        2,
        "injected Stop must not clear Start or upcoming boundary"
    );

    let pcm = audio_frame_at(&session, 0, 1_000);
    assert_eq!(
        host.broadcast_audio(&pcm)
            .expect("healthy peer keeps receiving PCM")
            .report
            .successful_peers,
        1
    );
    wait_for_frame(healthy.as_mut(), TransportChannel::Audio, |frame| {
        frame == &pcm
    });

    // A subsequent host-owned Pause/Stop must still be delivered and retired
    // correctly, rather than inheriting the rejected listener's fake Stop.
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_100),
    });
    host.broadcast_control(&pause).expect("real host Pause");
    wait_for_frame(healthy.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(pause.clone())
    });
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_200),
    });
    host.broadcast_control(&stop).expect("real host Stop");
    wait_for_frame(healthy.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(stop.clone())
    });
    assert!(host.playback_controls.lock().expect("cache").snapshot().is_empty());

    // A fresh TCP generation using the same unverified legacy ID must join
    // explicitly and must not inherit the old route or retired playback.
    let mut returning = join(&mut host, &clock, &session, "injecting-listener");
    assert_eq!(host.routes.lock().expect("routes").len(), 2);
    let fresh = stream_start(&session, 2_000);
    host.broadcast_control(&fresh).expect("fresh Play");
    for listener in [&mut healthy, &mut returning] {
        wait_for_frame(listener.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(fresh.clone())
        });
    }
    attacker.shutdown().expect("close rejected connection");
    healthy.shutdown().expect("close healthy listener");
    returning.shutdown().expect("close rejoined listener");
    host.shutdown().expect("close host");
}
