use std::sync::Arc;
use std::time::Duration;

use crate::domain::{DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
    SyncRequest, encode_frame,
};

use super::{
    FaultInjectingVirtualTransportFactory, HostTransportConfig, HostTransportNode,
    ListenerTransportConfig, ListenerTransportNode, ManualTransportClock, TransportChannel,
    TransportErrorKind, TransportEvent, TransportFactory, VirtualTransportFactory,
    VirtualTransportNetwork, VirtualUdpFaultConfig,
};

const EVENT_TIMEOUT: Duration = Duration::from_secs(1);
const LOSS_TIMEOUT: Duration = Duration::from_millis(25);

#[test]
fn virtual_udp_faults_drop_sync_and_reorder_audio_without_changing_send_reports() {
    let session_id = SessionId::new("fault-session").expect("test session ID is valid");
    let device_id = DeviceId::new("fault-listener").expect("test device ID is valid");
    let factory = VirtualTransportFactory::new(VirtualTransportNetwork::default()).with_udp_faults(
        VirtualUdpFaultConfig {
            drop_next_sync_events: 1,
            reorder_next_audio_pair: true,
            ..VirtualUdpFaultConfig::default()
        },
    );
    let clock = Arc::new(ManualTransportClock::new(4_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("virtual host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("virtual listener should connect");
    authorize_listener(&mut *host, &mut *listener, &session_id, &device_id);

    let first_sync = SyncRequest {
        session_id: session_id.clone(),
        correlation_id: 1,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(4_001),
    };
    let first_delivery = listener
        .send_sync_request(&first_sync)
        .expect("virtual sync send should report local delivery");
    assert_eq!(first_delivery.report.successful_peers, 1);
    let Err(loss_error) = host.recv_event(LOSS_TIMEOUT) else {
        panic!("first synchronization event should be dropped");
    };
    assert_eq!(loss_error.kind, TransportErrorKind::Timeout);

    let second_sync = SyncRequest {
        session_id: session_id.clone(),
        correlation_id: 2,
        t1_listener_send_elapsed_ms: MonotonicMillis::new(4_002),
    };
    listener
        .send_sync_request(&second_sync)
        .expect("second synchronization send should succeed");
    assert!(matches!(
        host.recv_event(EVENT_TIMEOUT)
            .expect("second synchronization event should arrive"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Synchronization,
            frame: ProtocolFrame::SyncRequest(value),
            ..
        } if value == second_sync
    ));

    let first_audio = audio_frame(&session_id, 10);
    let second_audio = audio_frame(&session_id, 11);
    assert_eq!(
        host.broadcast_audio(&first_audio)
            .expect("first audio send should report local delivery")
            .report
            .successful_peers,
        1
    );
    assert_eq!(
        host.broadcast_audio(&second_audio)
            .expect("second audio send should report local delivery")
            .report
            .successful_peers,
        1
    );
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(11));
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(10));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.3 "zero-fault parity": a fault-injecting factory configured
/// with every fault at its zero/disabled default behaves exactly like the
/// unwrapped virtual transport -- one send, one receive, no drop,
/// duplication, or reordering.
#[test]
fn zero_fault_parity_behaves_like_the_unfaulted_virtual_transport() {
    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig::default());

    let frame = audio_frame(&session_id, 1);
    assert_eq!(
        host.broadcast_audio(&frame)
            .expect("send should succeed")
            .report
            .successful_peers,
        1
    );
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(1));
    let Err(no_more) = listener.recv_event(LOSS_TIMEOUT) else {
        panic!("exactly one event should have been delivered, not duplicated");
    };
    assert_eq!(no_more.kind, TransportErrorKind::Timeout);

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.3 "deterministic loss sequence" / "identical seed produces
/// identical trace": the exact same seed, driven through the exact same
/// sequence of sends, drops the exact same sequence numbers every time.
#[test]
fn identical_seed_produces_an_identical_loss_sequence() {
    let first_trace = capture_loss_trace(7);
    let second_trace = capture_loss_trace(7);
    assert_eq!(first_trace, second_trace);
    // A real, observable fault: the seed above is not simply "never
    // drops anything".
    assert!(first_trace.contains(&false));
}

/// Block 39.3 "different seed changes trace where expected": changing
/// only the seed (same probability, same send sequence) changes which
/// sends survive.
#[test]
fn a_different_seed_changes_the_loss_trace() {
    let trace_a = capture_loss_trace(7);
    let trace_b = capture_loss_trace(99);
    assert_ne!(trace_a, trace_b);
}

/// Returns, in send order, whether each of 20 audio sends survived
/// (`true`) or was lost (`false`) under a 50% seeded loss probability.
fn capture_loss_trace(seed: u64) -> Vec<bool> {
    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig {
            loss_permille: 500,
            seed,
            ..VirtualUdpFaultConfig::default()
        });
    let mut trace = Vec::new();
    for sequence in 0..20 {
        host.broadcast_audio(&audio_frame(&session_id, sequence))
            .expect("send should report local delivery regardless of later loss");
        match listener.recv_event(LOSS_TIMEOUT) {
            Ok(TransportEvent::FrameReceived { .. }) => trace.push(true),
            Err(error) if error.kind == TransportErrorKind::Timeout => trace.push(false),
            Ok(_) => panic!("unexpected non-frame event"),
            Err(error) => panic!("unexpected non-timeout error: {error}"),
        }
    }
    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
    trace
}

/// Block 39.3 "duplicate detection": a always-duplicate configuration
/// delivers the same send twice, back to back.
#[test]
fn duplication_delivers_the_same_send_twice() {
    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig {
            duplicate_permille: 1_000,
            ..VirtualUdpFaultConfig::default()
        });

    host.broadcast_audio(&audio_frame(&session_id, 3))
        .expect("send should succeed");
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(3));
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(3));
    let Err(no_third) = listener.recv_event(LOSS_TIMEOUT) else {
        panic!("a duplicate must be delivered exactly twice, not three times");
    };
    assert_eq!(no_third.kind, TransportErrorKind::Timeout);

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.3 "reorder window": a bounded reorder buffer releases events
/// in a scrambled -- but deterministic and never-unbounded -- order, not
/// strict FIFO send order.
#[test]
fn reorder_window_releases_events_out_of_fifo_order_deterministically() {
    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig {
            reorder_window: 4,
            seed: 123,
            ..VirtualUdpFaultConfig::default()
        });

    // The buffer holds up to `reorder_window` events before releasing
    // any; sending exactly that many guarantees exactly one release.
    for sequence in 0..4 {
        host.broadcast_audio(&audio_frame(&session_id, sequence))
            .expect("send should succeed");
    }
    let released = recv_audio_sequence(&mut *listener);
    // Deterministic given the fixed seed above -- if this ever needs to
    // change because the PRNG's own algorithm changes, that is exactly
    // the kind of change that should be visible in a test diff.
    assert_eq!(released, PacketSequence::new(1));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.3 "malformed/corrupt packet diagnostics": a corrupted audio
/// send fails with a real, diagnosable protocol-level error -- the same
/// `decode_frame` path and `IntegrityMismatch`-derived classification
/// proven directly against the codec in `protocol::codec::tests` and
/// `protocol::vector_tests`, reached this time through the fault-injecting
/// transport rather than by calling the codec directly.
#[test]
fn corruption_produces_a_real_diagnosable_protocol_error() {
    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig {
            corrupt_next_events: 1,
            ..VirtualUdpFaultConfig::default()
        });

    let error = host
        .broadcast_audio(&audio_frame(&session_id, 1))
        .expect_err("a corrupted audio send must fail, not silently succeed");
    assert_eq!(error.kind, TransportErrorKind::Protocol);

    // The fault only applies to the configured count -- the very next
    // send is uncorrupted and arrives normally.
    host.broadcast_audio(&audio_frame(&session_id, 2))
        .expect("the next send is not corrupted");
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(2));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.3 "backpressure": a saturated event queue is reported as
/// `QueueFull`, never silently swallowed by fault processing sitting in
/// front of it.
#[test]
fn a_saturated_queue_is_reported_not_swallowed_by_fault_processing() {
    let session_id = SessionId::new("fault-backpressure").expect("test session ID is valid");
    let device_id = DeviceId::new("fault-backpressure-listener").expect("test device ID is valid");
    let factory = VirtualTransportFactory::new(VirtualTransportNetwork::default())
        .with_udp_faults(VirtualUdpFaultConfig::default());
    let clock = Arc::new(ManualTransportClock::new(1_000));
    let mut host_config = HostTransportConfig::loopback(session_id.clone());
    host_config.event_queue_capacity = 1;
    let mut host = factory
        .bind_host(host_config, clock.clone())
        .expect("virtual host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("virtual listener should connect");
    authorize_listener(&mut *host, &mut *listener, &session_id, &device_id);
    // `authorize_listener` already drained the host's tiny one-slot queue
    // down to empty by consuming PeerAccepted/FrameReceived/PeerAuthorized
    // as they arrived, so it starts genuinely empty here.

    listener
        .send_sync_request(&SyncRequest {
            session_id: session_id.clone(),
            correlation_id: 1,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1_001),
        })
        .expect("first sync send fills the one-slot queue");
    let overflow = listener
        .send_sync_request(&SyncRequest {
            session_id: session_id.clone(),
            correlation_id: 2,
            t1_listener_send_elapsed_ms: MonotonicMillis::new(1_002),
        })
        .expect_err("a second send into a full one-slot queue must be reported");
    assert_eq!(overflow.kind, TransportErrorKind::QueueFull);

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.3 "disconnect": once the configured event count is reached,
/// every later event on that channel is replaced by a synthesized
/// disconnect instead of being delivered.
#[test]
fn disconnect_after_events_replaces_later_events_with_a_synthesized_disconnect() {
    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig {
            disconnect_after_events: Some(1),
            ..VirtualUdpFaultConfig::default()
        });

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("first send should succeed");
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(1));

    host.broadcast_audio(&audio_frame(&session_id, 2))
        .expect("second send should still report local delivery");
    assert!(matches!(
        listener
            .recv_event(EVENT_TIMEOUT)
            .expect("a synthesized disconnect must still be delivered"),
        TransportEvent::PeerDisconnected { .. }
    ));

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.2 "connection refusal": a scripted refusal count rejects
/// exactly that many `connect_listener` attempts before allowing any
/// through.
#[test]
fn scripted_connection_refusal_rejects_exactly_the_configured_count() {
    let session_id = SessionId::new("fault-refusal").expect("test session ID is valid");
    let network = VirtualTransportNetwork::default();
    let factory = VirtualTransportFactory::new(network);
    let clock = Arc::new(ManualTransportClock::new(1_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("virtual host should bind");
    let faulted: FaultInjectingVirtualTransportFactory = factory
        .with_udp_faults(VirtualUdpFaultConfig::default())
        .with_connection_refusals(1);

    let device_id = DeviceId::new("fault-refused-listener").expect("test device ID is valid");
    let Err(refused) = faulted.connect_listener(
        ListenerTransportConfig::loopback(session_id.clone(), device_id.clone(), host.endpoint()),
        clock.clone(),
    ) else {
        panic!("the first connection attempt must be refused");
    };
    assert_eq!(refused.kind, TransportErrorKind::Connect);

    let mut listener = faulted
        .connect_listener(
            ListenerTransportConfig::loopback(session_id, device_id, host.endpoint()),
            clock,
        )
        .expect("the second connection attempt must succeed");

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

/// Block 39.2 "bandwidth limit": once a channel's cumulative encoded-byte
/// budget is exceeded, further sends on that channel are dropped.
#[test]
fn bandwidth_limit_drops_sends_once_the_byte_budget_is_exceeded() {
    let session_id_for_sizing = SessionId::new("fault-scenario").expect("test session ID is valid");
    let one_frame_bytes = encode_frame(&audio_frame(&session_id_for_sizing, 0))
        .expect("audio frame should encode")
        .len();
    // Enough for exactly one encoded audio frame plus a margin below two,
    // so the first send fits and the second (same-size) send does not.
    let budget = u64::try_from(one_frame_bytes).expect("frame size fits in u64") + 1;

    let (mut host, mut listener, session_id, _device_id) =
        bind_and_connect(VirtualUdpFaultConfig {
            bandwidth_limit_bytes: Some(budget),
            ..VirtualUdpFaultConfig::default()
        });

    host.broadcast_audio(&audio_frame(&session_id, 1))
        .expect("first send should report local delivery");
    assert_eq!(recv_audio_sequence(&mut *listener), PacketSequence::new(1));

    host.broadcast_audio(&audio_frame(&session_id, 2))
        .expect("second send should still report local delivery");
    let Err(over_budget) = listener.recv_event(LOSS_TIMEOUT) else {
        panic!("a send beyond the byte budget must be dropped");
    };
    assert_eq!(over_budget.kind, TransportErrorKind::Timeout);

    listener.shutdown().expect("listener should shut down");
    host.shutdown().expect("host should shut down");
}

fn bind_and_connect(
    config: VirtualUdpFaultConfig,
) -> (
    Box<dyn HostTransportNode>,
    Box<dyn ListenerTransportNode>,
    SessionId,
    DeviceId,
) {
    let session_id = SessionId::new("fault-scenario").expect("test session ID is valid");
    let device_id = DeviceId::new("fault-scenario-listener").expect("test device ID is valid");
    let factory =
        VirtualTransportFactory::new(VirtualTransportNetwork::default()).with_udp_faults(config);
    let clock = Arc::new(ManualTransportClock::new(1_000));
    let mut host = factory
        .bind_host(
            HostTransportConfig::loopback(session_id.clone()),
            clock.clone(),
        )
        .expect("virtual host should bind");
    let mut listener = factory
        .connect_listener(
            ListenerTransportConfig::loopback(
                session_id.clone(),
                device_id.clone(),
                host.endpoint(),
            ),
            clock,
        )
        .expect("virtual listener should connect");
    authorize_listener(&mut *host, &mut *listener, &session_id, &device_id);
    (host, listener, session_id, device_id)
}

fn authorize_listener(
    host: &mut dyn HostTransportNode,
    listener: &mut dyn ListenerTransportNode,
    session_id: &SessionId,
    device_id: &DeviceId,
) {
    assert!(matches!(
        host.recv_event(EVENT_TIMEOUT)
            .expect("host should observe accepted virtual peer"),
        TransportEvent::PeerAccepted { .. }
    ));
    listener
        .send_control(&ControlMessage::JoinRequest(JoinRequest {
            session_id: session_id.clone(),
            device: DeviceIdentity {
                device_id: device_id.clone(),
                display_name: "Fault Listener".to_owned(),
            },
            invite_code: None,
            sync_port: 0,
            audio_port: 0,
        }))
        .expect("join request should not be faulted");
    assert!(matches!(
        host.recv_event(EVENT_TIMEOUT)
            .expect("host should receive join request"),
        TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            frame: ProtocolFrame::Control(ControlMessage::JoinRequest(_)),
            ..
        }
    ));
    host.authorize_peer(device_id, listener.local_routes())
        .expect("listener routes should authorize");
    assert!(matches!(
        host.recv_event(EVENT_TIMEOUT)
            .expect("host should observe authorization"),
        TransportEvent::PeerAuthorized { .. }
    ));
}

fn audio_frame(session_id: &SessionId, sequence: u64) -> ProtocolFrame {
    ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id.clone(),
        stream_id: StreamId::new("fault-stream").expect("test stream ID is valid"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 2,
        first_sample_index: SampleIndex::new(sequence.saturating_mul(2)),
        host_presentation_time_ms: MonotonicMillis::new(5_000 + sequence),
        payload: vec![0, 0, 1, 0, 2, 0, 3, 0],
    })
}

fn recv_audio_sequence(listener: &mut dyn ListenerTransportNode) -> PacketSequence {
    match listener
        .recv_event(EVENT_TIMEOUT)
        .expect("faulted audio event should arrive")
    {
        TransportEvent::FrameReceived {
            channel: TransportChannel::Audio,
            frame: ProtocolFrame::Audio(value),
            ..
        } => value.sequence,
        _ => panic!("expected audio frame"),
    }
}
