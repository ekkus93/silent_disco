//! Inbound control handling for a listener, shared by the plaintext TCP reader
//! and the TLS pump. Both paths validate host frames the same way and end with
//! the same ordered terminal disconnect.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;

use crate::protocol::ProtocolFrame;

use super::super::{
    ListenerTransportConfig, TransportChannel, TransportClock, TransportError, TransportEvent,
    TransportPeer,
};
use super::listener::validate_listener_inbound;
use super::shared::{
    CounterState, ReadControlOutcome, classify_rejection, is_best_effort_event,
    send_authoritative_event, send_event,
};

pub(super) struct ListenerInbound {
    pub(super) config: ListenerTransportConfig,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) counters: Arc<CounterState>,
    event_sender: SyncSender<TransportEvent>,
    clock: Arc<dyn TransportClock>,
    host_peer: TransportPeer,
}

impl ListenerInbound {
    pub(super) fn new(
        config: ListenerTransportConfig,
        stop: Arc<AtomicBool>,
        counters: Arc<CounterState>,
        event_sender: SyncSender<TransportEvent>,
        clock: Arc<dyn TransportClock>,
    ) -> Self {
        let host_peer = TransportPeer {
            device_id: None,
            control_address: std::net::SocketAddr::new(
                config.endpoint.address,
                config.endpoint.control_port,
            ),
        };
        Self {
            config,
            stop,
            counters,
            event_sender,
            clock,
            host_peer,
        }
    }

    /// Handles one inbound outcome. Returns `false` when reading must stop;
    /// `terminal_error` records why the connection ends.
    pub(super) fn handle(
        &self,
        outcome: ReadControlOutcome,
        terminal_error: &mut Option<TransportError>,
    ) -> bool {
        match outcome {
            ReadControlOutcome::Frame(frame, bytes) => {
                self.counters.frame_received(bytes);
                match validate_listener_inbound(&self.config.device_id, &frame) {
                    Ok(()) => self.deliver(frame),
                    Err(error) => {
                        // A diagnostic Rejected event is lossy under
                        // queue pressure. Preserve the reason in the
                        // authoritative terminal disconnect as well.
                        *terminal_error = Some(error.clone());
                        classify_rejection(&self.counters, &error);
                        self.report_rejection(error);
                        false
                    }
                }
            }
            ReadControlOutcome::Closed => false,
            ReadControlOutcome::Rejected(error) => {
                *terminal_error = Some(error.clone());
                classify_rejection(&self.counters, &error);
                self.report_rejection(error);
                false
            }
        }
    }

    fn deliver(&self, frame: ProtocolFrame) -> bool {
        // Shared classification protects playback controls from telemetry loss.
        let event = TransportEvent::FrameReceived {
            channel: TransportChannel::Control,
            peer: self.host_peer.clone(),
            frame,
            received_at: self.clock.now(),
        };
        if is_best_effort_event(&event) {
            send_event(&self.event_sender, &self.counters, event);
            true
        } else {
            send_authoritative_event(&self.event_sender, &self.stop, &self.counters, event)
        }
    }

    fn report_rejection(&self, error: TransportError) {
        send_event(
            &self.event_sender,
            &self.counters,
            TransportEvent::Rejected {
                channel: TransportChannel::Control,
                source: self.host_peer.control_address,
                error,
                received_at: self.clock.now(),
            },
        );
    }

    /// Delivers the terminal disconnect even if a full event queue
    /// temporarily prevents the UI from polling. The owner can still interrupt
    /// this wait via `shutdown()`, which sets stop.
    pub(super) fn finish(&self, terminal_error: Option<TransportError>) {
        send_authoritative_event(
            &self.event_sender,
            &self.stop,
            &self.counters,
            TransportEvent::PeerDisconnected {
                peer: self.host_peer.clone(),
                error: terminal_error,
                received_at: self.clock.now(),
            },
        );
        self.stop.store(true, Ordering::Release);
    }
}
