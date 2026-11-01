use super::{FfiListenerTransportError, FfiListenerTransportHandle, ProtocolFrame};
use crate::audio_abi::registry_test_guard;
use crate::listener_playback::{FfiListenerPlaybackConfig, FfiListenerPlaybackHandle};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram, ControlMessage, Disconnect};
use silent_disco_core::transport::{TransportChannel, TransportEvent, TransportPeer};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, RwLock};

const SAMPLES_PER_PACKET: u32 = 240;

/// A handle with no transport: `forward_audio` only consults the
/// attached runtime, so this exercises the routing decision on its own,
/// without sockets or timing.
fn detached_handle() -> FfiListenerTransportHandle {
    FfiListenerTransportHandle {
        inner: RwLock::new(None),
        playback: Mutex::new(None),
        forwarded_audio: AtomicU64::new(0),
    }
}

fn playback() -> Arc<FfiListenerPlaybackHandle> {
    FfiListenerPlaybackHandle::open(FfiListenerPlaybackConfig {
        session_id: "session-forwarding".to_owned(),
        stream_id: "stream-forwarding".to_owned(),
        sample_rate: 48_000,
        host_start_time_ms: 0,
        samples_per_packet: SAMPLES_PER_PACKET,
        channels: 2,
        startup_buffer_target_ms: 400,
        rebuffer_target_ms: 400,
        ring_capacity_frames: 48_000,
        ring_target_fill_frames: 19_200,
        write_lead_ms: 400,
        max_prefill_ms: 800,
        volume: 1.0,
    })
    .expect("playback runtime opens")
}

fn audio_event(sequence: u64) -> TransportEvent {
    TransportEvent::FrameReceived {
        channel: TransportChannel::Audio,
        peer: TransportPeer {
            device_id: None,
            control_address: "127.0.0.1:1".parse().expect("addr"),
        },
        frame: ProtocolFrame::Audio(AudioDatagram {
            session_id: SessionId::new("session-forwarding").expect("session"),
            stream_id: StreamId::new("stream-forwarding").expect("stream"),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: SAMPLES_PER_PACKET,
            first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
            host_presentation_time_ms: MonotonicMillis::new(sequence * 5),
            payload: vec![0_u8; SAMPLES_PER_PACKET as usize * 2 * 2],
        }),
        received_at: MonotonicMillis::new(sequence * 5),
    }
}

fn control_event() -> TransportEvent {
    TransportEvent::FrameReceived {
        channel: TransportChannel::Control,
        peer: TransportPeer {
            device_id: None,
            control_address: "127.0.0.1:1".parse().expect("addr"),
        },
        frame: ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-forwarding").expect("session"),
            listener_id: silent_disco_core::domain::DeviceId::new("d").expect("device"),
            reason: "bye".to_owned(),
        })),
        received_at: MonotonicMillis::new(0),
    }
}

/// The whole point of attaching: audio stops occupying the foreign
/// caller's event stream. If these frames were still returned, the
/// caller would keep paying one event plus two payload copies per
/// packet, which is what delayed control traffic behind audio.
#[test]
fn attached_audio_is_consumed_and_never_surfaced() {
    let _guard = registry_test_guard();
    let handle = detached_handle();
    handle.attach_playback(playback());

    for sequence in 0..5 {
        assert!(
            handle
                .forward_audio(audio_event(sequence))
                .expect("forward succeeds")
                .is_none(),
            "audio must be consumed while a runtime is attached"
        );
    }
    assert_eq!(handle.forwarded_audio_count(), 5);
}

#[test]
fn attached_playback_failure_is_returned_instead_of_counted_as_forwarded_audio() {
    let _guard = registry_test_guard();
    let handle = detached_handle();
    let playback = playback();
    playback.stop().expect("playback stops");
    handle.attach_playback(Arc::clone(&playback));

    let error = handle
        .forward_audio(audio_event(0))
        .err()
        .expect("a dead attached runtime must make transport forwarding fail");
    assert!(matches!(error, FfiListenerTransportError::Io(_)));
    assert_eq!(handle.forwarded_audio_count(), 0);
}

/// Control traffic must be unaffected -- it is precisely what was being
/// delayed, so it has to keep reaching the caller promptly.
#[test]
fn control_events_are_never_consumed() {
    let _guard = registry_test_guard();
    let handle = detached_handle();
    handle.attach_playback(playback());

    assert!(
        handle
            .forward_audio(control_event())
            .expect("forward succeeds")
            .is_some(),
        "control frames must still reach the caller"
    );
    assert_eq!(handle.forwarded_audio_count(), 0);
}

/// With nothing attached the previous behaviour must be preserved, so a
/// caller that never attaches still receives audio events.
#[test]
fn unattached_audio_still_surfaces_to_the_caller() {
    let _guard = registry_test_guard();
    let handle = detached_handle();
    assert!(
        handle
            .forward_audio(audio_event(0))
            .expect("forward succeeds")
            .is_some(),
        "audio must surface when no runtime is attached"
    );
    assert_eq!(handle.forwarded_audio_count(), 0);
}

/// Stop must wait for a poller's in-flight submission to finish before
/// the attached runtime can be detached and stopped. The barrier is held
/// inside the *production* forwarding lock, not a simulated generation.
#[test]
fn detach_waits_for_in_flight_pcm_forwarding_before_runtime_shutdown() {
    let _guard = registry_test_guard();
    let handle = Arc::new(detached_handle());
    let runtime = playback();
    handle.attach_playback(Arc::clone(&runtime));
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let forwarding_handle = Arc::clone(&handle);
    let forwarding = std::thread::spawn(move || {
        forwarding_handle
            .with_playback_lock(|attached| {
                assert!(attached.is_some());
                entered_tx.send(()).expect("signal forwarding");
                release_rx.recv().expect("release forwarding");
            })
            .expect("forwarding attachment lock");
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("forwarding entered lock");
    assert!(
        handle.playback.try_lock().is_err(),
        "forwarding holds attachment lock"
    );

    let (attempt_tx, attempt_rx) = std::sync::mpsc::sync_channel(1);
    let (detached_tx, detached_rx) = std::sync::mpsc::sync_channel(1);
    let detaching_handle = Arc::clone(&handle);
    let detaching = std::thread::spawn(move || {
        attempt_tx.send(()).expect("signal detach attempt");
        detaching_handle.detach_playback();
        detached_tx.send(()).expect("signal detach completion");
    });
    attempt_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("detach attempted");
    assert!(
        matches!(
            detached_rx.recv_timeout(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "detach must not complete while a PCM submission owns the runtime"
    );
    release_tx.send(()).expect("finish forwarding");
    forwarding.join().expect("forwarding completes");
    detaching.join().expect("detach completes");
    runtime.stop().expect("detached runtime can stop");
    assert!(
        handle
            .forward_audio(audio_event(0))
            .expect("unattached audio remains available")
            .is_some()
    );
    assert_eq!(handle.forwarded_audio_count(), 0);
}

/// Detaching must stop forwarding immediately: submitting into a
/// runtime that is being stopped is exactly what the detach ordering in
/// the Kotlin controller exists to prevent.
#[test]
fn detaching_returns_audio_to_the_caller() {
    let _guard = registry_test_guard();
    let handle = detached_handle();
    handle.attach_playback(playback());
    assert!(
        handle
            .forward_audio(audio_event(0))
            .expect("forward succeeds")
            .is_none()
    );

    handle.detach_playback();
    assert!(
        handle
            .forward_audio(audio_event(1))
            .expect("forward succeeds")
            .is_some(),
        "audio must surface again once detached"
    );
    handle.detach_playback();
    assert_eq!(handle.forwarded_audio_count(), 1);
}

/// Actual TCP/UDP ingress through the production listener FFI poller must
/// submit PCM directly to the SAME runtime across Pause/Resume and a
/// midqueue boundary. A second listener verifies independent wire delivery.
/// This is a software-native runtime identity check, not an Oboe/DAC claim.
#[test]
fn real_socket_pcm_forwarding_survives_pause_late_join_resume_and_stop_without_runtime_reopen() {
    use super::Inner;
    use silent_disco_core::domain::DeviceId;
    use silent_disco_core::protocol::{Pause, Stop, StreamStart, TrackBoundary};
    use silent_disco_core::transport::test_support::{
        id_device, join_request, wait_for_authorized, wait_for_control_from, wait_for_frame,
    };
    use silent_disco_core::transport::{
        HostTransportConfig, ListenerTransportConfig, ManualTransportClock,
        SocketTransportFactory, TransportClock, TransportFactory,
    };

    let _guard = registry_test_guard();
    let session = SessionId::new("session-forwarding").expect("session");
    let stream = StreamId::new("stream-forwarding").expect("stream");
    let clock = Arc::new(ManualTransportClock::new(0));
    let mut host = SocketTransportFactory
        .bind_host(
            HostTransportConfig::loopback(session.clone()),
            clock.clone(),
        )
        .expect("production host sockets");
    let device = DeviceId::new("ffi-forwarding-listener").expect("device");
    let listener = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), device.clone(), host.endpoint()),
            clock.clone(),
        )
        .expect("production listener sockets");
    let routes = listener.local_routes();
    let transport_clock: Arc<dyn TransportClock> = clock.clone();
    let handle = FfiListenerTransportHandle {
        inner: RwLock::new(Some(Inner {
            transport: listener,
            session_id: session.clone(),
            device_id: device.clone(),
            clock: transport_clock,
        })),
        playback: Mutex::new(None),
        forwarded_audio: AtomicU64::new(0),
    };
    handle
        .send_join_request("FFI listener".to_owned(), None)
        .expect("real TCP join");
    wait_for_control_from(host.as_mut(), &device, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&device, routes).expect("authorize UDP");
    wait_for_authorized(host.as_mut(), &device);

    let runtime = playback();
    runtime
        .lock_same_process_host_clock(0)
        .expect("lock local test clock");
    let original_token = runtime.engine_token();
    handle.attach_playback(Arc::clone(&runtime));

    let start = ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(0),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: SAMPLES_PER_PACKET,
        current_entry_id: Some("track-a".to_owned()),
        current_display_name: Some("Track A".to_owned()),
    });
    host.broadcast_control(&start).expect("start stream");
    assert!(matches!(
        handle.poll_event(1_000).expect("FFI control poll"),
        Some(super::FfiListenerTransportEvent::StreamStarted { .. })
    ));
    let boundary = ControlMessage::TrackBoundary(TrackBoundary {
        session_id: session.clone(),
        stream_id: stream.clone(),
        first_sample_index: SampleIndex::new(480),
        entry_id: "track-b".to_owned(),
        display_name: "Track B".to_owned(),
    });
    host.broadcast_control(&boundary).expect("queue boundary");
    assert!(matches!(
        handle.poll_event(1_000).expect("FFI boundary poll"),
        Some(super::FfiListenerTransportEvent::TrackBoundary {
            first_sample_index: 480,
            ..
        })
    ));

    let make_packet = |sequence: u64, at: u64| {
        let sample = i16::try_from(sequence + 1).expect("bounded fixture");
        ProtocolFrame::Audio(AudioDatagram {
            session_id: session.clone(),
            stream_id: stream.clone(),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: SAMPLES_PER_PACKET,
            first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
            host_presentation_time_ms: MonotonicMillis::new(at),
            payload: [sample, -sample]
                .into_iter()
                .cycle()
                .take(SAMPLES_PER_PACKET as usize * 2)
                .flat_map(i16::to_le_bytes)
                .collect(),
        })
    };
    for sequence in 0..2 {
        assert_eq!(
            host.broadcast_audio(&make_packet(sequence, sequence * 5))
                .expect("production UDP PCM")
                .report
                .successful_peers,
            1
        );
    }
    assert!(
        handle
            .poll_event(300)
            .expect("drain forwarded PCM")
            .is_none()
    );
    assert_eq!(handle.forwarded_audio_count(), 2);
    assert_eq!(runtime.engine_token(), original_token);

    let pause = ControlMessage::Pause(Pause {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_pause_time_ms: MonotonicMillis::new(8),
    });
    host.broadcast_control(&pause).expect("pause");
    assert!(matches!(
        handle.poll_event(1_000).expect("FFI pause poll"),
        Some(super::FfiListenerTransportEvent::Paused { .. })
    ));
    assert_eq!(
        host.broadcast_audio(&make_packet(2, 110))
            .expect("paused audio suppressed")
            .report
            .intended_peers,
        0
    );

    // Pause before sample 480 becomes audible at 10 ms, so the late
    // listener must still receive that upcoming boundary in its cache.
    clock.set(9);
    let late_device = id_device("ffi-forwarding-late");
    let mut late = SocketTransportFactory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session.clone(),
                late_device.clone(),
                host.endpoint(),
            ),
            clock.clone(),
        )
        .expect("late listener");
    late.send_control(&join_request(&session, &late_device, "Late listener"))
        .expect("late TCP join");
    wait_for_control_from(host.as_mut(), &late_device, |control| {
        matches!(control, ControlMessage::JoinRequest(_))
    });
    host.authorize_peer(&late_device, late.local_routes())
        .expect("authorize late");
    wait_for_authorized(host.as_mut(), &late_device);
    for control in [&start, &boundary, &pause] {
        wait_for_frame(late.as_mut(), TransportChannel::Control, |frame| {
            frame == &ProtocolFrame::Control(control.clone())
        });
    }

    let ControlMessage::StreamStart(mut resumed) = start.clone() else {
        panic!("stream fixture");
    };
    resumed.host_start_time_ms = MonotonicMillis::new(200);
    let resume = ControlMessage::StreamStart(resumed);
    runtime
        .reanchor_presentation_time(200)
        .expect("reanchor the existing native runtime");
    host.broadcast_control(&resume).expect("resume");
    assert!(matches!(
        handle.poll_event(1_000).expect("FFI resume poll"),
        Some(super::FfiListenerTransportEvent::StreamStarted {
            host_start_time_ms: 200,
            ..
        })
    ));
    wait_for_frame(late.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(resume.clone())
    });
    assert_eq!(runtime.engine_token(), original_token);
    let resumed_pcm = make_packet(2, 210);
    assert_eq!(
        host.broadcast_audio(&resumed_pcm)
            .expect("resumed UDP")
            .report
            .successful_peers,
        2
    );
    wait_for_frame(late.as_mut(), TransportChannel::Audio, |frame| {
        frame == &resumed_pcm
    });
    assert!(
        handle
            .poll_event(300)
            .expect("forward resumed PCM")
            .is_none()
    );
    assert_eq!(handle.forwarded_audio_count(), 3);
    assert_eq!(runtime.engine_token(), original_token);

    let stop = ControlMessage::Stop(Stop {
        session_id: session.clone(),
        stream_id: stream,
        host_stop_time_ms: MonotonicMillis::new(300),
    });
    host.broadcast_control(&stop).expect("stop");
    assert!(matches!(
        handle.poll_event(1_000).expect("FFI Stop poll"),
        Some(super::FfiListenerTransportEvent::Stopped { .. })
    ));
    wait_for_frame(late.as_mut(), TransportChannel::Control, |frame| {
        frame == &ProtocolFrame::Control(stop.clone())
    });
    handle.detach_playback();
    runtime.stop().expect("stop detached runtime");
    assert_eq!(
        host.broadcast_audio(&make_packet(3, 305))
            .expect("stopped PCM suppressed")
            .report
            .intended_peers,
        0
    );
    assert_eq!(handle.forwarded_audio_count(), 3);
    handle.shutdown().expect("shutdown FFI listener");
    late.shutdown().expect("shutdown late listener");
    host.shutdown().expect("shutdown host");
}
