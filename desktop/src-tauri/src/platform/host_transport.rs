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

pub(crate) struct DesktopHostTransportRuntime {
    endpoint: silent_disco_core::runtime::NetworkEndpoint,
    stop: Arc<AtomicBool>,
    effect_sender: SyncSender<TransportEffect>,
    broadcast_sender: SyncSender<ProtocolFrame>,
    status: Arc<SharedStatus>,
    clock: Arc<dyn TransportClock>,
    worker: Option<JoinHandle<Result<(), DesktopNetworkError>>>,
}

impl DesktopHostTransportRuntime {
    pub(super) fn start(
        node: Box<dyn HostTransportNode>,
        advertisement: SessionAdvertisement,
        sink: Arc<dyn DesktopHostTransportEventSink>,
        clock: Arc<dyn TransportClock>,
    ) -> Result<Self, DesktopNetworkError> {
        let endpoint = node.endpoint();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(SharedStatus {
            running: AtomicBool::new(true),
            last_error: Mutex::new(None),
            broadcast: BroadcastCounters::default(),
        });
        let (effect_sender, effect_receiver) = sync_channel(TRANSPORT_EFFECT_QUEUE_CAPACITY);
        let (broadcast_sender, broadcast_receiver) = sync_channel(BROADCAST_FRAME_QUEUE_CAPACITY);
        let worker_stop = Arc::clone(&stop);
        let worker_status = Arc::clone(&status);
        let worker_clock = Arc::clone(&clock);
        let worker = thread::Builder::new()
            .name("silent-disco-desktop-host-transport".to_owned())
            .spawn(move || {
                run_transport_worker(
                    node,
                    &advertisement,
                    &sink,
                    &effect_receiver,
                    &broadcast_receiver,
                    &worker_stop,
                    &worker_status,
                    &worker_clock,
                )
            })
            .map_err(|error| {
                DesktopNetworkError::unavailable(format!(
                    "failed to start desktop host transport worker: {error}"
                ))
            })?;
        Ok(Self {
            endpoint,
            stop,
            effect_sender,
            broadcast_sender,
            status,
            clock,
            worker: Some(worker),
        })
    }

    /// Enqueues one control/sync/audio frame for the worker thread to
    /// broadcast on its next tick. Non-blocking: a full queue or a shut-down
    /// worker is reported as an error rather than stalling the caller (a
    /// playback pump thread), since audio delivery is inherently best-effort.
    pub(super) fn broadcast_frame(&self, frame: ProtocolFrame) -> Result<(), DesktopNetworkError> {
        if self.stop.load(Ordering::Acquire) {
            return Err(DesktopNetworkError::unavailable(
                "desktop host transport is shutting down",
            ));
        }
        match self.broadcast_sender.try_send(frame) {
            Ok(()) => {
                self.status.broadcast.record_enqueued();
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                self.status
                    .broadcast
                    .queue_overflows
                    .fetch_add(1, Ordering::Relaxed);
                Err(DesktopNetworkError::resource_limit(
                    "desktop host transport broadcast queue is full",
                ))
            }
            Err(TrySendError::Disconnected(_)) => Err(DesktopNetworkError::unavailable(
                "desktop host transport worker is unavailable",
            )),
        }
    }

    #[must_use]
    pub(crate) const fn endpoint(&self) -> silent_disco_core::runtime::NetworkEndpoint {
        self.endpoint
    }

    #[must_use]
    pub(crate) fn observed_at(&self) -> MonotonicMillis {
        self.clock.now()
    }

    pub(crate) fn dispatch(&self, effect: TransportEffect) -> Result<(), CoreError> {
        let operation_id = effect.operation_id.clone();
        if self.stop.load(Ordering::Acquire) {
            return Err(DesktopNetworkError::unavailable(
                "desktop host transport is shutting down",
            )
            .core_error(Some(operation_id)));
        }
        match self.effect_sender.try_send(effect) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(DesktopNetworkError::resource_limit(
                "desktop host transport effect queue is full",
            )
            .core_error(Some(operation_id))),
            Err(TrySendError::Disconnected(_)) => Err(DesktopNetworkError::unavailable(
                "desktop host transport effect worker is unavailable",
            )
            .core_error(Some(operation_id))),
        }
    }

    pub(super) fn status(&self) -> Result<HostTransportStatus, DesktopNetworkError> {
        let last_error = self
            .status
            .last_error
            .lock()
            .map_err(|_| {
                DesktopNetworkError::invalid_state(
                    "desktop host transport status mutex was poisoned",
                )
            })?
            .clone();
        Ok(HostTransportStatus {
            running: self.status.running.load(Ordering::Acquire),
            last_error,
            broadcast: self.status.broadcast.snapshot(),
        })
    }

    #[cfg(test)]
    pub(super) fn stop_worker_for_test(&mut self) -> Result<(), DesktopNetworkError> {
        self.stop.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        match worker.join() {
            Ok(result) => result,
            Err(_) => Err(DesktopNetworkError::unavailable(
                "desktop host transport worker panicked during test shutdown",
            )),
        }
    }

    pub(super) fn shutdown(mut self) -> Result<(), DesktopNetworkError> {
        self.stop.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        match worker.join() {
            Ok(result) => result,
            Err(_) => Err(DesktopNetworkError::unavailable(
                "desktop host transport worker panicked during shutdown",
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_transport_worker(
    mut node: Box<dyn HostTransportNode>,
    advertisement: &SessionAdvertisement,
    sink: &Arc<dyn DesktopHostTransportEventSink>,
    effect_receiver: &Receiver<TransportEffect>,
    broadcast_receiver: &Receiver<ProtocolFrame>,
    stop: &AtomicBool,
    status: &SharedStatus,
    clock: &Arc<dyn TransportClock>,
) -> Result<(), DesktopNetworkError> {
    let mut processor = HostTransportEventProcessor::new(Arc::clone(clock));
    let mut primary_error = None;
    while !stop.load(Ordering::Acquire) {
        if let Err(error) =
            process_effects(&[›ÙK	‰Í¥¹¬°•™™•Ñ}É••¥Ù•È°ÍÑ…ÑÕÌ°€™µÕĞÁÉ½•ÍÍ½È¤(€€€€€€€ì(€€€€€€€€€€€ÁÉ¥µ…Éå}•ÉÉ½È€ôM½µ”¡•ÉÉ½È¤ì(€€€€€€€€€€€‰É•…¬ì(€€€€€€€ô(€€€€€€€¥˜±•ĞÉÈ¡•ÉÉ½È¤€ôÁÉ½•ÍÍ}‰É½…‘…ÍÑ}™É…µ•Ì ˜››ÙKœ›ØYØ\İÜ™XÙZ]™\‹İ]\ÊHÂˆš[X\WÙ\œ›ÜˆHÛÛYJ\œ›ÜŠNÂˆœ™XZÎÂˆBˆ]ÛÚ[\˜[HYˆİ]\Ë˜œ›ØYØ\İœ]Y]YWÙ\›ØY
Ü™\š[™Î”™[^Y
HˆÂˆPÒÓÑ×ÔÓÒS•T•SˆH[ÙHÂˆU‘S•ÔÓÒS•T•SˆNÂˆX]Ú›ÙKœ™Xİ—Ù]™[
ÛÚ[\˜[
HÂˆÚÊ]™[
HOˆX]Ú›ØÙ\ÜÛÜ‹œ›ØÙ\ÜÊ]™[	‰¹½‘”°…‘Ù•ÉÑ¥Í•µ•¹Ğ°€˜œÚ[šÊHÂˆÚÊÛÛYJY\ÜØYÙJJHOˆÙ]Û\İÙ\œ›ÜŠİ]\ËY\ÜØYÙJOËˆÚÊ›Û™JHOˆßBˆ\œŠ\œ›ÜŠHOˆÂˆÙ]Û\İÙ\œ›ÜŠİ]\Ë\œ›Ü‹×Üİš[™Ê
JOÎÂˆš[X\WÙ\œ›ÜˆHÛÛYJ\œ›ÜŠNÂˆœ™XZÎÂˆBˆKˆ\œŠ\œ›ÜŠHYˆ\œ›Ü‹šÚ[™OH˜[œÜÜ\œ›Ü’Ú[™•[Y[İ]OˆßBˆ\œŠ\œ›ÜŠHOˆÂˆ]\œ›ÜˆH\ÚİÜ™]ÛÜšÑ\œ›Ü˜[œÜÜ
	™\œ›ÜŠNÂˆÙ]Û\İÙ\œ›ÜŠİ]\Ë\œ›Ü‹×Üİš[™Ê
JOÎÂˆš[X\WÙ\œ›ÜˆHÛÛYJ\œ›ÜŠNÂˆœ™XZÎÂˆBˆBˆB‚ˆ]˜Z[—Ù\œ›ÜˆH˜Z[Ü]Y]YYÙY™™XİÊY™™XİÜ™XÙZ]™\‹	ŠŠœÚ[šËİ]\ÊK™\œŠ
NÂˆ]Ú]İÛ—Ù\œ›ÜˆH›ÙBˆœÚ]İÛŠ
Bˆ›X\Ù\œŠ\œ›ÜŸ\ÚİÜ™]ÛÜšÑ\œ›Ü˜[œÜÜ
	™\œ›ÜŠJBˆ™\œŠ
NÂˆİ]\Ëœ[›š[™ËœİÜ™J˜[ÙKÜ™\š[™Î”™[X\ÙJNÂˆš[X\WÙ\œ›Ü‚ˆ›ÜŠ˜Z[—Ù\œ›ÜŠBˆ›ÜŠÚ]İÛ—Ù\œ›ÜŠBˆ›X\ÛÜŠÚÊ

JK\œŠBŸB‚‹ËËÈ˜Z[œÈ\ÈØPVĞ”“ĞQĞTÕÑ”SQT×ÔT—ÕPÒØHœ˜[Y\È]Y]YYHB‹ËËÈ^X˜XÚÈ[\™XY
İ™X[K\İ\ÛÛ›Û]Y[È]YÜ˜[\ÊH[™‹ËËÈœ›ØYØ\İÈXXÚÛˆHÚ[›™[]È›İØÛÛœ˜[YX˜\šX[™[Û™ÜÈË‚‹ËËÈH\‹Yœ˜[YH[]™\H˜Z[\™H\È™XÛÜ™Y\ÈH\İ\œ›Üˆ]Ù\È›İ‹ËËÈİÜHÛÜšÙ\ˆKHÛ™H›ÜY]Y[ÈXÚÙ]\È›İ˜][ÈHİ™X[K‹ËËÈX]Ú[™ÈH[™›ÚYÜİ	ÜÈ\‹\XÚÙ]œ›ØYØ\İX]Y[È[™[™Ë‚™›ˆ›ØÙ\Ü×Øœ›ØYØ\İÙœ˜[Y\Êˆ›ÙNˆ	™[ˆÜİ˜[œÜÜ›ÙKˆ™XÙZ]™\ˆ	”™XÙZ]™\›İØÛÛœ˜[YO‹ˆİ]\Îˆ	”Ú\™Yİ]\ËŠHOˆ™\İ[

K\ÚİÜ™]ÛÜšÑ\œ›ÜˆÂˆ›ÜˆÈ[ˆ‹“PVĞ”“ĞQĞTÕÑ”SQT×ÔT—ÕPÒÈÂˆ]œ˜[YHHX]Ú™XÙZ]™\‹WÜ™XİŠ
HÂˆÚÊœ˜[YJHOˆœ˜[YKˆ\œŠT™Xİ‘\œ›Ü‘[\JHOˆ™]\›ˆÚÊ

JKˆ\œŠT™Xİ‘\œ›Ü‘\ØÛÛ›™XİY
HOˆÂˆ™]\›ˆ\œŠ\ÚİÜ™]ÛÜšÑ\œ›Ü[˜]˜Z[X›Jˆ™\ÚİÜÜİ˜[œÜÜœ›ØYØ\İ]Y]YH\ØÛÛ›™XİY‹ˆ
JNÂˆBˆNÂˆİ]\Ë˜œ›ØYØ\İœ™XÛÜ™Ù\]Y]YY

NÂˆ][]™\HHX]Ú	™œ˜[YHÂˆ›İØÛÛœ˜[YNÛÛ›Û
Y\ÜØYÙJHOˆ›ÙK˜œ›ØYØ\İØÛÛ›Û
Y\ÜØYÙJKˆ›İØÛÛœ˜[YN]Y[ÊÊHOˆ›ÙK˜œ›ØYØ\İØ]Y[Ê	™œ˜[YJKˆ›İØÛÛœ˜[YN”Ş[˜Ô™\ÜÛœÙJÊHOˆ›ÙK˜œ›ØYØ\İÜŞ[˜Ê	™œ˜[YJKˆ›İØÛÛœ˜[YN”Ş[˜Ô™\]Y\İ
ÊHOˆÛÛ[YKËÈHÜİ™]™\ˆÙ[™È\Èœ˜[YHÚ[™ˆNÂˆX]Ú[]™\HÂˆÚÊ[]™\JHOˆİ]\Ë˜œ›ØYØ\İœ™XÛÜ™Ù[]™\J	™[]™\JKˆ\œŠ\œ›ÜŠHOˆÂˆİ]\Ë˜œ›ØYØ\İœ™XÛÜ™Ù˜Z[\™J
NÂˆÙ]Û\İÙ\œ›ÜŠİ]\Ë\ÚİÜ™]ÛÜšÑ\œ›Ü˜[œÜÜ
	™\œ›ÜŠK×Üİš[™Ê
JOÎÂˆBˆBˆBˆÚÊ

JBŸB