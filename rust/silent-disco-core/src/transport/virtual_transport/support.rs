use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

use crate::domain::{DeviceId, MonotonicMillis};
use crate::protocol::{ControlMessage, ProtocolFrame, decode_frame, encode_frame};
use crate::runtime::NetworkEndpoint;
use crate::transport::{
    TransportChannel, TransportCounters, TransportError, TransportErrorKind, TransportEvent,
    TransportPeer,
};

use super::network::VirtualNetworkState;

/// In-process wire item used by the virtual transport. Protocol frames cross
/// the virtual link as their canonical encoded bytes plus transport metadata;
/// lifecycle notifications remain typed because they are transport events, not
/// protocol-wire frames. `recv_virtual_event` is the sole frame decode point.
#[derive(Clone)]
pub(super) enum VirtualWireEvent {
    Frame {
        channel: TransportChannel,
        peer: TransportPeer,
        bytes: Vec<u8>,
        received_at: MonotonicMillis,
    },
    Lifecycle(TransportEvent),
}

pub(super) fn allocate_endpoint(
    state: &mut VirtualNetworkState,
    requested_address: IpAddr,
) -> Result<NetworkEndpoint, TransportError> {
    let address = if requested_address.is_unspecified() {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        requested_address
    };
    let control_port = allocate_port(state)?;
    let sync_port = allocate_port(state)?;
    let audio_port = allocate_port(state)?;
    NetworkEndpoint::new(address, control_port, sync_port, audio_port).map_err(|error| {
        TransportError::new(
            TransportErrorKind::Bind,
            TransportChannel::Runtime,
            error.to_string(),
        )
    })
}

pub(super) fn allocate_port(state: &mut VirtualNetworkState) -> Result<u16, TransportError> {
    let current = state.next_port.max(20_000);
    let next = current.checked_add(1).ok_or_else(|| {
        TransportError::new(
            TransportErrorKind::Bind,
            TransportChannel::Runtime,
            "virtual transport port space is exhausted",
        )
    })?;
    state.next_port = next;
    Ok(current)
}

pub(super) fn encode_wire_frame(
    frame: &ProtocolFrame,
    channel: TransportChannel,
) -> Result<Vec<u8>, TransportError> {
    encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))
}

pub(super) fn try_frame(
    sender: &SyncSender<VirtualWireEvent>,
    channel: TransportChannel,
    peer: TransportPeer,
    bytes: Vec<u8>,
    received_at: MonotonicMillis,
) -> Result<(), TransportError> {
    try_wire_event(
        sender,
        VirtualWireEvent::Frame {
            channel,
            peer,
            bytes,
            received_at,
        },
    )
}

pub(super) fn try_event(
    sender: &SyncSender<VirtualWireEvent>,
    event: TransportEvent,
) -> Result<(), TransportError> {
    if matches!(event, TransportEvent::FrameReceived { .. }) {
        return Err(TransportError::new(
            TransportErrorKind::Protocol,
            TransportChannel::Runtime,
            "virtual protocol frames must cross the byte-level wire boundary",
        ));
    }
    try_wire_event(sender, VirtualWireEvent::Lifecycle(event))
}

fn try_wire_event(
    sender: &SyncSender<VirtualWireEvent>,
    event: VirtualWireEvent,
) -> Result<(), TransportError> {
    match sender.try_send(event) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => Err(TransportError::new(
            TransportErrorKind::QueueFull,
            TransportChannel::Runtime,
            "virtual transport event queue is full",
        )),
        Err(TrySendError::Disconnected(_)) => Err(shutting_down()),
    }
}

pub(super) fn recv_virtual_event(
    receiver: &Receiver<VirtualWireEvent>,
    timeout: Duration,
) -> Result<TransportEvent, TransportError> {
    match receiver.recv_timeout(timeout) {
        Ok(VirtualWireEvent::Lifecycle(event)) => Ok(event),
        Ok(VirtualWireEvent::Frame {
            channel,
            peer,
            bytes,
            received_at,
        }) => {
            let frame =
                decode_frame(&bytes).map_err(|error| TransportError::protocol(channel, &error))?;
            Ok(TransportEvent::FrameReceived {
                channel,
                peer,
                frame,
                received_at,
            })
        }
        Err(RecvTimeoutError::Timeout) => Err(TransportError::new(
            TransportErrorKind::Timeout,
            TransportChannel::Runtime,
            "timed out waiting for a virtual transport event",
        )),
        Err(RecvTimeoutError::Disconnected) => Err(shutting_down()),
    }
}

pub(super) fn update_counters(
    counters: &Mutex<TransportCounters>,
    update: impl FnOnce(&mut TransportCounters),
) -> Result<(), TransportError> {
    let mut counters = counters.lock().map_err(|_| {
        TransportError::new(
            TransportErrorKind::WorkerPanicked,
            TransportChannel::Runtime,
            "virtual transport counters are poisoned",
        )
    })?;
    update(&mut counters);
    Ok(())
}

pub(super) fn validate_virtual_listener_identity(
    device_id: &DeviceId,
    message: &ControlMessage,
) -> Result<(), TransportError> {
    let matches = match message {
        ControlMessage::JoinRequest(value) => &value.device.device_id == device_id,
        ControlMessage::Heartbeat(value) => &value.listener_id == device_id,
        ControlMessage::Disconnect(value) => &value.listener_id == device_id,
        ControlMessage::ResyncNotice(value) => &value.listener_id == device_id,
        ControlMessage::SynchronizationReport(value) => &value.listener_id == device_id,
        ControlMessage::Hello(_)
        | ControlMessage::JoinApproval(_)
        | ControlMessage::JoinRejection(_)
        | ControlMessage::StreamStart(_)
        | ControlMessage::Pause(_)
        | ControlMessage::Stop(_) => false,
    };
    if matches {
        Ok(())
    } else {
        Err(unauthorized(TransportChannel::Control))
    }
}

pub(super) fn to_u32(value: usize, channel: TransportChannel) -> Result<u32, TransportError> {
    u32::try_from(value).map_err(|_| {
        TransportError::new(
            TransportErrorKind::Delivery,
            channel,
            "peer count exceeds delivery accounting range",
        )
    })
}

pub(super) fn unauthorized(channel: TransportChannel) -> TransportError {
    TransportError::new(
        TransportErrorKind::Unauthorized,
        channel,
        "transport frame is not authorized for this session or peer",
    )
}

pub(super) fn shutting_down() -> TransportError {
    TransportError::new(
        TransportErrorKind::ShuttingDown,
        TransportChannel::Runtime,
        "virtual transport node is not available",
    )
}

pub(super) fn network_poisoned<T>(_: std::sync::PoisonError<T>) -> TransportError {
    TransportError::new(
        TransportErrorKind::WorkerPanicked,
        TransportChannel::Runtime,
        "virtual transport network is poisoned",
    )
}
