//! Shared classification for host and listener TCP/UDP event pressure.
use crate::protocol::{ControlMessage, ProtocolFrame};
use crate::transport::{TransportChannel, TransportEvent};

/// Only diagnostics, datagrams and optional TCP telemetry may be dropped.
/// Admission, playback and terminal events require reliable delivery.
pub(crate) fn is_best_effort_event(event: &TransportEvent) -> bool {
    matches!(
        event,
        TransportEvent::Rejected { .. }
            | TransportEvent::FrameReceived {
                channel: TransportChannel::Audio | TransportChannel::Synchronization,
                ..
            }
            | TransportEvent::FrameReceived {
                channel: TransportChannel::Control,
                frame: ProtocolFrame::Control(
                    ControlMessage::Heartbeat(_) | ControlMessage::SynchronizationReport(_),
                ),
                ..
            }
    )
}

#[cfg(test)]
mod tests {
    use super::is_best_effort_event;
    use crate::domain::MonotonicMillis;
    use crate::protocol::{ControlMessage, Heartbeat, ProtocolFrame};
    use crate::transport::test_support::{id_device, id_session, join_request, stream_stop};
    use crate::transport::{TransportChannel, TransportEvent, TransportPeer};

    #[test]
    fn playback_join_and_terminal_events_are_never_best_effort() {
        let session = id_session("event-delivery-classification");
        let device = id_device("event-delivery-device");
        let peer = TransportPeer {
            device_id: Some(device.clone()),
            control_address: "127.0.0.1:43123".parse().expect("loopback endpoint"),
        };
        let received_at = MonotonicMillis::new(123);
        let event = |frame| TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            peer: peer.clone(),
            frame: ProtocolFrame::Control(frame),
            received_at,
        };
        assert!(!is_best_effort_event(&TransportEvent::PeerAccepted {
            peer: peer.clone(),
            received_at,
        }));
        assert!(!is_best_effort_event(&TransportEvent::PeerDisconnected {
            peer: peer.clone(),
            error: None,
            received_at,
        }));
        assert!(!is_best_effort_event(&event(join_request(
            &session, &device, "New join",
        ))));
        assert!(!is_best_effort_event(&event(stream_stop(&session, 900))));
        assert!(is_best_effort_event(&event(ControlMessage::Heartbeat(
            Heartbeat {
                session_id: session,
                listener_id: device,
                sent_at_elapsed_ms: received_at,
            },
        ))));
    }

    #[test]
    fn best_effort_sender_rejects_misrouted_terminal_before_queueing() {
        use super::super::{CounterState, send_event};
        use std::sync::mpsc::sync_channel;

        let (sender, receiver) = sync_channel(1);
        let counters = CounterState::default();
        let terminal = TransportEvent::PeerDisconnected {
            peer: TransportPeer {
                device_id: None,
                control_address: "127.0.0.1:42345".parse().expect("loopback"),
            },
            error: None,
            received_at: MonotonicMillis::new(1),
        };
        let result = std::panic::catch_unwind(|| send_event(&sender, &counters, terminal));
        assert!(
            result.is_err(),
            "critical terminal must never enter lossy delivery"
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(counters.snapshot().best_effort_event_drops, 0);
    }
}
