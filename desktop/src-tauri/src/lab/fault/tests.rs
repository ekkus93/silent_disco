use super::{LabFaultController, LabLatencyConfig, LabLatencyTransportFactory};
use crate::lab::clock::{LabClock, LabNodeClock};
use silent_disco_core::domain::MonotonicMillis;
use silent_disco_core::domain::{DeviceId, PacketSequence, SampleIndex, SessionId, StreamId};
use silent_disco_core::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
};
use silent_disco_core::transport::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    TransportChannel, TransportClock, TransportErrorKind, TransportEvent, TransportFactory,
    VirtualTransportFactory, VirtualTransportNetwork,
};
use std::sync::Arc;
use std::time::Duration;

/// Short and real -- only used to bound "is anything new on the
/// underlying channel" checks; never scaled by virtual scenario time.
const POLL_TIMEOUT: Duration = Duration::from_millis(25);

fn audio_frame(session_id: &SessionId, sequence: u64) -> ProtocolFrame {
    ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id.clone(),
        stream_id: StreamId::new("lab-latency-stream").expect("test stream ID is valid"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 2,
        first_sample_index: SampleIndex::new(sequence.saturating_mul(2)),
        host_presentation_time_ms: MonotonicMillis::new(sequence),
        payload: vec![0, 0, 1, 0, 2, 0, 3, 0],
    })
}

fn bind_and_connect(
    config: LabLatencyConfig,
) -> (
    Arc<LabClock>,
    Box<dyn HostTransportNode>,
    Box<dyn ListenerTransportNode>,
    SessionId,
) {
    bind_and_connect_with_listener_clock(config, 0, 0)
}

fn bind_and_connect_with_listener_clock(
    config: LabLatencyConfig,
    listener_offset_ms: i64,
    listener_drift_ppm: i64,
) -> (
    Arc<LabClock>,
    Box<dyn HostTransportNode>,
    Box<dyn ListenerTransportNode>,
    SessionId,
) {
    let session_id = SessionId::new("lab-latency-session").expect("test session ID is valid");
    let device_id = DeviceId::new("lab-latency-listener").expect("test device ID is valid");
    let clock = Arc::new(LabClock::new(1_000));
    let inner = VirtualTransportFactory::new(VirtualTransportNetwork::default());
    let factory = LabLatencyTransportFactory::new(inner, Arc::clone(&clock), config);

    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock_handle(&clock),
        )
        .expect("host should bind");
    let listener_clock: Arc<dyn TransportClock> = Arc::new(
        LabNodeClock::new(Arc::clone(&clock), listener_offset_ms, listener_drift_ppm)
            .expect("listener test clock should be valid"),
    );
    let listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            listener_clock,
        )
        .expect("listener should connect");

    assert!(matches!(
        host.recv_event(POLL_TIMEOUT)
            .expect("host should observe accepted peer"),
        TransportEvent::PeerAccepted { .. }
    ));
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: device_id.clone(),
                display_name: "Lab Latency Listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("join request should send");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT)
            .expect("host should receive join request"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            ..
        }
    ));
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("listener routes should authorize");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT)
            .expect("host should observe authorization"),
        TransportEvent::PeerAuthorized { .. }
    ));

    (clock, host, listener, session_id)
}

/// A zero-offset/drift `TransportClock` handle over the shared virtual
/// timeline for the host side of these tests.
fn clock_handle(clock: &Arc<LabClock>) -> Arc<dyn TransportClock> {
    Arc::new(LabNodeClock::new(Arc::clone(clock), 0, 0).expect("zero offset/drift is always valid"))
}

/// Block 39.3 "exact fixed latency": a held event is not delivered before
/// its deadline, and is delivered exactly once virtual time reaches it --
/// proven by advancing by one millisecond less than the configured
/// latency first (still held), then by exactly the remainder (released).
#[test]
fn exact_fixed_latency_holds_until_the_precise_deadline() {
    let (clock, mut host, mut listener, session_id) = bind_and_connect(LabLatencyConfig {
        fixed_latency_ms: 100,
        jitter_ms: 0,
        seed: 0,
    });

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("send should report local delivery");

    let Err(not_yet) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("event must be held before any virtual time has passed");
    };
    assert_eq!(not_yet.kind, TransportErrorKind::Timeout);

    clock.advance(99).expect("advance short of the deadline");
    let Err(still_not_yet) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("event must still be held one millisecond short of its deadline");
    };
    assert_eq!(still_not_yet.kind, TransportErrorKind::Timeout);

    clock.advance(1).expect("advance to the exact deadline");
    match listener
        .recv_event(POLL_TIMEOUT)
        .expect("event must be released exactly at its deadline")
    {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            received_at,
            ..
        } => assert_eq!(received_at, MonotonicMillis::new(1_100)),
        _ => panic!("expected delayed audio frame"),
    }

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Polling the wrapped transport for the first time at the deadline must
/// still expose the faulted delivery timestamp, not the underlying
/// pre-latency timestamp. This exercises the direct-release path rather
/// than the held-queue path above.
#[test]
fn first_poll_at_deadline_uses_the_faulted_delivery_timestamp() {
    let (clock, mut host, mut listener, session_id) = bind_and_connect(LabLatencyConfig {
        fixed_latency_ms: 100,
        jitter_ms: 0,
        seed: 0,
    });

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("send should report local delivery");
    clock
        .advance(100)
        .expect("advance directly to the deadline");

    match listener
        .recv_event(POLL_TIMEOUT)
        .expect("event must be deliverable at its deadline")
    {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            received_at,
            ..
        } => assert_eq!(received_at, MonotonicMillis::new(1_100)),
        _ => panic!("expected delayed audio frame"),
    }

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Listener-local clock skew must change only the externally visible
/// `received_at` timestamp. The 100 ms network-delay deadline itself
/// remains on the shared Lab timeline and therefore still releases at
/// base time `1_100` even with both offset and drift configured.
#[test]
fn listener_clock_offset_and_drift_do_not_distort_latency_deadline() {
    let (clock, mut host, mut listener, session_id) = bind_and_connect_with_listener_clock(
        LabLatencyConfig {
            fixed_latency_ms: 100,
            jitter_ms: 0,
            seed: 0,
        },
        500,
        10_000,
    );

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("send should report local delivery");
    let Err(not_yet) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("skewed listener clock must not make latency release early");
    };
    assert_eq!(not_yet.kind, TransportErrorKind::Timeout);

    clock.advance(99).expect("advance short of base deadline");
    let Err(still_not_yet) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("listener offset/drift must not move the base latency deadline");
    };
    assert_eq!(still_not_yet.kind, TransportErrorKind::Timeout);

    clock.advance(1).expect("advance to base deadline");
    match listener
        .recv_event(POLL_TIMEOUT)
        .expect("event must release at the base-domain deadline")
    {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            received_at,
            ..
        } => {
            // Base 1_100 with +1% drift is 1_111, then +500 ms offset.
            assert_eq!(received_at, MonotonicMillis::new(1_611));
        }
        _ => panic!("expected delayed audio frame"),
    }

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Jitter of `0` is exact latency with no seed-dependent variation --
/// zero-fault-adjacent parity for this wrapper specifically.
#[test]
fn zero_jitter_is_exact_latency_regardless_of_seed() {
    let (clock, mut host, mut listener, session_id) = bind_and_connect(LabLatencyConfig {
        fixed_latency_ms: 50,
        jitter_ms: 0,
        seed: 12_345,
    });

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("send should report local delivery");
    clock.advance(50).expect("advance to the deadline");
    assert!(matches!(
        listener
            .recv_event(POLL_TIMEOUT)
            .expect("event must be released exactly at its deadline"),
        TransportEvent::FrameReceived { .. }
    ));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Jitter bounds the release deadline to `[latency - jitter, latency +
/// jitter]`: advancing to the lower bound never releases early beyond
/// that window, and advancing to the upper bound always releases.
#[test]
fn jitter_keeps_the_deadline_within_its_configured_bound() {
    let (clock, mut host, mut listener, session_id) = bind_and_connect(LabLatencyConfig {
        fixed_latency_ms: 100,
        jitter_ms: 20,
        seed: 7,
    });

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("send should report local delivery");

    // Below the lowest possible deadline (100 - 20): must never be due.
    clock.advance(79).expect("advance below the jitter window");
    let Err(too_early) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("event must not be released before the earliest possible deadline");
    };
    assert_eq!(too_early.kind, TransportErrorKind::Timeout);

    // At the highest possible deadline (100 + 20): must always be due by now.
    clock
        .advance(41)
        .expect("advance to the latest possible deadline");
    assert!(matches!(
        listener
            .recv_event(POLL_TIMEOUT)
            .expect("event must be released by the latest possible deadline"),
        TransportEvent::FrameReceived { .. }
    ));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Control-channel events are never delayed, even with latency
/// configured -- only the datagram channels are in scope (matching the
/// shared core's own fault model).
#[test]
fn control_channel_events_are_never_delayed() {
    let (_clock, mut host, mut listener, session_id) = bind_and_connect(LabLatencyConfig {
        fixed_latency_ms: 10_000,
        jitter_ms: 0,
        seed: 0,
    });

    host.send_pending_control(
        &silent_disco_core::domain::DeviceId::new("lab-latency-listener").expect("device id"),
        &ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: DeviceId::new("lab-latency-listener").expect("device id"),
                display_name: "Lab Latency Listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }),
    )
    .expect("control send should succeed");
    assert!(matches!(
        listener
            .recv_event(POLL_TIMEOUT)
            .expect("a control-channel event must arrive immediately, undelayed"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            ..
        }
    ));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

#[test]
fn live_controller_changes_loss_without_reconnecting_the_listener() {
    let session_id = SessionId::new("lab-dynamic-fault-session").expect("session ID");
    let device_id = DeviceId::new("lab-dynamic-fault-listener").expect("device ID");
    let clock = Arc::new(LabClock::new(2_000));
    let controller = LabFaultController::new(
        LabLatencyConfig {
            fixed_latency_ms: 0,
            jitter_ms: 0,
            seed: 123,
        },
        0,
    );
    let factory = LabLatencyTransportFactory::new_dynamic(
        VirtualTransportFactory::new(VirtualTransportNetwork::default()),
        Arc::clone(&clock),
        controller.clone(),
    );
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock_handle(&clock),
        )
        .expect("host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock_handle(&clock),
        )
        .expect("listener should connect");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT).expect("peer accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: device_id.clone(),
                display_name: "Dynamic Fault Listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("join should send");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT).expect("join should arrive"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            ..
        }
    ));
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("listener should authorize");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT)
            .expect("authorization should arrive"),
        TransportEvent::PeerAuthorized { .. }
    ));

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("pre-mutation send should succeed");
    assert!(matches!(
        listener
            .recv_event(POLL_TIMEOUT)
            .expect("pre-mutation audio should arrive"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            ..
        }
    ));

    controller.update(0, 0, 1_000);
    host.broadcast_audio(&audio_frame(&session_id, 2))
        .expect("post-mutation UDP send should still report local success");
    let Err(dropped) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("100% live loss must drop the next audio datagram");
    };
    assert_eq!(dropped.kind, TransportErrorKind::Timeout);

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

#[test]
fn live_controller_changes_latency_without_reconnecting_the_listener() {
    let session_id = SessionId::new("lab-dynamic-latency-session").expect("session ID");
    let device_id = DeviceId::new("lab-dynamic-latency-listener").expect("device ID");
    let clock = Arc::new(LabClock::new(5_000));
    let controller = LabFaultController::new(
        LabLatencyConfig {
            fixed_latency_ms: 0,
            jitter_ms: 0,
            seed: 456,
        },
        0,
    );
    let factory = LabLatencyTransportFactory::new_dynamic(
        VirtualTransportFactory::new(VirtualTransportNetwork::default()),
        Arc::clone(&clock),
        controller.clone(),
    );
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock_handle(&clock),
        )
        .expect("host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock_handle(&clock),
        )
        .expect("listener should connect");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT).expect("peer accepted"),
        TransportEvent::PeerAccepted { .. }
    ));
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: device_id.clone(),
                display_name: "Dynamic Latency Listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("join should send");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT).expect("join should arrive"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            ..
        }
    ));
    host.authorize_peer(&device_id, listener.local_routes())
        .expect("listener should authorize");
    assert!(matches!(
        host.recv_event(POLL_TIMEOUT)
            .expect("authorization should arrive"),
        TransportEvent::PeerAuthorized { .. }
    ));

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("pre-mutation send should succeed");
    assert!(matches!(
        listener
            .recv_event(POLL_TIMEOUT)
            .expect("pre-mutation audio should arrive immediately"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            ..
        }
    ));

    controller.update(100, 0, 0);
    host.broadcast_audio(&audio_frame(&session_id, 2))
        .expect("post-mutation send should succeed");
    let Err(held) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("new 100ms latency must hold the next audio datagram");
    };
    assert_eq!(held.kind, TransportErrorKind::Timeout);

    clock
        .advance(99)
        .expect("advance just short of new latency deadline");
    let Err(still_held) = listener.recv_event(POLL_TIMEOUT) else {
        panic!("audio must remain held one millisecond before the deadline");
    };
    assert_eq!(still_held.kind, TransportErrorKind::Timeout);

    clock
        .advance(1)
        .expect("advance to exact new latency deadline");
    assert!(matches!(
        listener
            .recv_event(POLL_TIMEOUT)
            .expect("audio should release exactly at the mutated latency deadline"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            received_at,
            ..
        } if received_at == MonotonicMillis::new(5_100)
    ));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}
