//! Owned desktop host transport worker.

use super::host_transport_events::{DesktopHostTransportEventSink, HostTransportEventProcessor};
use super::network_error::DesktopNetworkError;
use silent_disco_core::domain::{DeliverySeverity, MonotonicMillis};
use silent_disco_core::error::CoreError;
use silent_disco_core::protocol::{
    ControlMessage, Disconnect, JoinApproval, JoinRejection, ProtocolFrame,
};
use silent_disco_core::runtime::{
    DeliveryReport, SessionAdvertisement, TransportEffect, TransportEffectRequest,
    TransportEvent as CoreTransportEvent,
};
use silent_disco_core::transport::{
    HostTransportNode, TransportClock, TransportDelivery, TransportErrorKind,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Poll interval used instead of [`EVENT_POLL_INTERVAL`] whenever the
/// broadcast queue still has frames in it after a drain pass. `recv_event`
/// blocks for up to the poll interval when no control-plane traffic
/// arrives, during which nothing drains the broadcast queue at all -- at
/// the real packetizer's 5ms/200-per-second cadence (`DEFAULT_PACKET_DURATION_MS`,
/// dropped from 20ms the day after this worker's polling was sized; see
/// `git log` on this file vs. that constant), a real slow send (genuine
/// Wi-Fi congestion, not the fast/deterministic loopback this worker's own
/// tests run against) stalls draining for that same span, during which the
/// queue can fill. A short poll here shrinks the worst-case recovery gap
/// from 20ms to ~1ms once backlog is observed, without busy-polling during
/// the (far more common) idle-hosting periods when nothing is playing.
const BACKLOG_POLL_INTERVAL: Duration = Duration::from_millis(1);
const TRANSPORT_EFFECT_QUEUE_CAPACITY: usize = 32;
const MAX_EFFECTS_PER_TICK: usize = 8;
/// Bounded output queue between a playback pump thread and this worker.
///
/// Must comfortably exceed one full send-ahead horizon's worth of packets,
/// not just the packetizer's own 32-frame default -- the pump
/// (`playback_streamer.rs`'s `SEND_AHEAD_HORIZON_MS`, currently 1000ms) is
/// deliberately allowed to burst out an entire horizon of already-
/// packetized audio with no pacing at all at stream start, and at the
/// packetizer's `DEFAULT_PACKET_DURATION_MS` (5ms) that is up to 200
/// packets arriving here far faster than this worker can be expected to
/// drain them one poll tick at a time. A queue sized only for a "momentary
/// stall" (the previous 64-frame/320ms sizing) guarantees that every
/// stream's opening burst overflows it -- confirmed on a real device
/// (LG G6, 2026-08-09): `queue_overflows` climbed from 0 to 59 in the first
/// 15 seconds of every run, entirely before any pause ever happened, and
/// stayed exactly flat afterward once the burst was over. 256 gives ~28%
/// headroom over the 200-packet worst case at today's defaults; this is a
/// sizing choice tied to those defaults, not an enforced invariant -- it
/// must be revisited if `SEND_AHEAD_HORIZON_MS` grows or the default packet
/// duration shrinks.
const BROADCAST_FRAME_QUEUE_CAPACITY: usize = 256;
const MAX_BROADCAST_FRAMES_PER_TICK: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostTransportStatus {
    pub(crate) running: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) broadcast: BroadcastDiagnostics,
}

/// Delivery and queue-pressure accounting for the real-time broadcast path.
///
/// `broadcast_audio`/`broadcast_sync` already return a [`TransportDelivery`]
/// describing how many recipients a frame was intended for and how many it
/// actually reached; the worker used to discard that entirely and keep only an
/// aggregate last-error string. Nothing therefore reported partial delivery,
/// and a broadcast to *zero* recipients was indistinguishable from a
/// successful one -- which CLAUDE.md names explicitly as not-success.
///
/// These are counts across a delivery attempt, not per-peer identities: the
/// transport reports intended/successful/failed totals, and attributing a
/// failure to a specific listener would need a change in the shared transport
/// layer rather than here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct BroadcastDiagnostics {
    /// Frames the worker attempted to broadcast.
    pub(crate) frames_attempted: u64,
    /// Frames the transport rejected outright.
    pub(crate) frames_failed: u64,
    /// Frames that reached every intended recipient.
    pub(crate) frames_fully_delivered: u64,
    /// Frames that reached some but not all intended recipients.
    pub(crate) frames_partially_delivered: u64,
    /// Frames broadcast while no listener was connected. Not a failure of the
    /// transport, but not delivery either, and it must stay visible.
    pub(crate) frames_without_recipients: u64,
    /// Recipient-sends attempted, summed across frames.
    pub(crate) recipients_intended: u64,
    /// Recipient-sends that succeeded.
    pub(crate) recipients_delivered: u64,
    /// Frames currently waiting in the broadcast queue.
    pub(crate) queue_depth: u64,
    /// Deepest the broadcast queue has been.
    pub(crate) queue_peak_depth: u64,
    /// Frames dropped because the broadcast queue was full.
    pub(crate) queue_overflows: u64,
}

pub(crate) struct ActiveHostSessionSnapshot {
    pub(crate) advertisement: SessionAdvertisement,
    pub(crate) endpoint: silent_disco_core::runtime::NetworkEndpoint,
    pub(crate) worker_running: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) observed_at_ms: u64,
    pub(crate) broadcast: BroadcastDiagnostics,
}

#[derive(Debug)]
struct SharedStatus {
    running: AtomicBool,
    last_error: Mutex<Option<String>>,
    broadcast: BroadcastCounters,
}

/// Atomic backing for [`BroadcastDiagnostics`]. Updated on the real-time
/// broadcast path, so every field is a plain relaxed counter rather than
/// anything that could block the worker or the playback pump.
#[derive(Debug, Default)]
struct BroadcastCounters {
    frames_attempted: AtomicU64,
    frames_failed: AtomicU64,
    frames_fully_delivered: AtomicU64,
    frames_partially_delivered: AtomicU64,
    frames_without_recipients: AtomicU64,
    recipients_intended: AtomicU64,
    recipients_delivered: AtomicU64,
    queue_depth: AtomicU64,
    queue_peak_depth: AtomicU64,
    queue_overflows: AtomicU64,
}

impl BroadcastCounters {
    fn snapshot(&self) -> BroadcastDiagnostics {
        BroadcastDiagnostics {
            frames_attempted: self.frames_attempted.load(Ordering::Relaxed),
            frames_failed: self.frames_failed.load(Ordering::Relaxed),
            frames_fully_delivered: self.frames_fully_delivered.load(Ordering::Relaxed),
            frames_partially_delivered: self.frames_partially_delivered.load(Ordering::Relaxed),
            frames_without_recipients: self.frames_without_recipients.load(Ordering::Relaxed),
            recipients_intended: self.recipients_intended.load(Ordering::Relaxed),
            recipients_delivered: self.recipients_delivered.load(Ordering::Relaxed),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            queue_peak_depth: self.queue_peak_depth.load(Ordering::Relaxed),
            queue_overflows: self.queue_overflows.load(Ordering::Relaxed),
        }
    }

    /// Records one frame entering the queue, keeping the peak depth current.
    fn record_enqueued(&self) {
        let depth = self.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        self.queue_peak_depth.fetch_max(depth, Ordering::Relaxed);
    }

    fn record_dequeued(&self) {
        // Saturating rather than wrapping: a depth that has already reached
        // zero must not roll over into a nonsense diagnostic.
        let _ = self
            .queue_depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                Some(depth.saturating_sub(1))
            });
    }

    fn record_delivery(&self, delivery: &TransportDelivery) {
        let intended = u64::from(delivery.report.intended_peers);
        let successful = u64::from(delivery.report.successful_peers);
        self.frames_attempted.fetch_add(1, Ordering::Relaxed);
        self.recipients_intended
            .fetch_add(intended, Ordering::Relaxed);
        self.recipients_delivered
            .fetch_add(successful, Ordering::Relaxed);
        if intended == 0 {
            self.frames_without_recipients
                .fetch_add(1, Ordering::Relaxed);
        } else if successful == intended {
            self.frames_fully_delivered.fetch_add(1, Ordering::Relaxed);
        } else {
            self.frames_partially_delivered
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_failure(&self) {
        self.frames_attempted.fetch_add(1, Ordering::Relaxed);
        self.frames_failed.fetch_add(1, Ordering::Relaxed);
    }
}

include!("host_transport/runtime.rs");
include!("host_transport/worker.rs");
include!("host_transport/effects.rs");
