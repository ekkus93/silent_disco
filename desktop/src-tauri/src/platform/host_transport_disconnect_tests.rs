//! Desktop production effect routing: a kick must retire admitted UDP access,
//! while a not-yet-authorized listener still receives an explicit TCP denial.
use super::host_transport::DesktopHostTransportRuntime;
use super::host_transport_events::DesktopHostTransportEventSink;
use silent_disco_core::domain::{
    ApprovalMode, DeviceId, MonotonicMillis, OperationId, PacketSequence, RequestId, SampleIndex,
    SessionId, StreamId,
};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
    StreamStart,
};
use silent_disco_core::runtime::{
    CoreSnapshot, NetworkEndpoint, SessionAdvertisement, TransportEffect, TransportEffectRequest,
    TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportConfig, ListenerTransportConfig, ListenerTransportNode, SystemTransportClock,
    TransportChannel, TransportErrorKind, TransportEvent, TransportFactory,
    production_transport_factory,
};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

struct RecordingSink(mpsc::Sender<CoreTransportEvent>);

impl DesktopHostTransportEventSink for RecordingSink {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(CoreSnapshot::default())
    }

    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError> {
        self.0.send(event).expect("test observer is alive");
        Ok(())
    }

    fn submit_audio_event(
        &self,
        _event: silent_disco_core::runtime::AudioEvent,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}

fn setup() -> (
    DesktopHostTransportRuntime,
    SessionId,
    NetworkEndpoint,
    mpsc::Receiver<CoreTransportEvent>,
) {
    let session = SessionId::new("desktop-effect-disconnect-session").expect("session");
    let host = DeviceId::new("desktop-effect-disconnect-host").expect("host");
    let advertisement = SessionAdvertisement::new(
        session.clone(),
        host,
        "Disconnect regression",
        ApprovalMode::Manual,
        silent_disco_core::protocol::PROTOCOL_VERSION,
        None,
    )
    .expect("advertisement");
    let clock = Arc::new(SystemTransportClock::default());
    let node = production_transport_factory()
        .bind_host(HostTransportConfig::loopback(session.clone()), clock.clone())
        .expect("real host TCP/UDP");
    let endpoint = node.endpoint();
    let (sender, receiver) = mpsc::channel();
    let runtime = DesktopHostTransportRuntime::start(
        node,
        advertisement,
        Arc::new(RecordingSink(sender)),
        clock,
    )
    .expect("desktop host worker");
    (runtime, session, endpoint, receiver)
}

fn wait_event(
    receiver: &mpsc::Receiver<CoreTransportEvent>,
    predicate: impl Fn(&CoreTransportEvent) -> bool,
) -> CoreTransportEvent {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let event = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("expected host actor event");
        if predicate(&event) {
            return event;
        }
    }
}

fn wait_frame(
    listener: &mut dyn ListenerTransportNode,
    predicate: impl Fn(&ProtocolFrame) -> bool,
) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived { frame, .. }) if predicate(&frame) => return,
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(Instant::now() < deadline, "missing listener frame");
    }
}

fn join(
    session: &SessionId,
    endpoint: NetworkEndpoint,
    name: &str,
    receiver: &mpsc::Receiver<CoreTransportEvent>,
) -> Box<dyn ListenerTransportNode> {
    let id = DeviceId::new(name).expect("listener ID");
    let listener = production_transport_factory()
        .connect_listener(
            ListenerTransportConfig::loopback(session.clone(), id.clone(), endpoint),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("real listener");
    let ports = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session.clone(),
            device: DeviceIdentity {
                device_id: id.clone(),
                display_name: name.to_owned(),
            },
            invite_code: None,
            sync_port: ports.synchronization.port(),
            audio_port: ports.audio.port(),
        }))
        .expect("send join");
    let mut listener = listener;
    wait_frame(listener.as_mut(), |frame| {
        matches!(frame, ProtocolFrame::Control(ControlMessage::Hello(_)))
    });
    assert!(matches!(
        wait_event(receiver, |event| {
            matches!(event, CoreTransportEvent::JoinRequested(request) if request.device_id == id)
        }),
        CoreTransportEvent::JoinRequested(_)
    ));
    listener
}

fn dispatch(
    runtime: &DesktopHostTransportRuntime,
    effect: TransportEffectRequest,
    operation: &str,
) {
    runtime
        .dispatch(
            TransportEffect::new(OperationId::new(operation).expect("operation"), effect)
                .expect("effect"),
        )
        .expect("queue effect");
}

fn delivered(receiver: &mpsc::Receiver<CoreTransportEvent>) {
    assert!(matches!(
        wait_event(receiver, |event| {
            matches!(event, CoreTransportEvent::DeliveryCompleted { .. })
        }),
        CoreTransportEvent::DeliveryCompleted { report, .. }
            if report.intended_peers == 1
                && report.successful_peers == 1
                && report.failed_peers == 0
    ));
}

fn approve(
    runtime: &DesktopHostTransportRuntime,
    session: &SessionId,
    id: &DeviceId,
    listener: &mut dyn ListenerTransportNode,
    receiver: &mpsc::Receiver<CoreTransportEvent>,
    operation: &str,
) {
    dispatch(
        runtime,
        TransportEffectRequest::DeliverJoinApproval {
            request_id: RequestId::new(format!("{operation}-request")).expect("request"),
            session_id: session.clone(),
            listener_id: id.clone(),
            trusted_for_future: false,
        },
        operation,
    );
    wait_frame(listener, |frame| {
        matches!(frame, ProtocolFrame::Control(ControlMessage::JoinApproval(value))
            if value.listener_id == *id)
    });
    delivered(receiver);
    assert!(matches!(
        wait_event(receiver, |event| {
            matches!(event, CoreTransportEvent::ListenerConnected(value)
                if value.device_id == *id)
        }),
        CoreTransportEvent::ListenerConnected(_)
    ));
}

#[test]
fn authorized_disconnect_effect_retires_udp_but_preserves_healthy_playback_and_reconnect() {
    let (runtime, session, endpoint, receiver) = setup();
    let kicked_id = DeviceId::new("desktop-kicked-listener").expect("ID");
    let healthy_id = DeviceId::new("desktop-healthy-listener").expect("ID");
    let mut kicked = join(&session, endpoint, kicked_id.as_str(), &receiver);
    approve(
        &runtime, &session, &kicked_id, kicked.as_mut(), &receiver, "approve-kicked",
    );
    let mut healthy = join(&session, endpoint, healthy_id.as_str(), &receiver);
    approve(
        &runtime, &session, &healthy_id, healthy.as_mut(), &receiver, "approve-healthy",
    );

    dispatch(
        &runtime,
        TransportEffectRequest::DisconnectListener {
            session_id: session.clone(),
            listener_id: kicked_id.clone(),
            reason_code: "host_removed_listener".to_owned(),
        },
        "kick-authorized",
    );
    wait_frame(kicked.as_mut(), |frame| {
        matches!(frame, ProtocolFrame::Control(ControlMessage::Disconnect(value))
            if value.listener_id == kicked_id)
    });
    delivered(&receiver);

    let stream = StreamId::new("desktop-kick-playback").expect("stream");
    let start = ProtocolFrame::Control(ControlMessage::StreamStart(StreamStart {
        session_id: session.clone(),
        stream_id: stream.clone(),
        host_start_time_ms: MonotonicMillis::new(1_000),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        current_entry_id: Some("still-playing".to_owned()),
        current_display_name: Some("Still playing".to_owned()),
    }));
    runtime.broadcast_frame(start.clone()).expect("broadcast Start");
    wait_frame(healthy.as_mut(), |frame| frame == &start);
    let pcm = ProtocolFrame::Audio(AudioDatagram {
        session_id: session.clone(),
        stream_id: stream,
        sequence: PacketSequence::new(0),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 240,
        first_sample_index: SampleIndex::new(0),
        host_presentation_time_ms: MonotonicMillis::new(1_000),
        payload: vec![0x23; 960],
    });
    runtime.broadcast_frame(pcm.clone()).expect("broadcast PCM");
    wait_frame(healthy.as_mut(), |frame| frame == &pcm);
    assert_eq!(
        kicked.counters().audio_datagrams_received,
        0,
        "a kicked TCP generation must not retain UDP playback access"
    );
    drop(kicked.shutdown());

    let mut replacement = join(&session, endpoint, kicked_id.as_str(), &receiver);
    approve(
        &runtime, &session, &kicked_id, replacement.as_mut(), &receiver, "approve-reconnect",
    );
    wait_frame(replacement.as_mut(), |frame| frame == &start);
    replacement.shutdown().expect("replacement shutdown");
    healthy.shutdown().expect("healthy shutdown");
    runtime.shutdown().expect("host shutdown");
}

#[test]
fn pending_disconnect_effect_still_delivers_terminal_control_without_udp_authorization() {
    let (runtime, session, endpoint, receiver) = setup();
    let id = DeviceId::new("desktop-pending-kick").expect("ID");
    let mut pending = join(&session, endpoint, id.as_str(), &receiver);
    dispatch(
        &runtime,
        TransportEffectRequest::DisconnectListener {
            session_id: session.clone(),
            listener_id: id.clone(),
            reason_code: "host_cancelled_pending".to_owned(),
        },
        "kick-pending",
    );
    wait_frame(pending.as_mut(), |frame| {
        matches!(frame, ProtocolFrame::Control(ControlMessage::Disconnect(value))
            if value.listener_id == id)
    });
    delivered(&receiver);
    assert_eq!(pending.counters().audio_datagrams_received, 0);
    assert!(runtime.status().expect("host status").running);
    pending.shutdown().expect("pending shutdown");
    runtime.shutdown().expect("host shutdown");
}
