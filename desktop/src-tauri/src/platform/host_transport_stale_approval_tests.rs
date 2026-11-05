//! A join approval is bound to the request generation that produced it. After
//! the same device reconnects, a delayed or repeated approval for the retired
//! request must fail visibly and must never admit the replacement connection.
use super::host_transport::DesktopHostTransportRuntime;
use super::host_transport_admission_tests::{
    RecordingSink, TEST_TIMEOUT, advertisement, connect_listener, send_join, wait_for_control,
};
use silent_disco_core::domain::{DeviceId, OperationId, RequestId, SessionId};
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame};
use silent_disco_core::runtime::{
    TransportEffect, TransportEffectRequest, TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportConfig, ListenerTransportNode, SystemTransportClock, TransportChannel,
    TransportErrorKind, TransportEvent, TransportFactory, production_transport_factory,
};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn recv_until(
    receiver: &mpsc::Receiver<CoreTransportEvent>,
    predicate: impl Fn(&CoreTransportEvent) -> bool,
) -> CoreTransportEvent {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = receiver
            .recv_timeout(remaining)
            .expect("expected transport event before timeout");
        if predicate(&event) {
            return event;
        }
    }
}

fn next_join_request_id(receiver: &mpsc::Receiver<CoreTransportEvent>) -> RequestId {
    match recv_until(receiver, |event| {
        matches!(event, CoreTransportEvent::JoinRequested(_))
    }) {
        CoreTransportEvent::JoinRequested(request) => request.request_id,
        other => panic!("expected join request, got {other:?}"),
    }
}

fn approval_effect(
    operation: &str,
    request_id: RequestId,
    session_id: &SessionId,
    listener_id: &DeviceId,
) -> TransportEffect {
    TransportEffect::new(
        OperationId::new(operation).expect("operation"),
        TransportEffectRequest::DeliverJoinApproval {
            request_id,
            session_id: session_id.clone(),
            listener_id: listener_id.clone(),
            trusted_for_future: false,
        },
    )
    .expect("effect")
}

fn assert_no_join_approval(listener: &mut dyn ListenerTransportNode, window: Duration) {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        match listener.recv_event(Duration::from_millis(50)) {
            Ok(TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(_)),
                ..
            }) => panic!("a stale approval admitted the replacement connection"),
            Ok(_) => {}
            Err(error) if error.kind == TransportErrorKind::Timeout => {}
            Err(error) => panic!("listener transport failed: {error}"),
        }
    }
}

fn delivered_peers(event: &CoreTransportEvent) -> Option<u32> {
    match event {
        CoreTransportEvent::DeliveryCompleted { report, .. } => Some(report.successful_peers),
        _ => None,
    }
}

#[test]
fn stale_request_approval_after_reconnect_fails_and_current_request_still_admits() {
    let factory = production_transport_factory();
    let session_id = SessionId::new("stale-approval-session").expect("session");
    let host_id = DeviceId::new("stale-approval-host").expect("host");
    let listener_id = DeviceId::new("stale-approval-listener").expect("listener");
    let host_clock = Arc::new(SystemTransportClock::default());
    let node = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            host_clock.clone(),
        )
        .expect("bind host");
    let endpoint = node.endpoint();
    let (sender, receiver) = mpsc::channel();
    let runtime = DesktopHostTransportRuntime::start(
        node,
        advertisement(&session_id, &host_id),
        RecordingSink::new(sender),
        host_clock,
    )
    .expect("runtime");

    // Generation 1: join, approve, and consume the delivery report.
    let second_endpoint = endpoint;
    let mut first = connect_listener(&factory, &session_id, &listener_id, endpoint);
    send_join(&*first, &session_id, &listener_id);
    let first_request = next_join_request_id(&receiver);
    runtime
        .dispatch(approval_effect(
            "approve-first",
            first_request.clone(),
            &session_id,
            &listener_id,
        ))
        .expect("dispatch first approval");
    wait_for_control(&mut *first, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    let first_delivery = recv_until(&receiver, |event| delivered_peers(event).is_some());
    assert_eq!(delivered_peers(&first_delivery), Some(1));

    // Generation 2: the same device reconnects over a new TCP connection and
    // is assigned a distinct request generation.
    let mut second = connect_listener(&factory, &session_id, &listener_id, second_endpoint);
    send_join(&*second, &session_id, &listener_id);
    let second_request = next_join_request_id(&receiver);
    assert_ne!(first_request, second_request);

    // A delayed or repeated approval for the retired request fails visibly and
    // admits nothing.
    runtime
        .dispatch(approval_effect(
            "approve-stale",
            first_request,
            &session_id,
            &listener_id,
        ))
        .expect("stale dispatch is reported through delivery, not a crash");
    let stale = recv_until(&receiver, |event| delivered_peers(event).is_some());
    assert_eq!(delivered_peers(&stale), Some(0));
    assert_no_join_approval(&mut *second, Duration::from_millis(400));

    // The current generation is still approvable, so the guard refuses only
    // the stale request rather than blocking the reconnect.
    runtime
        .dispatch(approval_effect(
            "approve-current",
            second_request,
            &session_id,
            &listener_id,
        ))
        .expect("dispatch current approval");
    wait_for_control(&mut *second, |message| {
        matches!(message, ControlMessage::JoinApproval(_))
    });
    let current = recv_until(&receiver, |event| delivered_peers(event).is_some());
    assert_eq!(delivered_peers(&current), Some(1));

    drop(first.shutdown());
    second.shutdown().expect("replacement shutdown");
    runtime.shutdown().expect("runtime shutdown");
}
