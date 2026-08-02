use std::sync::Arc;
use std::time::Duration;

use crate::domain::{DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
    SyncRequest,
};

use super::{
    HostTransportConfig, HostTransportNode, ListenerTransportConfig, ListenerTransportNode,
    ManualTransportClock, TransportChannel, TransportErrorKind, TransportEvent, TransportFactory,
    VirtualTransportFactory, VirtualTransportNetwork, VirtualUdpFaultConfig,
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
