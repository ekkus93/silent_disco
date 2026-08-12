//! Focused Block 26 transport-queue failure regression coverage.

use super::host_transport::DesktopHostTransportRuntime;
use silent_disco_core::domain::{ApprovalMode, DeviceId, MonotonicMillis, StreamId};
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame, Stop};
use silent_disco_core::runtime::{CoreActorConfig, CoreActorRuntime, SessionAdvertisement};
use silent_disco_core::transport::{
    HostTransportConfig, SystemTransportClock, TransportFactory, production_transport_factory,
};
use std::sync::Arc;
use std::sync::mpsc::channel;

#[test]
fn a_full_broadcast_queue_is_a_visible_resource_limit_failure() {
    let (sender, receiver) = channel();
    let actor = CoreActorRuntime::start(
        CoreActorConfig::new(DeviceId::new("desktop-broadcast-full").expect("host ID")),
        move |notification| {
            sender.send(notification).expect("observer receiver");
            Ok(())
        },
    )
    .expect("start actor");
    let handle = actor.handle();
    let _notifications = receiver;

    let advertisement = SessionAdvertisement::new(
        silent_disco_core::domain::SessionId::new("session-broadcast-full").expect("session ID"),
        DeviceId::new("desktop-broadcast-full").expect("device ID"),
        "Broadcast full",
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
        Arc::new(handle),
        Arc::new(SystemTransportClock::default()),
    )
    .expect("start desktop transport worker");
    let frame = ProtocolFrame::Control(ControlMessage::Stop(Stop {
        session_id: advertisement.session_id,
        stream_id: StreamId::new("stream-broadcast-full").expect("stream ID"),
        host_stop_time_ms: MonotonicMillis::new(0),
    }));

    let mut failure = None;
    for _ in 0..10_000 {
        if let Err(error) = runtime.broadcast_frame(frame.clone()) {
            failure = Some(error);
            break;
        }
    }
    let error = failure.expect("a bounded broadcast queue must eventually report saturation");
    assert!(
        error.to_string().contains("broadcast queue is full"),
        "unexpected saturation error: {error}"
    );
    assert!(
        runtime
            .status()
            .expect("transport status")
            .broadcast
            .queue_overflows
            > 0,
        "queue saturation must increment overflow diagnostics"
    );

    runtime.shutdown().expect("desktop transport shutdown");
    actor.shutdown().expect("actor shutdown");
}
