use super::host_transport::DesktopHostTransportRuntime;
use silent_disco_core::domain::{
    AppRole, ApprovalMode, DeviceId, HostLifecycle, PlaybackState,
};
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
};
use silent_disco_core::runtime::{
    AudioSourceDescriptor, AudioSourcePatch, CoreActorConfig, CoreActorRuntime, CoreCommand,
    CoreCommandRequest, CoreNotification, HostDraftPatch, InviteCodePatch, PlatformEffectRequest,
    PlatformEvent, PlatformOperationCompletion, SnapshotRevision,
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
    submit(&handle, 0, CoreCommand::SelectRole { role: AppRole::Host });
    next_snapshot(&receiver, 1);
    let source = AudioSourceDescriptor::new("source-block22", "fixture.wav", Some(4096), Some(2000))
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
    let PlatformEffectRequest::StartAdvertising(advertisement) = advertisement_effect.request else {
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
        }))
        .expect("send join request");

    wait_for_hello(&mut *listener, &advertisement.session_id);
    let joined = wait_snapshot(&handle, |snapshot| snapshot.pending_join_requests.len() == 1);
    assert_eq!(joined.pending_join_requests[0].device_id, listener_id);
    assert_eq!(joined.playback_state, PlaybackState::Stopped);
    assert_eq!(listener.counters().audio_datagrams_received, 0);

    listener.shutdown().expect("listener shutdown");
    let disconnected = wait_snapshot(&handle, |snapshot| snapshot.pending_join_requests.is_empty());
    assert!(disconnected.listeners.is_empty());
    runtime.shutdown().expect("desktop transport shutdown");
    actor.shutdown().expect("actor shutdown");
}

fn submit(handle: &silent_disco_core::runtime::CoreActorHandle, revision: u64, command: CoreCommand) {
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

fn next_effect(receiver: &Receiver<CoreNotification>) -> silent_disco_core::runtime::PlatformEffect {
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
        assert!(Instant::now() < deadline, "timed out waiting for actor state");
        std::thread::sleep(Duration::from_millis(10));
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
        assert!(Instant::now() < deadline, "timed out waiting for host Hello");
    }
}
