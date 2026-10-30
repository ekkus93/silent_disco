//! Real TCP recovery replay of the host's authoritative control cache.
//! No synthetic UI state or new native output is created for resync.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::{MonotonicMillis, SampleIndex};
use crate::protocol::{ControlMessage, Pause, ProtocolFrame, ResyncNotice, Stop, TrackBoundary};
use crate::transport::test_support::{
    audio_frame_at, id_device, id_session, join_request, stream_start, wait_for_authorized,
    wait_for_control_from, wait_for_frame,
};
use crate::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, SocketTransportFactory, TransportChannel, TransportEvent,
    TransportFactory,
};

use super::SocketHostTransport;

fn join(
    host: &mut SocketHostTransport,
    factory: SocketTransportFactory,
    clock: &Arc<ManualTransportClock>,
    session: &crate::domain::SessionId,
    name: &str,
) -> Box<dyn ListenerTransportNode> {
    let device = id_device(name);
    let listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("connect listener");
    listener
        .send_control(&join_request(session, &device, name))
        .expect("send join request");
    wait_for_control_from(host, &device, |message| {
        matches!(message, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, listener.local_routes())
        .expect("authorize listener");
    wait_for_authorized(host, &device);
    listener
}

fn expect_controls(listener: &dyn ListenerTransportNode, expected: &[ControlMessage]) {
    for control in expected {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "missing replayed TCP control");
            if let TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(received),
                ..
            } = listener
                .recv_event(remaining)
                .expect("read listener control")
            {
                assert!(received == *control, "replay order and contents");
                break;
            }
        }
    }
}

#[test]
fn resync_replays_only_to_admitted_requester_and_stop_disarms_recovery() {
    let session = id_session("resync-authoritative-snapshot");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind host");
    let factory = SocketTransportFactory;
    let first_id = id_device("resync-first");
    let mut first = join(&mut host, factory, &clock, &session, "resync-first");
    let mut second = join(&mut host, factory, &clock, &session, "resync-second");
    let start = stream_start(&session, 1_000);
    let ControlMessage::StreamStart(start_info) = &start else {
        panic!("start fixture");
    };
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: start_info.stream_id.clone(),
        first_sample_index: SampleIndex::new(48_000),
        entry_id: "next".to_owned(),
        display_name: "Next track".to_owned(),
    });
    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: start_info.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_200),
    });
    let stream_id = start_info.stream_id.clone();
    let expected = [start, boundary, pause];
    for control in &expected {
        assert_eq!(
            host.broadcast_control(control)
                .expect("broadcast")
                .report
                .successful_peers,
            2
        );
    }
    expect_controls(first.as_ref(), &expected);
    expect_controls(second.as_ref(), &expected);

    first
        .send_control(&ControlMessage::ResyncNotice(ResyncNotice {
            session_id: session.clone(),
            listener_id: first_id.clone(),
            reason: "missing control after reconnect".to_owned(),
        }))
        .expect("authorized listener requests recovery");
    wait_for_control_from(&mut host, &first_id, |message| {
        matches!(message, ControlMessage::ResyncNotice(_))
    });
    let before = host.counters().control_frames_sent;
    let delivery = host
        .replay_cached_playback_to_peer(&first_id)
        .expect("targeted recovery");
    assert_eq!(delivery.report.successful_peers, 1);
    assert!(delivery.bytes_sent > 0);
    assert_eq!(host.counters().control_frames_sent - before, 3);
    expect_controls(first.as_ref(), &expected);

    // Stop invalidates the active cache. A new join sees no playback, but an
    // already-admitted listener requesting recovery must receive the missed
    // terminal Stop instead of continuing its retired output indefinitely.
    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id,
        host_stop_time_ms: MonotonicMillis::new(1_500),
    });
    host.broadcast_control(&stop).expect("stop playback");
    expect_controls(first.as_ref(), std::slice::from_ref(&stop));
    expect_controls(second.as_ref(), std::slice::from_ref(&stop));
    let stopped = host.counters().control_frames_sent;
    assert_eq!(
        host.replay_cached_playback_to_peer(&first_id)
            .expect("terminal recovery")
            .report
            .successful_peers,
        1
    );
    expect_controls(first.as_ref(), std::slice::from_ref(&stop));
    assert_eq!(host.counters().control_frames_sent, stopped + 1);
    assert!(
        host.replay_cached_playback_to_peer(&id_device("unknown"))
            .is_err()
    );

    // New joins after Stop receive no retired control, while a later fresh
    // Play replaces the recovery watermark rather than replaying old Stop.
    let before_late = host.counters().control_frames_sent;
    let mut late = join(&mut host, factory, &clock, &session, "resync-late");
    assert_eq!(host.counters().control_frames_sent, before_late);
    let fresh = stream_start(&session, 2_000);
    assert_eq!(
        host.broadcast_control(&fresh)
            .expect("new Play after terminal recovery")
            .report
            .successful_peers,
        3
    );
    expect_controls(first.as_ref(), std::slice::from_ref(&fresh));
    expect_controls(second.as_ref(), std::slice::from_ref(&fresh));
    expect_controls(late.as_ref(), std::slice::from_ref(&fresh));
    assert_eq!(
        host.replay_cached_playback_to_peer(&first_id)
            .expect("fresh playback recovery")
            .report
            .successful_peers,
        1
    );
    expect_controls(first.as_ref(), std::slice::from_ref(&fresh));
    late.shutdown().expect("close late");
    first.shutdown().expect("close first");
    second.shutdown().expect("close second");
    host.shutdown().expect("close host");
}

#[test]
fn paused_multitrack_recovery_preserves_exact_pcm_and_stop_replay_on_real_sockets() {
    let session = id_session("resync-pcm-pause-resume-stop");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketHostTransport::bind(
        HostTransportConfig::loopback(session.clone()),
        clock.clone(),
    )
    .expect("bind production TCP and UDP sockets");
    let factory = SocketTransportFactory;
    let first_id = id_device("recovery-pcm-first");
    let mut first = join(&mut host, factory, &clock, &session, "recovery-pcm-first");
    let mut other = join(&mut host, factory, &clock, &session, "recovery-pcm-other");

    let ControlMessage::StreamStart(mut start) = stream_start(&session, 1_000) else {
        panic!("StreamStart fixture");
    };
    start.samples_per_packet = 2;
    let original = ControlMessage::StreamStart(start.clone());
    let boundaries: Vec<ControlMessage> = [(4, "second"), (8, "third")]
        .into_iter()
        .map(|(index, name)| {
            ControlMessage::TrackBoundary(TrackBoundary {
                session_id: session.clone(),
                stream_id: start.stream_id.clone(),
                first_sample_index: SampleIndex::new(index),
                entry_id: name.to_owned(),
                display_name: name.to_owned(),
            })
        })
        .collect();
    for control in std::iter::once(&original).chain(boundaries.iter()) {
        assert_eq!(
            host.broadcast_control(control)
                .expect("live control")
                .report
                .successful_peers,
            2,
        );
        expect_controls(first.as_ref(), std::slice::from_ref(control));
        expect_controls(other.as_ref(), std::slice::from_ref(control));
    }

    // Real UDP, not a synthetic callback: each stereo packet has distinct
    // signed PCM values and exact contiguous sample/sequence coordinates.
    let pcm = |sequence: u64, presentation_ms: u64| {
        let mut frame = audio_frame_at(&session, sequence, presentation_ms);
        let ProtocolFrame::Audio(ref mut audio) = frame else {
            panic!("PCM fixture");
        };
        let base = i16::try_from(sequence * 10).expect("bounded fixture");
        audio.payload = [base, base + 1, base + 2, base + 3]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        frame
    };
    for sequence in 0..2 {
        let packet = pcm(sequence, 1_000 + sequence);
        assert_eq!(
            host.broadcast_audio(&packet)
                .expect("live PCM")
                .report
                .successful_peers,
            2,
        );
        for listener in [&mut first, &mut other] {
            wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
                frame == &packet
            });
        }
    }

    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: start.stream_id.clone(),
        host_pause_time_ms: MonotonicMillis::new(1_050),
    });
    host.broadcast_control(&pause).expect("pause active stream");
    expect_controls(first.as_ref(), std::slice::from_ref(&pause));
    expect_controls(other.as_ref(), std::slice::from_ref(&pause));
    assert_eq!(
        host.broadcast_audio(&pcm(2, 1_052))
            .expect("paused producer must be suppressed")
            .report
            .intended_peers,
        0,
    );

    // The same already-admitted TCP peer requests recovery while paused.
    // Its exact authoritative snapshot is Start + both pending boundaries +
    // Pause. The other peer must not receive a duplicate control stream.
    first
        .send_control(&ControlMessage::ResyncNotice(ResyncNotice {
            session_id: session.clone(),
            listener_id: first_id.clone(),
            reason: "lost second-track boundary".to_owned(),
        }))
        .expect("request recovery");
    wait_for_control_from(&mut host, &first_id, |message| {
        matches!(message, ControlMessage::ResyncNotice(_))
    });
    let before = host.counters().control_frames_sent;
    assert_eq!(
        host.replay_cached_playback_to_peer(&first_id)
            .expect("replay paused timeline")
            .report
            .successful_peers,
        1,
    );
    assert_eq!(host.counters().control_frames_sent - before, 4);
    for control in std::iter::once(&original)
        .chain(boundaries.iter())
        .chain(std::iter::once(&pause))
    {
        expect_controls(first.as_ref(), std::slice::from_ref(control));
    }

    start.host_start_time_ms = MonotonicMillis::new(1_200);
    let resume = ControlMessage::StreamStart(start.clone());
    host.broadcast_control(&resume).expect("reanchor resume");
    expect_controls(first.as_ref(), std::slice::from_ref(&resume));
    expect_controls(other.as_ref(), std::slice::from_ref(&resume));
    assert_eq!(
        host.broadcast_audio(&pcm(2, 1_052))
            .expect("old pre-pause UDP must stay retired")
            .report
            .intended_peers,
        0,
    );

    // Recovery after Resume must not resurrect Pause or the old anchor.
    let before = host.counters().control_frames_sent;
    host.replay_cached_playback_to_peer(&first_id)
        .expect("recover resumed timeline");
    assert_eq!(host.counters().control_frames_sent - before, 3);
    for control in std::iter::once(&resume).chain(boundaries.iter()) {
        expect_controls(first.as_ref(), std::slice::from_ref(control));
    }
    for sequence in 2..6 {
        let packet = pcm(sequence, 1_200 + sequence);
        assert_eq!(
            host.broadcast_audio(&packet)
                .expect("contiguous post-recovery PCM")
                .report
                .successful_peers,
            2,
        );
        for listener in [&mut first, &mut other] {
            wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
                frame == &packet
            });
        }
    }

    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: start.stream_id.clone(),
        host_stop_time_ms: MonotonicMillis::new(1_400),
    });
    host.broadcast_control(&stop).expect("stop queue");
    expect_controls(first.as_ref(), std::slice::from_ref(&stop));
    expect_controls(other.as_ref(), std::slice::from_ref(&stop));
    assert_eq!(
        host.broadcast_audio(&pcm(6, 1_406))
            .expect("retired PCM must not escape")
            .report
            .intended_peers,
        0,
    );
    host.replay_cached_playback_to_peer(&first_id)
        .expect("recover missed terminal Stop");
    expect_controls(first.as_ref(), std::slice::from_ref(&stop));

    // Fresh late join receives neither old Stop nor retired StreamStart.
    let before_late = host.counters().control_frames_sent;
    let mut late = join(&mut host, factory, &clock, &session, "recovery-pcm-late");
    assert_eq!(host.counters().control_frames_sent, before_late);
    start.host_start_time_ms = MonotonicMillis::new(1_500);
    let fresh = ControlMessage::StreamStart(start);
    host.broadcast_control(&fresh).expect("fresh Play");
    for listener in [&mut first, &mut other, &mut late] {
        expect_controls(listener.as_ref(), std::slice::from_ref(&fresh));
    }
    let first_packet = pcm(0, 1_500);
    assert_eq!(
        host.broadcast_audio(&first_packet)
            .expect("fresh generation packet zero")
            .report
            .successful_peers,
        3,
    );
    for listener in [&mut first, &mut other, &mut late] {
        wait_for_frame(listener.as_mut(), TransportChannel::Audio, |frame| {
            frame == &first_packet
        });
        listener.shutdown().expect("close listener");
    }
    host.shutdown().expect("close host");
}
