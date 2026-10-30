//! Production TCP/UDP integration: a paused multi-track stream survives
//! listener replacement, targeted clock sync, Resume, Stop and fresh Play.
use std::sync::Arc;

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{
    ControlMessage, Pause, ProtocolFrame, SyncRequest, SyncResponse, TrackBoundary,
};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, stream_stop,
    wait_for_authorized, wait_for_control_from, wait_for_frame, wait_for_frame_from,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportFactory,
};

use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let id = id_device(name);
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), id.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect real TCP/UDP listener");
    listener
        .send_control(&join_request(session, &id, name))
        .expect("send join");
    wait_for_control_from(host, &id, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&id, listener.local_routes())
        .expect("authorize real listener");
    wait_for_authorized(host, &id);
    listener
}

fn expect_control(listener: &mut dyn ListenerTransportNode, control: &ControlMessage) {
    wait_for_frame(listener, TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(control.clone())
    });
}

fn pcm(session: &crate::domain::SessionId, sequence: u64, at: u64) -> ProtocolFrame {
    let mut frame = audio_frame_at(session, sequence, at);
    let ProtocolFrame::Audio(ref mut packet) = frame else {
        panic!("PCM fixture");
    };
    let base = i16::try_from(sequence * 100).expect("bounded fixture sequence");
    packet.payload = [-32_000_i16, base, 32_767_i16, -base]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect();
    frame
}

fn deliver(
    host: &SocketHostTransport,
    listeners: &mut [&mut dyn ListenerTransportNode],
    packet: &ProtocolFrame,
) {
    assert_eq!(
        host.broadcast_audio(packet)
            .expect("real UDP PCM")
            .report
            .successful_peers,
        u32::try_from(listeners.len()).expect("bounded listeners"),
    );
    for listener in listeners {
        wait_for_frame(*listener, TransportChannel::Audio, |received| {
            received == packet
        });
    }
}

#[test]
fn paused_multitrack_reconnect_targeted_clock_and_fresh_replay_preserve_exact_pcm() {
    let session = id_session("clock-reconnect-pcm-multitrack");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind real TCP/UDP host");
    let first_id = id_device("clock-returning");
    let second_id = id_device("clock-continuing");
    let mut first = join(&mut host, &clock, &session, "clock-returning");
    let mut second = join(&mut host, &clock, &session, "clock-continuing");

    let ControlMessage::StreamStart(mut started) = stream_start(&session, 1_000) else {
        panic!("start fixture");
    };
    started.samples_per_packet = 2;
    let start = ControlMessage::StreamStart(started.clone());
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        first_sample_index: SampleIndex::new(4),
        entry_id: "track-two".to_owned(),
        display_name: "Track two".to_owned(),
    });
    for control in [&start, &boundary] {
        assert_eq!(
            host.broadcast_control(control)
                .expect("broadcast control")
                .report
                .successful_peers,
            2,
        );
        expect_control(first.as_mut(), control);
        expect_control(second.as_mut(), control);
    }
    for sequence in 0..2 {
        let packet = pcm(&session, sequence, 1_000 + sequence);
        deliver(&host, &mut [first.as_mut(), second.as_mut()], &packet);
    }

    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: started.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_050),
    });
    host.broadcast_control(&pause).expect("pause");
    expect_control(first.as_mut(), &pause);
    expect_control(second.as_mut(), &pause);
    assert_eq!(
        host.broadcast_audio(&pcm(&session, 2, 1_052))
            .expect("paused PCM is suppressed")
            .report
            .intended_peers,
        0,
    );

    // Retire only one TCP/UDP generation. The continuing listener remains
    // admitted and may request a host timestamp while the playlist is paused.
    host.disconnect_peer(&first_id).expect("retire old route");
    first.shutdown().expect("shutdown old generation");
    let probe = SyncRequest {
        session_id: session.clone(),
        correlation_id: 42,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(500),
    };
    second.send_sync_request(&probe).expect("real UDP probe");
    wait_for_frame_from(
        &mut host,
        TransportChannel::Synchronization,
        &second_id,
        |frame| {
            matches!(frame, ProtocolFrame::SyncRequest(request)
            if request.correlation_id == 42)
        },
    );
    let response = ProtocolFrame::SyncResponse(SyncResponse {
        session_id: session.clone(),
        correlation_id: 42,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(500),
        t2_host_receive_elapsed_ms: MonotonicMillis::new(503),
        t3_host_send_elapsed_ms: MonotonicMillis::new(505),
    });
    assert_eq!(
        host.send_sync_to_peer(&second_id, &response)
            .expect("targeted response")
            .report
            .successful_peers,
        1,
    );
    wait_for_frame(
        second.as_mut(),
        TransportChannel::Synchronization,
        |frame| frame == &response,
    );

    let mut returning = join(&mut host, &clock, &session, "clock-returning");
    for control in [&start, &boundary, &pause] {
        expect_control(returning.as_mut(), control);
    }
    assert_eq!(
        returning.counters().sync_datagrams_received,
        0,
        "new TCP/UDP generation must not inherit another peer's sync response",
    );

    started.host_start_time_ms = MonotonicMillis::new(1_200);
    let resume = ControlMessage::StreamStart(started.clone());
    host.broadcast_control(&resume).expect("resume");
    expect_control(returning.as_mut(), &resume);
    expect_control(second.as_mut(), &resume);
    assert_eq!(
        host.broadcast_audio(&pcm(&session, 2, 1_052))
            .expect("reject stale pre-pause PCM")
            .report
            .intended_peers,
        0,
    );
    for sequence in 2..4 {
        let packet = pcm(&session, sequence, 1_200 + sequence);
        deliver(&host, &mut [returning.as_mut(), second.as_mut()], &packet);
    }

    let stop = stream_stop(&session, 1_300);
    host.broadcast_control(&stop).expect("stop");
    expect_control(returning.as_mut(), &stop);
    expect_control(second.as_mut(), &stop);
    assert_eq!(
        host.replay_cached_playback_to_peer(&first_id)
            .expect("targeted terminal recovery")
            .report
            .successful_peers,
        1,
    );
    expect_control(returning.as_mut(), &stop);
    assert_eq!(
        host.broadcast_audio(&pcm(&session, 4, 1_301))
            .expect("suppress stopped PCM")
            .report
            .intended_peers,
        0,
    );

    started.host_start_time_ms = MonotonicMillis::new(1_500);
    let fresh = ControlMessage::StreamStart(started);
    host.broadcast_control(&fresh).expect("fresh Play");
    expect_control(returning.as_mut(), &fresh);
    expect_control(second.as_mut(), &fresh);
    assert_eq!(
        host.broadcast_audio(&pcm(&session, 4, 1_504))
            .expect("retired producer cannot skip packet zero")
            .report
            .intended_peers,
        0,
    );
    for sequence in 0..2 {
        let packet = pcm(&session, sequence, 1_500 + sequence);
        deliver(&host, &mut [returning.as_mut(), second.as_mut()], &packet);
    }
    returning.shutdown().expect("shutdown returning");
    second.shutdown().expect("shutdown continuing");
    host.shutdown().expect("shutdown host");
}
