//! Owned desktop playback pump: drains a packetizer worker and forwards
//! frames to the host transport worker, staying within a bounded send-ahead
//! horizon of the transport's current time rather than pacing one packet per
//! `packet_duration`.

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
                 cleanle",
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
        if let Some(pump) = self.pump.take() {
            // Nothing can be returned from `drop`, so this is the one path that
            // cannot report a failing pump. `join` is the supported route and
            // every caller that can propagate uses it; reaching here means the
            // streamer was dropped without being stopped.
            drop(pump.join());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_pump(
    packetizer: StreamingPacketizeHandle,
    network: Arc<DesktopHostNetworkControl>,
    handle: CoreActorHandle,
    session_id: SessionId,
    stream_id: StreamId,
    host_start_time_ms: u64,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    accumulated_pause_offset_ms: Arc<AtomicU64>,
    monitor_tap: Option<SyncSender<AudioDatagram>>,
) -> Result<JoinHandle<Result<(), DesktopErrorDto>>, DesktopErrorDto> {
    thread::Builder::new()
        .name("silent-disco-desktop-playback-pump".to_owned())
        .spawn(move || {
            run_pump(
                packetizer,
                &network,
                &handle,
                session_id,
                stream_id,
                host_start_time_ms,
                &stop,
                &paused,
                &accumulated_pause_offset_ms,
                monitor_tap.as_ref(),
            )
        })
        .map_err(|error| {
            DesktopErrorDto::new(
                "desktop.playback.pump_start_failed",
                "audio",
                "error",
                true,
                &format!("failed to start desktop playback pump: {error}"),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn run_pump(
    packetizer: StreamingPacketizeHandle,
    network: &Arc<DesktopHostNetworkControl>,
    handle: &CoreActorHandle,
    session_id: SessionId,
    stream_id: StreamId,
    host_start_time_ms: u64,
    stop: &AtomicBool,
    paused: &AtomicBool,
    accumulated_pause_offset_ms: &AtomicU64,
    monitor_tap: Option<&SyncSender<AudioDatagram>>,
) -> Result<(), DesktopErrorDto> {
    let mut last_reported_position_ms: Option<u64> = None;
    let mut streaming_error: Option<DesktopErrorDto> = None;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if paused.load(Ordering::Acquire) {
            thread::sleep(PAUSE_POLL_INTERVAL);
            continue;
        }
        match packetizer.recv_timeout(PUMP_RECV_TIMEOUT) {
            Ok(mut frame) => {
                // Position must reflect actual song content progress, which
                // is exactly what the packetizer's own (unshifted) sequence
                // math already gives -- compute it before the pause offset
                // below inflates the frame's presentation time by however
                // long the stream has spent paused so far.
                report_position_if_due(
                    &frame,
                    host_start_time_ms,
                    &mut last_reported_position_ms,
                    handle,
                    &stream_id,
                );
                apply_pause_offset(
                    &mut frame,
                    accumulated_pause_offset_ms.load(Ordering::Acquire),
                );
                forward_to_monitor(&frame, monitor_tap);
                match wait_until_within_send_ahead_horizon(&frame, network, stop) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        streaming_error = Some(error);
                        break;
                    }
                }
                if let Err(error) = network.broadcast_playback_frame(frame) {
                    streaming_error = Some(error);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    // Tearing the monitor down is attempted unconditionally, exactly like
    // every other shutdown step below -- and, like them, is never allowed
    // to prevent listeners being told the stream ended or the actor
    // leaving `Playing`. Ordered before the packetizer cancellation only so
    // the monitor's own thread/device stop first, not because anything
    // downstream depends on that order.
    network.monitor.on_stream_stopped();
    // Every shutdown step is attempted even when an earlier one fails -- a
    // packetizer that will not cancel must not prevent the listeners being
    // told the stream ended, nor the actor leaving `Playing` -- and the first
    // failure is what gets reported.
    let packetizer_summary = packetizer.cancel_and_join();
    // The packetizer only ever reaches `Ok` by emitting its final packet and
    // exiting on its own -- every other exit, cancellation included, is an
    // `Err`. So `Ok` here means the source genuinely finished, distinct from
    // being told to stop, and that distinction is worth keeping visible to a
    // playback UI rather than folding both into the same generic `Stopped`.
    let ended_naturally = packetizer_summary.is_ok();
    let packetizer_result = match packetizer_summary {
        Ok(_) => Ok(()),
        // Cancellation is precisely what stopping asks the worker to do, so it
        // is this path's normal outcome rather than a failure. Every other kind
        // -- a decode failure, a packetize failure, a panicking worker -- is
        // real and must not be reported as a clean stop.
        Err(error) if error.kind == PacketizerWorkerErrorKind::Cancelled => Ok(()),
        Err(error) => Err(DesktopErrorDto::new(
            "desktop.playback.packetizer_shutdown_failed",
            "audio",
            "error",
            false,
            &error.message,
        )),
    };
    let ending_stream_id = stream_id.clone();
    let broadcast_result = network.transport_now().and_then(|host_stop_time_ms| {
        network.broadcast_playback_frame(ProtocolFrame::Control(ControlMessage::Stop(Stop {
            session_id,
            stream_id,
            host_stop_time_ms,
        }))
    });
    let stopped_result = if ended_naturally {
        handle.submit_audio_event(AudioEvent::EndOfStream {
            stream_id: ending_stream_id,
        })
    } else {
        handle.submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
    }
    .map_err(DesktopErrorDto::from);
    if let Some(primary) = streaming_error {
        return Err(primary
            .with_appended_cleanup(packetizer_result.err())
            .with_appended_cleanup(broadcast_result.err())
            .with_appended_cleanup(stopped_result.err()));
    }
    packetizer_result.and(broadcast_result).and(stopped_result)
}

/// Submits a throttled `PositionAdvanced` event for one emitted audio frame.
///
/// Position is computed from the frame's own presentation time against the
/// stream's start -- the authoritative timeline this stream is scheduled
/// against -- rather than from wall-clock elapsed time, which would drift
/// under pause or send-ahead bursting. A failed position submission remains
/// advisory and is retried on the next due frame; unlike a transport broadcast
/// failure, it does not mean listener delivery has become unavailable.
fn report_position_if_due(
    frame: &ProtocolFrame,
    host_start_time_ms: u64,
    last_reported_position_ms: &mut Option<u64>,
    handle: &CoreActorHandle,
    stream_id: &StreamId,
) {
    let ProtocolFrame::Audio(datagram) = frame else {
        return;
    };
    let position_ms = datagram
        .host_presentation_time_ms
        .get()
        .saturating_sub(hhost_start_time_ms);
    let due = match *last_reported_position_ms {
        Some(previous) => position_ms >= previous.saturating_add(POSITION_REPORT_INTERVAL_MS),
        None => true,
    };
    if !due {
        return;
    }
    if handle
        .submit_audio_event(AudioEvent::PositionAdvanced {
            stream_id: stream_id.clone(),
            position_ms,
        })
        .is_ok()
    {
        *last_reported_position_ms = Some(position_ms);
    }
}

/// Adds the stream's accumulated pause offset to one outgoing audio frame's
/// presentation time. The packetizer computes `host_presentation_time_ms`
/// from a fixed anchor set once at stream start, so it has no way to know
/// real time kept moving while the pump stopped draining it for a pause --
/// every frame it produces from then on reads as further and further behind
/// schedule. Left uncorrected, [`wait_until_within_send_ahead_horizon`]
/// reads that lag as "already late", disabling the send-ahead throttle and
/// bursting the whole backlog at once, which is what overwhelmed the
/// broadcast queue on a real device after a pause/resume. Non-audio frames
/// are untouched; a zero offset (the common case, stream never paused) is a
/// no-op.
fn apply_pause_offset(frame: &mut ProtocolFrame, offset_ms: u64) {
    if offset_ms == 0 {
        return;
    }
    if let ProtocolFrame::Audio(datagram) = frame {
        datagram.host_presentation_time_ms = MonotonicMillis::new(
            datagram
                .host_presentation_time_ms
                .get()
                .saturating_add(offset_ms),
        );
    }
}

/// Forwards one outgoing audio frame's datagram to the local monitor pump,
/// if a monitor is currently active for this stream (Block 34).
///
/// Best-effort and non-blocking (`try_send`): a monitor that cannot keep up
/// simply misses frames rather than ever slowing or blocking the network
/// broadcast path below this call, which must never be affected by monitor
/// health (34.2 policy). Runs after [`apply_pause_offset`] so the monitor's
/// own scheduler sees the same pause-corrected timeline the network
/// broadcast does, and before the send-ahead wait, since the monitor has no
// use for that network-specific pacing at all.
fn forward_to_monitor(frame: &ProtocolFrame, monitor_tap: Option<&SyncSender<AudioDatagram>>) {
    let (Some(tap), ProtocolFrame::Audio(datagram)) = (monitor_tap, frame) else {
        return;
    };
    drop(tap.try_send(datagram.clone()));
}

/// Blocks, in short stop-responsive increments, until `frame`'s presentation
/// time is no more than [`SEND_AHEAD_HORIZON_MS`] ahead of the transport's
/// current time. Non-audio frames (there are none on this path today, but
/// [`StreamingPacketizeHandle::recv_timeout`] returns `ProtocolFrame`) pass
/// through immediately. Returns `Ok(false)` if `stop` fired while waiting, so
/// the caller can exit without sending a stale frame, and propagates a
/// transport-clock failure instead of silently treating it as permission to
/// send.
fn wait_until_within_send_ahead_horizon(
    frame: &ProtocolFrame,
    network: &Arc<DesktopHostNetworkControl>,
    stop: &AtomicBool,
) -> Result<bool, DesktopErrorDto> {
    let ProtocolFrame::Audio(datagram) = frame else {
        return Ok(true);
    };
    let presentation_time_ms = datagram.host_presentation_time_ms.get();
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(false);
        }
        let now = network.transport_now()?;
        let lead_ms = presentation_time_ms.saturating_sub(now.get());
        if lead_ms <= SEND_AHEAD_HORIZON_MS {
            return Ok(true);
        }
        thread::sleep(SEND_AHEAD_POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_pause_offset, forward_to_monitor};
    use silent_disco_core::domain::{
        MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
    };
    use silent_disco_core::protocol::{
        AudioCodec, AudioDatagram, ControlMessage, Disconnect, ProtocolFrame,
    };

    fn audio_frame(host_presentation_time_ms: u64) -> ProtocolFrame {
        ProtocolFrame::Audio(AudioDatagram {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            stream_id: StreamId::new("stream-pump-test").expect("stream id"),
            sequence: PacketSequence::new(0),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: 240,
            first_sample_index: SampleIndex::new(0),
            host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
            payload: vec![0_u8; 240 * 2 * 2],
        })
    }

    /// This is the fix's whole mechanism: a resumed stream's packetizer keeps
    /// computing presentation times from its original, now-stale anchor, so
    /// the pump must add back exactly the elapsed pause duration before the
    /// send-ahead pacing check sees the frame -- get the arithmetic wrong and
    /// either pacing stays broken (offset too small) or every frame reads as
    /// further in the future than it really is (offset too large).
    #[test]
    fn adds_the_offset_to_an_audio_frames_presentation_time() {
        let mut frame = audio_frame(1_000);
        apply_pause_offset(&mut frame, 500);
        match frame {
            ProtocolFrame::Audio(datagram) => {
                assert_eq!(datagram.host_presentation_time_ms.get(), 1_500);
            }
            ProtocolFrame::Control(_) => panic!("expected an audio frame, got a control frame"),
            _ => panic!("expected an audio frame"),
        }
    }

    /// The overwhelmingly common case -- a stream that has never paused --
    /// must be a true no-op, not just a zero-valued shift, since this runs on
    /// every single frame of every stream.
    #[test]
    fn a_zero_offset_leaves_the_presentation_time_unchanged() {
        let mut frame = audio_frame(1_000);
        apply_pause_offset(&mut frame, 0);
        match frame {
            ProtocolFrame::Audio(datagram) => {
                assert_eq!(datagram.host_presentation_time_ms.get(), 1_000);
            }
            ProtocolFrame::Control(_) => panic!("expected an audio frame, got a control frame"),
            _ => panic!("expected an audio frame"),
        }
    }

    #[test]
    fn a_non_audio_frame_is_left_untouched() {
        let mut frame = ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            listener_id: silent_disco_core::domain::DeviceId::new("device-pump-test")
                .expect("device id"),
            reason: "test".to_owned(),
        }));
        let before = frame.clone();
        apply_pause_offset(&mut frame, 500);
        assert!(frame == before, "a non-audio frame must never be mutated");
    }

    /// Saturates rather than wraps -- a wrapped timestamp would read as an
    /// enormous negative lead in `wait_until_within_send_ahead_horizon`'s
    /// `saturating_sub`, which is exactly the "reads as due immediately"
    /// failure this fix exists to prevent, just triggered a different way.
    #[test]
    fn saturates_instead_of_overflowing() {
        let mut frame = audio_frame(u64::MAX - 10);
        apply_pause_offset(&mut frame, 500);
        match frame {
            ProtocolFrame::Audio(datagram) => {
                assert_eq!(datagram.host_presentation_time_ms.get(), u64::MAX);
            }
            ProtocolFrame::Control(_) => panic!("expected an audio frame, got a control frame"),
            _ => panic!("expected an audio frame"),
        }
    }

    /// Block 34.3 "host transmit continues or stops exactly according to
    /// policy": this is the actual mechanism that guarantees it. A
    /// capacity-0 `sync_channel` (a rendezvous channel) makes `try_send`
    /// fail immediately whenever no receiver is concurrently waiting --
    /// exactly the "monitor cannot keep up right now" case -- and
    /// `forward_to_monitor` must swallow that and return normally rather
    /// than blocking or propagating an error, since [`run_pump`]'s call
    /// site always reaches the unconditional broadcast immediately after
    /// this call regardless of what happened here.
    #[test]
    fn forwarding_to_a_tap_that_cannot_accept_right_now_never_blocks_or_panics() {
        let (tap, _receiver) = std::sync::mpsc::sync_channel(0);
        let frame = audio_frame(1_000);
        forward_to_monitor(&frame, Some(&tap));
        // Reaching here at all is the proof: a blocking or panicking
        // implementation would never return control to this line.
    }

    /// The overwhelmingly common case -- no monitor active at all -- must
    /// also be a true no-op.
    #[test]
    fn forwarding_with_no_monitor_tap_is_a_no_op() {
        let frame = audio_frame(1_000);
        forward_to_monitor(&frame, None);
    }

    /// A non-audio frame must never be forwarded to the monitor, which only
    /// ever understands `AudioDatagram`s.
    #[test]
    fn a_non_audio_frame_is_never_forwarded_to_the_monitor() {
        let (tap, receiver) = std::sync::mpsc::sync_channel(1);
        let frame = ProtocolFrame::Control(ControlMessage::Disconnect(Disconnect {
            session_id: SessionId::new("session-pump-test").expect("session id"),
            listener_id: silent_disco_core::domain::DeviceId::new("device-pump-test")
                .expect("device id"),
            reason: "test".to_owned(),
        }));
        forward_to_monitor(&frame, Some(&tap));
        assert!(
            receiver.try_recv().is_err(),
            "a control frame must never reach the monitor tap"
         );
    }
}
