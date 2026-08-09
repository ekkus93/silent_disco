use super::host_transport::DesktopHostTransportRuntime;
use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, MonotonicMillis, PacketSequence, PlaybackState,
    SampleIndex, StreamId,
};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, HostDraftPatch, InviteCodePatch, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, SessionAdvertisement, SnapshotRevision,
};
use silent_disco_core::transport::{
    HostTransportConfig, ListenerTransportConfig, ListenerTransportNode, SystemTransportClock,
    TransportChannel, TransportEvent, TransportFactory, production_transport_factory,
};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn desktop_host_manual_endpoint_accepts_control_join_and_surfaces_disconnect() {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-block22-host").expect("host ID")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    next_snapshot(&receiver, 0);
    submit(
        &handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(&receiver, 1);
    let source =
        AudioSourceDescriptor::new("source-block22", "fixture.wav", Some(4096), Some(2000))
            .expect("source");
    submit(
        &handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Manual endpoint host".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(&receiver, 2);
    submit(&handle, 2, CoreCommand::CreateHostSession);
    let creating = next_snapshot(&receiver, 3);
    assert_eq!(creating.host_lifecycle, HostLifecycle::CreatingSession);
    let advertisement_effect = next_effect(&receiver);
    let PlatformEffectRequest::StartAdvertising(advertisement) = advertisement_effect.request
    else {
        panic!("expected start advertising effect");
    };

    let factory = production_transport_factory();
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(advertisement.session_id.clone()),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("bind desktop host");
    let runtime = DesktopHostTransportRuntime::start(
        node,
        advertisement.clone(),
        Arc::new(handle.clone()),
        Arc::new(SystemTransportClock::default()),
    )
    .expect("start desktop transport worker");
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertisement_effect.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising completion");
    next_snapshot(&receiver, 4);

    let listener_id = DeviceId::new("desktop-block22-listener").expect("listener ID");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                advertisement.session_id.clone(),
                listener_id.clone(),
                runtime.endpoint(),
            ),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("manual listener connects to advertised endpoint");
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: advertisement.session_id.clone(),
            device: DeviceIdentity {
                device_id: listener_id.clone(),
                display_name: "Control-only listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("send join request");

    wait_for_hello(&mut *listener, &advertisement.session_id);
    let joined = wait_snapshot(&handle, |snapshot| {
        snapshot.pending_join_requests.len() == 1
    });
    assert_eq!(joined.pending_join_requests[0].device_id, listener_id);
    assert_eq!(joined.playback_state, PlaybackState::Stopped);
    assert_eq!(listener.counters().audio_datagrams_received, 0);

    listener.shutdown().expect("listener shutdown");
    let disconnected = wait_snapshot(&handle, |snapshot| {
        snapshot.pending_join_requests.is_empty()
    });
    assert!(disconnected.listeners.is_empty());
    runtime.shutdown().expect("desktop transport shutdown");
    actor.shutdown().expect("actor shutdown");
}

/// A broadcast that reached nobody must be visible as exactly that.
///
/// `broadcast_audio` already returns a `TransportDelivery` saying how many
/// recipients a frame was intended for and how many it reached; the worker
/// discarded it and kept only an aggregate last-error string. So a stream
/// broadcast into an empty session -- zero listeners, zero delivery -- looked
/// identical to a healthy one, which CLAUDE.md names explicitly as not
/// success. Queue depth had no diagnostic at all.
#[test]
fn broadcasting_to_no_listeners_is_reported_rather_than_counted_as_delivery() {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-broadcast-diag").expect("host ID")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    // Kept alive but undrained: dropping it makes the actor's notification
    // thread panic on send, which is not what this test is about.
    let _notifications = receiver;

    let advertisement = silent_disco_core::runtime::SessionAdvertisement::new(
        silent_disco_core::domain::SessionId::new("session-broadcast-diag").expect("session ID"),
        DeviceId::new("desktop-broadcast-diag").expect("device ID"),
        "Broadcast diagnostics",
        ApprovalMode::Manual,
        2,
        None,
    )
    .expect("advertisement");
    let factory = production_transport_factory();
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(advertisement.session_id.clone()),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("bind desktop host");
    let runtime = DesktopHostTransportRuntime::start(
        node,
        advertisement.clone(),
        Arc::new(handle.clone()),
        Arc::new(SystemTransportClock::default()),
    )
    .expect("start desktop transport worker");

    // No listener has joined, so this frame has nowhere to go.
    runtime
        .broadcast_frame(ProtocolFrame::Control(ControlMessage::Stop(
            silent_disco_core::protocol::Stop {
                session_id: advertisement.session_id.clone(),
                stream_id: silent_disco_core::domain::StreamId::new("stream-broadcast-diag")
                    .expect("stream ID"),
                host_stop_time_ms: silent_disco_core::domain::MonotonicMillis::new(0),
            },
        )))
        .expect("frame accepted into the broadcast queue");

    let deadline = Instant::now() + TEST_TIMEOUT;
    let broadcast = loop {
        let status = runtime.status().expect("transport status");
        if status.broadcast.frames_attempted > 0 {
            break status.broadcast;
        }
        assert!(Instant::now() < deadline, "the frame was never broadcast");
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(broadcast.frames_attempted, 1);
    assert_eq!(
        broadcast.frames_without_recipients, 1,
        "a broadcast with no recipients must not be counted as delivered"
    );
    assert_eq!(broadcast.frames_fully_delivered, 0);
    assert_eq!(broadcast.recipients_delivered, 0);
    // The queue accounting must balance: what went in has come back out.
    assert_eq!(broadcast.queue_depth, 0);
    assert_eq!(broadcast.queue_peak_depth, 1);
    assert_eq!(broadcast.queue_overflows, 0);

    actor.shutdown().expect("actor shutdown");
}

/// Matches the real packetizer's cadence, sustained long enough (2 real
/// seconds) that a per-tick draining shortfall compounds into a
/// measurable, non-vacuous overflow instead of hiding in one tick's slack
/// (a single missed 20ms tick only accumulates 4 frames, well under the
/// 64-slot queue; many consecutive misses are what overflows it, and that
/// only shows up over a real, sustained run).
const THROUGHPUT_TEST_FRAME_COUNT: u64 = 400;
const THROUGHPUT_TEST_PACKET_DURATION: Duration = Duration::from_millis(5);

/// The real packetizer produces one audio frame every
/// `DEFAULT_PACKET_DURATION_MS` (currently 5ms, 200/s) -- this worker's own
/// bounded broadcast queue must drain at least that fast, sustained, or
/// frames pile up and get silently dropped (`queue_overflows`), which is a
/// real, audible defect: reproduced against a real Android emulator and a
/// real physical phone (LG G6, 2026-08-09), both showing hundreds of
/// overflows during a ~35s stream and, on the physical device, audible
/// clipping near the end of the run.
///
/// `EVENT_POLL_INTERVAL` (20ms) and `MAX_BROADCAST_FRAMES_PER_TICK` (16)
/// were sized the day before the packet duration dropped from 20ms to 5ms
/// (confirmed via `git log`) and were never revisited for the desktop
/// worker specifically -- `recv_event` blocks for the full poll interval
/// when no control-plane traffic arrives, during which nothing drains the
/// broadcast queue at all.
#[test]
fn broadcast_queue_keeps_up_with_real_packet_pacing_without_overflowing() {
    // `_receiver` is kept alive (not read further) for the duration of the
    // test: dropping the actor's notification channel early panics its
    // notification-sending thread on the next send.
    let (actor, _handle, _receiver, advertisement, runtime, mut listener) =
        start_host_with_approved_listener();

    let stream_id = StreamId::new("stream-broadcast-throughput").expect("stream ID");
    for sequence in 0..THROUGHPUT_TEST_FRAME_COUNT {
        runtime
            .broadcast_frame(ProtocolFrame::Audio(AudioDatagram {
                session_id: advertisement.session_id.clone(),
                stream_id: stream_id.clone(),
                sequence: PacketSequence::new(sequence),
                codec: AudioCodec::PcmS16Le,
                sample_rate: 48_000,
                channels: 2,
                samples_per_packet: 240,
                first_sample_index: SampleIndex::new(sequence * 240),
                host_presentation_time_ms: MonotonicMillis::new(sequence * 5),
                payload: vec![0_u8; 240 * 2 * 2],
            }))
            .expect("frame accepted into the broadcast queue");
        std::thread::sleep(THROUGHPUT_TEST_PACKET_DURATION);
    }

    let deadline = Instant::now() + TEST_TIMEOUT;
    let broadcast = loop {
        let status = runtime.status().expect("transport status");
        if status.broadcast.frames_attempted + status.broadcast.frames_failed
            >= THROUGHPUT_TEST_FRAME_COUNT
        {
            break status.broadcast;
        }
        assert!(
            Instant::now() < deadline,
            "worker never caught up: attempted={}, overflows={}",
            status.broadcast.frames_attempted,
            status.broadcast.queue_overflows,
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        broadcast.queue_overflows, 0,
        "the broadcast queue must keep up with real packet pacing without dropping frames \
         (attempted={}, without_recipients={})",
        broadcast.frames_attempted, broadcast.frames_without_recipients,
    );

    listener.shutdown().expect("listener shutdown");
    runtime.shutdown().expect("desktop transport shutdown");
    actor.shutdown().expect("actor shutdown");
}

/// Drives a real actor through role selection, host draft, and session
/// creation, and binds a real desktop host transport worker on loopback.
#[allow(clippy::type_complexity)]
fn start_host_session() -> (
    CoreActorRuntime,
    silent_disco_core::runtime::CoreActorHandle,
    Receiver<CoreNotification>,
    SessionAdvertisement,
    DesktopHostTransportRuntime,
) {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-broadcast-throughput").expect("host ID")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    next_snapshot(&receiver, 0);
    submit(
        &handle,
        0,
        CoreCommand::SelectRole {
            role: AppRole::Host,
        },
    );
    next_snapshot(&receiver, 1);
    let source = AudioSourceDescriptor::new(
        "source-broadcast-throughput",
        "fixture.wav",
        Some(4096),
        Some(2000),
    )
    .expect("source");
    submit(
        &handle,
        1,
        CoreCommand::UpdateHostDraft(HostDraftPatch {
            session_name: Some("Broadcast throughput".to_owned()),
            approval_mode: Some(ApprovalMode::Manual),
            invite_code: InviteCodePatch::Unchanged,
            audio_source: AudioSourcePatch::Set(source),
            remember_approved_devices: Some(false),
        }),
    );
    next_snapshot(&receiver, 2);
    submit(&handle, 2, CoreCommand::CreateHostSession);
    next_snapshot(&receiver, 3);
    let advertisement_effect = next_effect(&receiver);
    let PlatformEffectRequest::StartAdvertising(advertisement) = advertisement_effect.request
    else {
        panic!("expected start advertising effect");
    };

    let factory = production_transport_factory();
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(advertisement.session_id.clone()),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("bind desktop host");
    let runtime = DesktopHostTransportRuntime::start(
        node,
        advertisement.clone(),
        Arc::new(handle.clone()),
        Arc::new(SystemTransportClock::default()),
    )
    .expect("start desktop transport worker");
    handle
        .submit_platform_event(PlatformEvent::OperationSucceeded {
            operation_id: advertisement_effect.operation_id,
            completion: PlatformOperationCompletion::AdvertisingStarted,
        })
        .expect("advertising completion");
    next_snapshot(&receiver, 4);

    (actor, handle, receiver, advertisement, runtime)
}

/// Combines [`start_host_session`] with a real listener taken all the way
/// through join and approval (not just a join -- approval is what
/// authorizes the sync/audio ports `broadcast_audio` actually sends to,
/// and a broadcast to zero listeners short-circuits without any real
/// per-recipient socket I/O).
#[allow(clippy::type_complexity)]
fn start_host_with_approved_listener() -> (
    CoreActorRuntime,
    silent_disco_core::runtime::CoreActorHandle,
    Receiver<CoreNotification>,
    SessionAdvertisement,
    DesktopHostTransportRuntime,
    Box<dyn ListenerTransportNode>,
) {
    let (actor, handle, receiver, advertisement, runtime) = start_host_session();

    let listener_id = DeviceId::new("desktop-broadcast-throughput-listener").expect("listener ID");
    let factory = production_transport_factory();
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                advertisement.session_id.clone(),
                listener_id.clone(),
                runtime.endpoint(),
            ),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("listener connects to advertised endpoint");
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: advertisement.session_id.clone(),
            device: DeviceIdentity {
                device_id: listener_id.clone(),
                display_name: "Throughput listener".to_owned(),
            },
            invite_code: None,
            sync_port: listener.local_routes().synchronization.port(),
            audio_port: listener.local_routes().audio.port(),
        }))
        .expect("send join request");
    wait_for_hello(&mut *listener, &advertisement.session_id);
    let joined = wait_snapshot(&handle, |snapshot| {
        !snapshot.pending_join_requests.is_empty()
    });
    let request_id = joined.pending_join_requests[0].request_id.clone();
    submit(
        &handle,
        joined.revision.get(),
        CoreCommand::ApproveJoin {
            request_id,
            remember_for_future: false,
        },
    );
    let approval_effect = next_transport_effect(&receiver);
    runtime
        .dispatch(approval_effect)
        .expect("dispatch join approval");
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });

    (actor, handle, receiver, advertisement, runtime, listener)
}

fn submit(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    revision: u64,
    command: CoreCommand,
) {
    handle
        .submit_command(
            CoreCommandRequest::new(SnapshotRevision::new(revision), command).expect("command"),
        )
        .expect("submit command");
}

fn next_snapshot(
    receiver: &Receiver<CoreNotification>,
    minimum: u64,
) -> silent_disco_core::runtime::CoreSnapshot {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Snapshot(snapshot)) if snapshot.revision.get() >= minimum => {
                return snapshot;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for snapshot"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn next_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::PlatformEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::Effect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn wait_snapshot(
    handle: &silent_disco_core::runtime::CoreActorHandle,
    predicate: impl Fn(&silent_disco_core::runtime::CoreSnapshot) -> bool,
) -> silent_disco_core::runtime::CoreSnapshot {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let snapshot = handle.current_snapshot().expect("current snapshot");
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for actor state"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn next_transport_effect(
    receiver: &Receiver<CoreNotification>,
) -> silent_disco_core::runtime::TransportEffect {
    loop {
        match receiver.recv_timeout(TEST_TIMEOUT) {
            Ok(CoreNotification::TransportEffect(effect)) => return effect,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for transport effect"),
            Err(RecvTimeoutError::Disconnected) => panic!("observer disconnected"),
        }
    }
}

fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    matches_expected: impl Fn(&ControlMessage) -> bool,
) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(message),
                ..
            }) if matches_expected(&message) => return,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for expected control message"
        );
    }
}

fn wait_for_hello(
    listener: &mut dyn ListenerTransportNode,
    session_id: &silent_disco_core::domain::SessionId,
) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::Hello(hello)),
                ..
            }) if &hello.session_id == session_id => return,
            Ok(_) => {}
            Err(error)
                if error.kind == silent_disco_core::transport::TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for host Hello"
        );
    }
}
