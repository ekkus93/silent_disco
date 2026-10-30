use super::host_transport::DesktopHostTransportRuntime;
use super::host_transport_events::DesktopHostTransportEventSink;
use silent_disco_core::domain::{ApprovalMode, DeviceId, OperationId, RequestId, SessionId};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{
    ControlMessage, DeviceIdentity, JoinApproval, JoinRequest, ProtocolFrame,
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
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

struct RecordingSink {
    snapshot: Mutex<CoreSnapshot>,
    sender: mpsc::Sender<CoreTransportEvent>,
}

impl DesktopHostTransportEventSink for RecordingSink {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(self.snapshot.lock().expect("snapshot").clone())
    }

    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError> {
        self.sender.send(event).map_err(|_| {
            CoreError::new(
                silent_disco_core::error::CoreErrorCode::WorkerStopped,
                "transport test receiver closed",
                silent_disco_core::error::ErrorSeverity::Error,
                false,
                None,
            )
            .expect("static error")
        })
    }

    fn submit_audio_event(
        &self,
        _event: silent_disco_core::runtime::AudioEvent,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}

#[test]
fn approval_effect_delivers_to_pending_peer_and_reports_success() {
    let factory = production_transport_factory();
    let session_id = SessionId::new("block23-session").expect("session");
    let host_id = DeviceId::new("block23-host").expect("host");
    let listener_id = DeviceId::new("block23-listener").expect("listener");
    let advertisement = advertisement(&session_id, &host_id);
    let host_clock = Arc::new(SystemTransportClock::default());
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            host_clock.clone(),
        )
        .expect("bind host");
    let endpoint = node.endpoint();
    let (sender, receiver) = mpsc::channel();
    let sink = Arc::new(RecordingSink {
        snapshot: Mutex::new(CoreSnapshot::default()),
        sender,
    });
    let runtime =
        DesktopHostTransportRuntime::start(node, advertisement, sink, host_clock).expect("runtime");
    let mut listener = connect_listener(&factory, &session_id, &listener_id, endpoint);
    send_join(&*listener, &session_id, &listener_id);
    wait_for_control(&mut *listener, |message| {
        matches!(message, ControlMessage::Hello(_))
    });

    runtime
        .dispatch(
            TransportEffect::new(
                OperationId::new("approve-effect").expect("operation"),
                TransportEffectRequest::DeliverJoinApproval {
                    request_id: RequestId::new("request-1").expect("request"),
                    session_id: session_id.clone(),
                    listener_id: listener_id.clone(),
                    trusted_for_future: true,
                },
            )
            .expect("effect"),
        )
        .expect("dispatch");

    wait_for_control(&mut *listener, |message| {
        matches!(
            message,
            ControlMessage::JoinApproval(JoinApproval {
                listener_id: delivered,
                trusted_for_future: true,
                ..
            }) if delivered == &listener_id
        )
    });
    let delivery = wait_for_delivery(&receiver);
    assert!(matches!(
        delivery,
        CoreTransportEvent::DeliveryCompleted { report, .. }
            if report.intended_peers == 1
                && report.successful_peers == 1
                && report.failed_peers == 0
    ));
    let connected = receiver
        .recv_timeout(TEST_TIMEOUT)
        .expect("connected listener event");
    assert!(matches!(
        connected,
        CoreTransportEvent::ListenerConnected(listener)
            if listener.device_id == listener_id
                && listener.display_name == "Block 23 listener"
                && listener.trust_state == silent_disco_core::domain::TrustState::Trusted
                && listener.transport_state == silent_disco_core::domain::TransportState::Connected
    ));

    listener.shutdown().expect("listener shutdown");
    runtime.shutdown().expect("runtime shutdown");
}

#[test]
#[allow(clippy::too_many_lines)] // Full real-socket reconnect lifecycle is one acceptance scenario.
fn real_socket_same_id_reconnect_does_not_project_retired_peer_disconnect() {
    let factory = production_transport_factory();
    let session_id = SessionId::new("reconnect-projection-session").expect("session");
    let host_id = DeviceId::new("reconnect-projection-host").expect("host");
    let listener_id = DeviceId::new("reconnect-projection-listener").expect("listener");
    let advertisement = advertisement(&session_id, &host_id);
    let clock = Arc::new(SystemTransportClock::default());
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("bind host");
    let endpoint = node.endpoint();
    let (sender, receiver) = mpsc::channel();
    let sink = Arc::new(RecordingSink {
        snapshot: Mutex::new(CoreSnapshot::default()),
        sender,
    });
    let runtime =
        DesktopHostTransportRuntime::start(node, advertisement, sink, clock).expect("runtime");

    let mut first = connect_listener(&factory, &session_id, &listener_id, endpoint);
    send_join(&*first, &session_id, &listener_id);
    wait_for_control(&mut *first, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
    assert!(matches!(
        receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("first join event"),
        CoreTransportEvent::JoinRequested(_)
    ));
    runtime
        .dispatch(
            TransportEffect::new(
                OperationId::new("reconnect-approval-first").expect("operation"),
                TransportEffectRequest::DeliverJoinApproval {
                    request_id: RequestId::new("reconnect-request-first").expect("request"),
                    session_id: session_id.clone(),
                    listener_id: listener_id.clone(),
                    trusted_for_future: false,
                },
            )
            .expect("effect"),
        )
        .expect("approve first");
    wait_for_control(&mut *first, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    assert!(matches!(
        wait_for_delivery(&receiver),
        CoreTransportEvent::DeliveryCompleted { report, .. } if report.successful_peers == 1
    ));
    assert!(matches!(
        receiver.recv_timeout(TEST_TIMEOUT).expect("first connected"),
        CoreTransportEvent::ListenerConnected(listener) if listener.device_id == listener_id
    ));

    let mut replacement = connect_listener(&factory, &session_id, &listener_id, endpoint);
    send_join(&*replacement, &session_id, &listener_id);
    wait_for_control(&mut *replacement, |message| {
        matches!(message, ControlMessage::Hello(_))
    });
    assert!(matches!(
        receiver
            .recv_timeout(TEST_TIMEOUT)
            .expect("replacement join event"),
        CoreTransportEvent::JoinRequested(_)
    ));
    runtime
        .dispatch(
            TransportEffect::new(
                OperationId::new("reconnect-approval-second").expect("operation"),
                TransportEffectRequest::DeliverJoinApproval {
                    request_id: RequestId::new("reconnect-request-second").expect("request"),
                    session_id,
                    listener_id: listener_id.clone(),
                    trusted_for_future: false,
                },
            )
            .expect("effect"),
        )
        .expect("approve replacement");
    wait_for_control(&mut *replacement, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    assert!(matches!(
        wait_for_delivery(&receiver),
        CoreTransportEvent::DeliveryCompleted { report, .. } if report.successful_peers == 1
    ));
    assert!(matches!(
        receiver.recv_timeout(TEST_TIMEOUT).expect("replacement connected"),
        CoreTransportEvent::ListenerConnected(listener) if listener.device_id == listener_id
    ));

    // Native route replacement closes the original TCP peer. Its delayed
    // disconnect cannot evict the replacement's actor state. Allow the
    // worker to process the retired reader's terminal event.
    drop(first.shutdown());
    let deadline = Instant::now() + Duration::from_millis(500);
    while let Ok(event) = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        assert!(
            !matches!(
                event,
                CoreTransportEvent::ListenerDisconnected { device_id, .. }
                    if device_id == listener_id
            ),
            "retired socket disconnected the active replacement"
        );
        if Instant::now() >= deadline {
            break;
        }
    }

    replacement.shutdown().expect("replacement shutdown");
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "missing replacement disconnect");
        let event = receiver.recv_timeout(remaining).expect("terminal event");
        if let CoreTransportEvent::ListenerDisconnected { device_id, .. } = event {
            assert_eq!(device_id, listener_id);
            break;
        }
    }
    runtime.shutdown().expect("runtime shutdown");
}

#[test]
fn missing_pending_peer_reports_failed_delivery_without_stopping_worker() {
    let factory = production_transport_factory();
    let session_id = SessionId::new("block23-missing-session").expect("session");
    let host_id = DeviceId::new("block23-missing-host").expect("host");
    let advertisement = advertisement(&session_id, &host_id);
    let clock = Arc::new(SystemTransportClock::default());
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("bind host");
    let (sender, receiver) = mpsc::channel();
    let sink = Arc::new(RecordingSink {
        snapshot: Mutex::new(CoreSnapshot::default()),
        sender,
    });
    let runtime =
        DesktopHostTransportRuntime::start(node, advertisement, sink, clock).expect("runtime");
    runtime
        .dispatch(
            TransportEffect::new(
                OperationId::new("missing-effect").expect("operation"),
                TransportEffectRequest::DeliverJoinRejection {
                    request_id: RequestId::new("missing-request").expect("request"),
                    session_id,
                    listener_id: DeviceId::new("missing-listener").expect("listener"),
                    reason_code: "host_rejected".to_owned(),
                },
            )
            .expect("effect"),
        )
        .expect("queue effect");

    let delivery = receiver.recv_timeout(TEST_TIMEOUT).expect("delivery");
    assert!(matches!(
        delivery,
        CoreTransportEvent::DeliveryCompleted { report, .. }
            if report.intended_peers == 1
                && report.successful_peers == 0
                && report.failed_peers == 1
    ));
    assert!(runtime.status().expect("status").running);
    runtime.shutdown().expect("runtime shutdown");
}

fn advertisement(session_id: &SessionId, host_id: &DeviceId) -> SessionAdvertisement {
    SessionAdvertisement::new(
        session_id.clone(),
        host_id.clone(),
        "Block 23 host",
        ApprovalMode::Manual,
        silent_disco_core::protocol::PROTOCOL_VERSION,
        None,
    )
    .expect("advertisement")
}

fn connect_listener(
    factory: &dyn TransportFactory,
    session_id: &SessionId,
    listener_id: &DeviceId,
    endpoint: NetworkEndpoint,
) -> Box<dyn ListenerTransportNode> {
    factory
        .connect_listener(
            ListenerTransportConfig::loopback(session_id.clone(), listener_id.clone(), endpoint),
            Arc::new(SystemTransportClock::default()),
        )
        .expect("connect listener")
}

fn send_join(listener: &dyn ListenerTransportNode, session_id: &SessionId, listener_id: &DeviceId) {
    let routes = listener.local_routes();
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: listener_id.clone(),
                display_name: "Block 23 listener".to_owned(),
            },
            invite_code: None,
            sync_port: routes.synchronization.port(),
            audio_port: routes.audio.port(),
        }))
        .expect("send join");
}

fn wait_for_delivery(receiver: &mpsc::Receiver<CoreTransportEvent>) -> CoreTransportEvent {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = receiver
            .recv_timeout(remaining)
            .expect("delivery event before timeout");
        if matches!(event, CoreTransportEvent::DeliveryCompleted { .. }) {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for delivery event"
        );
    }
}

fn wait_for_control(
    listener: &mut dyn ListenerTransportNode,
    predicate: impl Fn(&ControlMessage) -> bool,
) {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        match listener.recv_event(Duration::from_millis(100)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(message),
                ..
            }) if predicate(&message) => return,
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
        assert!(Instant::now() < deadline, "timed out waiting for control");
    }
}

// A full or unavailable authoritative actor must never silently strand an
// accepted TCP listener, nor terminate unrelated desktop-host playback.
struct RejectFirstJoin {
    attempts: std::sync::atomic::AtomicUsize,
    accepted: mpsc::Sender<CoreTransportEvent>,
}

impl DesktopHostTransportEventSink for RejectFirstJoin {
    fn current_snapshot(&self) -> Result<CoreSnapshot, CoreError> {
        Ok(CoreSnapshot::default())
    }

    fn submit_transport_event(&self, event: CoreTransportEvent) -> Result<(), CoreError> {
        if self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            return Err(CoreError::new(
                silent_disco_core::error::CoreErrorCode::WorkerStopped,
                "simulated actor mailbox refusal",
                silent_disco_core::error::ErrorSeverity::Error,
                false,
                None,
            )
            .expect("static error"));
        }
        self.accepted.send(event).expect("test observer");
        Ok(())
    }

    fn submit_audio_event(
        &self,
        _event: silent_disco_core::runtime::AudioEvent,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}

#[test]
fn rejected_actor_join_gets_explicit_tcp_refusal_and_next_listener_can_join() {
    let factory = production_transport_factory();
    let session = SessionId::new("desktop-join-refusal-recovery").expect("session");
    let host = DeviceId::new("desktop-refusal-host").expect("host");
    let advertisement = advertisement(&session, &host);
    let clock = Arc::new(SystemTransportClock::default());
    let node = factory
        .bind_host(HostTransportConfig::loopback(session.clone()), clock.clone())
        .expect("bind real host TCP");
    let endpoint = node.endpoint();
    let (tx, rx) = mpsc::channel();
    let sink = Arc::new(RejectFirstJoin {
        attempts: std::sync::atomic::AtomicUsize::new(0),
        accepted: tx,
    });
    let runtime = DesktopHostTransportRuntime::start(
        node, advertisement, sink.clone(), clock,
    ).expect("start host worker");

    let mut refused = connect_listener(
        &factory,
        &session,
        &DeviceId::new("desktop-first-refused").expect("listener"),
        endpoint,
    );
    send_join(
        refused.as_ref(),
        &session,
        &DeviceId::new("desktop-first-refused").expect("listener"),
    );
    wait_for_control(refused.as_mut(), |message| {
        matches!(
            message,
            ControlMessage::JoinRejection(denial)
                if denial.reason == "host_busy"
                    && denial.listener_id.as_str() == "desktop-first-refused"
        )
    });
    assert!(runtime.status().expect("status").running);

    let second_id = DeviceId::new("desktop-second-accepted").expect("listener");
    let mut accepted = connect_listener(&factory, &session, &second_id, endpoint);
    send_join(accepted.as_ref(), &session, &second_id);
    wait_for_control(accepted.as_mut(), |message| matches!(message, ControlMessage::Hello(_)));
    let event = rx.recv_timeout(TEST_TIMEOUT).expect("second actor join");
    assert!(matches!(
        event,
        CoreTransportEvent::JoinRequested(request) if request.device_id == second_id
    ));
    assert_eq!(
        sink.attempts.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert!(runtime.status().expect("status").running);

    refused.shutdown().expect("refused socket shutdown");
    accepted.shutdown().expect("accepted socket shutdown");
    runtime.shutdown().expect("host shutdown");
}
