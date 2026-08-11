use std::sync::atomic::Ordering;

use crate::protocol::{ProtocolFrame, encode_frame};

use super::super::super::{
    TransportChannel, TransportDelivery, TransportError, TransportErrorKind,
};
use super::SocketHostTransport;

impl SocketHostTransport {
    pub(super) fn broadcast_datagram(
        &self,
        channel: TransportChannel,
        frame: &ProtocolFrame,
    ) -> Result<TransportDelivery, TransportError> {
        match (channel, frame) {
            (
                TransportChannel::Synchronization,
                ProtocolFrame::SyncRequest(_) | ProtocolFrame::SyncResponse(_),
            )
            | (TransportChannel::Audio, ProtocolFrame::Audio(_)) => {}
            _ => {
                return Err(TransportError::new(
                    TransportErrorKind::Protocol,
                    channel,
                    "protocol frame does not belong on the requested datagram channel",
                ));
            }
        }
        if frame.session_id() != &self.session_id {
            return Err(TransportError::new(
                TransportErrorKind::Unauthorized,
                channel,
                "outbound datagram belongs to a different session",
            ));
        }
        let bytes =
            encode_frame(frame).map_err(|error| TransportError::protocol(channel, &error))?;
        let routes = self.authorized_routes()?;
        let intended = u32::try_from(routes.len()).map_err(|_| {
            TransportError::new(
                TransportErrorKind::Delivery,
                channel,
                "peer count exceeds delivery accounting range",
            )
        })?;
        let socket = match channel {
            TransportChannel::Synchronization => &self.sync_socket,
            TransportChannel::Audio => &self.audio_socket,
            TransportChannel::Control | TransportChannel::Runtime => {
                return Err(TransportError::new(
                    TransportErrorKind::InvalidConfiguration,
                    channel,
                    "datagram delivery requires a datagram channel",
                ));
            }
        };
        let mut successful = 0_u32;
        let mut failed = 0_u32;
        let mut sent_bytes = 0_u64;
        for (_, route) in routes {
            let destination = match channel {
                TransportChannel::Synchronization => route.routes.synchronization,
                TransportChannel::Audio => route.routes.audio,
                TransportChannel::Control | TransportChannel::Runtime => {
                    return Err(TransportError::new(
                        TransportErrorKind::InvalidConfiguration,
                        channel,
                        "datagram route requires a datagram channel",
                    ));
                }
            };
            match socket.send_to(&bytes, destination) {
                Ok(written) if written == bytes.len() => {
                    successful = successful.saturating_add(1);
                    sent_bytes =
                        sent_bytes.saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
                    self.counters.datagram_sent(channel, written);
                    route.peer.consecutive_failures.store(0, Ordering::Release);
                }
                Ok(_) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                    self.record_peer_result(
                        &route.peer,
                        &Err(TransportError::new(
                            TransportErrorKind::Delivery,
                            channel,
                            "datagram send reported a partial write",
                        )),
                    );
                }
                Err(error) => {
                    failed = failed.saturating_add(1);
                    self.counters.delivery_failure();
                    // A `WouldBlock`/timeout here means `DATAGRAM_SEND_TIMEOUT`
                    // fired -- the send didn't complete in time, not that
                    // this peer is gone. Audio/sync delivery is inherently
                    // best-effort and lossy by design (a real device drops
                    // ~1% of UDP packets in ordinary conditions), so a
                    // routine timeout under momentary congestion must not
                    // count toward `max_consecutive_failures` the same way
                    // a genuine I/O error (unreachable host, connection
                    // reset) does -- confirmed on real hardware
                    // (2026-08-09): failing fast on every timeout tripped
                    // the 3-consecutive-failure disconnect threshold within
                    // ~15ms of a brief real congestion blip, disconnecting
                    // a listener that was still there. It stays a counted,
                    // visible delivery failure either way; only the
                    // auto-disconnect vote is skipped.
                    if is_datagram_send_timeout(&error) {
                        continue;
                    }
                    self.record_peer_result(
                        &route.peer,
                        &Err(TransportError::io(
                            TransportErrorKind::Delivery,
                            channel,
                            "failed to send datagram",
                            &error,
                        )),
                    );
                }
            }
        }
        TransportDelivery::new(intended, successful, failed, sent_bytes)
    }
}

/// Classifies whether a `send_to` error was `DATAGRAM_SEND_TIMEOUT` firing
/// (routine under momentary congestion, must not count toward
/// `max_consecutive_failures`) rather than a genuine send failure (must
/// still count). Extracted as a pure function so this specific decision is
/// unit-testable without a real, slow/congested socket -- loopback UDP
/// essentially never blocks or times out, so the full path this guards
/// against was only reproducible on real hardware (confirmed 2026-08-09:
/// a real, older Android phone under real Wi-Fi load).
fn is_datagram_send_timeout(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
}

#[cfg(test)]
mod send_timeout_classification_tests {
    use super::is_datagram_send_timeout;
    use std::io;

    #[test]
    fn would_block_is_classified_as_a_send_timeout() {
        assert!(is_datagram_send_timeout(&io::Error::from(
            io::ErrorKind::WouldBlock
        )));
    }

    #[test]
    fn other_error_kinds_are_not_classified_as_a_send_timeout() {
        assert!(!is_datagram_send_timeout(&io::Error::from(
            io::ErrorKind::ConnectionRefused
        )));
        assert!(!is_datagram_send_timeout(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_datagram_send_timeout(&io::Error::other(
            "unexpected send failure"
        )));
    }
}
