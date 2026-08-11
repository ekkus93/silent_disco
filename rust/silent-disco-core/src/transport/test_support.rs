use std::thread;
use std::time::{Duration, Instant};

use crate::domain::{DeviceId, MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, DeviceIdentity, JoinRequest, ProtocolFrame,
};

use super::{
    HostTransportNode, ListenerTransportNode, TransportChannel, TransportErrorKind, TransportEvent,
};

pub(super) const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn join_request(
    session_id: &SessionId,
    device_id: &DeviceId,
    display_name: &str,
) -> ControlMessage {
    ControlMessage::JoinRequest(JoinRequest {
        session_id: session_id.clone(),
        device: DeviceIdentity {
            device_id: device_id.clone(),
            display_name: display_name.to_owned(),
        },
        invite_code: None,
        sync_port: 0,
        audio_port: 0,
    })
}

pub(super) fn audio_frame(session_id: &SessionId, sequence: u64) -> ProtocolFrame {
    ProtocolFrame::Audio(AudioDatagram {
        session_id: session_id.clone(),
        stream_id: StreamId::new("stream-1").expect("test stream ID is valid"),
        sequence: PacketSequence::new(sequence),
        codec: AudioCodec::PcmS16Le,
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 2,
        first_sample_index: SampleIndex::new(sequence.saturating_mul(2)),
        host_presentation_time_ms: MonotonicMillis::new(500 + sequence),
        payload: vec![0, 0, 1, 0, 2, 0, 3, 0],
    })
}

pub(super) fn wait_for_control_from(
    host: &mut dyn HostTransportNode,
    device_id: &DeviceId,
    predicate: impl Fn(&ControlMessage) -> bool,
) -> TransportEvent {
    wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                peer,
                frame: ProtocolFrame::Control(message),
                ..
            } if peer.device_id.as_ref() == Some(device_id) && predicate(message)
        )
    })
}

pub(super) fn wait_for_authorized(host: &mut dyn HostTransportNode, device_id: &DeviceId) {
    drop(wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::PeerAuthorized { peer, .. }
                if peer.device_id.as_ref() == Some(device_id)
        )
    }));
}

pub(super) fn wait_for_control_target(
    listener: &mut dyn ListenerTransportNode,
    device_id: &DeviceId,
) {
    drop(wait_for_listener_event(listener, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(ControlMessage::JoinApproval(value)),
                ..
            } if &value.listener_id == device_id
        )
    }));
}

pub(super) fn wait_for_frame(
    listener: &mut dyn ListenerTransportNode,
    channel: TransportChannel,
    predicate: impl Fn(&ProtocolFrame) -> bool,
) -> TransportEvent {
    wait_for_listener_event(listener, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: actual,
                frame,
                ..
            } if *actual == channel && predicate(frame)
        )
    })
}

pub(super) fn wait_for_frame_from(
    host: &mut dyn HostTransportNode,
    channel: TransportChannel,
    device_id: &DeviceId,
    predicate: impl Fn(&ProtocolFrame) -> bool,
) -> TransportEvent {
    wait_for_host_event(host, |event| {
        matches!(
            event,
            TransportEvent::FrameReceived {
                channel: actual,
                peer,
                frame,
                ..
            } if *actual == channel
                && peer.device_id.as_ref() == Some(device_id)
                && predicate(frame)
        )
    })
}

pub(super) fn wait_for_rejection(host: &mut dyn HostTransportNode, kind: TransportErrorKind) {
    drop(wait_for_host_event(
        host,
        |event| matches!(event, TransportEvent::Rejected { error, .. } if error.kind == kind),
    ));
}

pub(super) fn wait_for_host_event(
    host: &mut dyn HostTransportNode,
    predicate: impl Fn(&TransportEvent) -> bool,
) -> TransportEvent {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for host transport event"
        );
        let event = host
            .recv_event(remaining)
            .expect("host event queue should remain connected");
        if predicate(&event) {
            return event;
        }
    }
}

pub(super) fn wait_for_listener_event(
    listener: &mut dyn ListenerTransportNode,
    predicate: impl Fn(&TransportEvent) -> bool,
) -> TransportEvent {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for listener transport event"
        );
        let event = listener
            .recv_event(remaining)
            .expect("listener event queue should remain connected");
        if predicate(&event) {
            return event;
        }
    }
}

pub(super) fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition did not become true");
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn id_session(value: &str) -> SessionId {
    SessionId::new(value).expect("test session ID is valid")
}

pub(super) fn id_device(value: &str) -> DeviceId {
    DeviceId::new(value).expect("test device ID is valid")
}
