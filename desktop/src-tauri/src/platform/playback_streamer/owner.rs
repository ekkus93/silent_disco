use super::network::DesktopHostNetworkControl;
use crate::dto::DesktopErrorDto;
use silent_disco_core::audio::{PacketizerWorkerErrorKind, StreamingPacketizeHandle};
use silent_disco_core::domain::{MonotonicMillis, PlaybackState, SessionId, StreamId};
use silent_disco_core::protocol::{
    AudioDatagram, ControlMessage, ProtocolFrame, Stop, StreamStart,
};
use silent_disco_core::runtime::{AudioEvent, CoreActorHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How long the pump waits for one packetizer frame before re-checking the
/// stop/pause flags. Short enough that an explicit stop is observed quickly.
const PUMP_RECV_TIMEOUT: Duration = Duration::from_millis(200);
/// How long the pump sleeps between checks while paused, instead of draining
/// the packetizer -- letting its bounded queue fill and backpressure the
/// decoder, which is the entire pause mechanism (no separate decoder pause).
const PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// How far ahead of the transport's current time the pump is allowed to send.
/// Sending strictly one packet every `packet_duration` gives listeners zero
/// replenishable lead: any transient stall downstream (network jitter, a
/// listener-side relay hiccup) can only ever drain the listener's buffer,
/// never refill it, since the host never gets ahead of real time in the
/// first place. Maintaining this bounded horizon instead lets the pump burst
/// out already-packetized audio up front and keep the horizon topped up
/// afterward, giving listeners a real, continuously-replenished surplus to
/// draw on instead of a one-time, only-shrinking startup cushion.
const SEND_AHEAD_HORIZON_MS: u64 = 1_000;
/// Poll granularity while the pump is holding a frame back to stay within
/// [`SEND_AHEAD_HORIZON_MS`].
const SEND_AHEAD_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Minimum advance, in stream-timeline milliseconds, between two
/// `PositionAdvanced` reports. At the 5ms packet duration, reporting every
/// frame would submit 200 actor inputs per second for a diagnostic value a
/// UI only needs a few times a second; this throttles the report, not the
/// underlying position, which still advances every frame.
const POSITION_REPORT_INTERVAL_MS: u64 = 250;

/// Owner of one active playback stream's real-time pump thread.
///
/// Fields other than the pump thread's own lifecycle are `pub(super)`
/// because `network.rs` reads/mutates them directly while already holding
/// `DesktopHostNetworkControl`'s state lock -- routing pause/resume/stop
/// through methods on this type that re-lock via `network` would deadlock.
pub(super) struct DesktopPlaybackStreamer {
    pub(super) session_id: SessionId,
    pub(super) stream_id: StreamId,
    pub(super) handle: CoreActorHandle,
    pub(super) paused: Arc<AtomicBool>,
    /// The stream's original `StreamStart`, exactly as first broadcast.
    /// `resume_playback` clones this and shifts `host_start_time_ms` by the
    /// accumulated pause offset to build the re-anchoring re-broadcast,
    /// rather than reconstructing the message from scratch.
    pub(super) stream_start: StreamStart,
    /// Transport-clock time the current pause began, or `0` while playing.
    /// Read and cleared by `resume_playback`, which is the sole reader.
    pub(super) paused_at_ms: Arc<AtomicU64>,
    /// Total milliseconds this stream has spent paused so far. Added to
    /// every subsequent audio frame's presentation time by the pump (see
    /// `apply_pause_offset`) so pacing keeps comparing against real time
    /// instead of a timeline that silently fell behind during the pause.
    pub(super) accumulated_pause_offset_ms: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<Result<(), DesktopErrorDto>>>,
}

impl DesktopPlaybackStreamer {
    /// Transitions the actor to `Playing`, broadcasts the stream-start
    /// control message, then starts the real-time pump thread.
    ///
    /// # Errors
    ///
    /// Returns a structured error if the actor rejects the `Playing`
    /// transition or the initial stream-start broadcast fails; the caller's
    /// packetizer is dropped (cancelling it) in that case.
    pub(super) fn start(
        packetizer: StreamingPacketizeHandle,
        session_id: SessionId,
        stream_id: StreamId,
        network: Arc<DesktopHostNetworkControl>,
        handle: CoreActorHandle,
    ) -> Result<Self, DesktopErrorDto> {
        handle
            .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .map_err(DesktopErrorDto::from)?;
        network.broadcast_playback_frame(packetizer.stream_start_message().clone())?;
        let stream_start = match packetizer.stream_start_message() {
            ProtocolFrame::Control(ControlMessage::StreamStart(start)) => start.clone(),
            _ => unreachable!("a packetizer's stream_start_message is always a StreamStart"),
        };
        let host_start_time_ms = stream_start.host_start_time_ms.get();

        // Standing up the monitor (if enabled) never fails this stream's
        // start -- a `None` tap here just means nothing gets forwarded
        // below; the monitor's own failure reason is recorded separately
        // and surfaced through `DesktopHostNetworkControl::monitor_status`
        // (Block 34.2: monitor failure never affects host transmission).
        let monitor_now = {
            let network = Arc::clone(&network);
            move || network.transport_now().ok().map(MonotonicMillis::get)
        };
        let monitor_tap = network.monitor.on_stream_started(
            session_id.clone(),
            stream_id.clone(),
            host_start_time_ms,
            stream_start.sample_rate,
            stream_start.channels,
            stream_start.samples_per_packet,
            monitor_now,
        );

        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let paused_at_ms = Arc::new(AtomicU64::new(0));
        let accumulated_pause_offset_ms = Arc::new(AtomicU64::new(0));
        let pump = spawn_pump(
            packetizer,
            network,
            handle.clone(),
            session_id.clone(),
            stream_id.clone(),
            host_start_time_ms,
            Arc::clone(&stop),
            Arc::clone(&paused),
            Arc::clone(&accumulated_pause_offset_ms),
            monitor_tap,
        )?;

        Ok(Self {
            session_id,
            stream_id,
            handle,
            paused,
            stream_start,
            paused_at_ms,
            accumulated_pause_offset_ms,
            stop,
            pump: Some(pump),
        })
    }

    /// Signals the pump thread to stop. The pump thread itself performs the
    /// `Stop` broadcast and the `Stopped` actor transition as part of its
    /// exit, so this is also correct for a stream already at end-of-file.
    pub(super) fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// Blocks until the pump thread has exited, reporting whatever it observed
    /// on the way out. Call after [`Self::request_stop`].
    ///
    /// # Errors
    ///
    /// Returns a structured error when the pump thread panicked, or when any
    /// of its shutdown steps failed -- cancelling the packetizer, broadcasting
    /// the `Stop` control message, or transitioning the actor to
    /// [`PlaybackState::Stopped`]. Discarding this is what let `stop_playback`
    /// report success while the session never actually left `Playing`.
    pub(super) fn join(mut self) -> Result<(), DesktopErrorDto> {
        let Some(pump) = self.pump.take() else {
            return Ok(());
        };
        pump.join().map_err(|_| {
            DesktopErrorDto::new(
                "desktop.playback.pump_panicked",
                "audio",
                "error",
                false,
                "the playback pump thread ended by panicking, so the stream was never stopped \
                 cleanly",
            )
        })?
    }

    /// True once the pump thread has exited on its own (end-of-file or a
    /// terminal failure), without anyone having called [`Self::request_stop`].
    #[must_use]
    pub(super) fn is_finished(&self) -> bool {
        self.pump.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for DesktopPlaybackStreamer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(pump) = self.pump.take()
            && let Some(failure) = implicit_join_failure(pump.join())
        {
            // Every normal owner removes the streamer and calls `join()`
            // explicitly, where the structured error is propagated. Reaching
            // this fallback while the thread is not already unwinding is an
            // ownership invariant violation: fail loudly instead of turning a
            // worker panic/error into an apparently clean drop. During an
            // existing panic, avoid a double-panic abort; the process is
            // already visibly failing and the worker has still been joined.
            assert!(
                std::thread::panicking(),
                "DesktopPlaybackStreamer dropped without explicit join: {failure}"
            );
        }
    }
}

fn implicit_join_failure(
    outcome: std::thread::Result<Result<(), DesktopErrorDto>>,
) -> Option<String> {
    match outcome {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!(
            "playback pump failed during implicit shutdown: {}",
            error.message
        )),
        Err(_) => Some("playback pump panicked during implicit shutdown".to_owned()),
    }
}
