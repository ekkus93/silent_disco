//! Owned desktop playback pump: drains a packetizer worker and forwards
//! frames to the host transport worker, staying within a bounded send-ahead
//! horizon of the transport's current time rather than pacing one packet per
//! `packet_duration`.

use super::network::DesktopHostNetworkControl;
use crate::dto::DesktopErrorDto;
use silent_disco_core::audio::{PacketizerWorkerErrorKind, StreamingPacketizeHandle};
use silent_disco_core::domain::{PlaybackState, SessionId, StreamId};
use silent_disco_core::protocol::{ControlMessage, ProtocolFrame, Stop};
use silent_disco_core::runtime::{AudioEvent, CoreActorHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
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
        let host_start_time_ms = match packetizer.stream_start_message() {
            ProtocolFrame::Control(ControlMessage::StreamStart(start)) => {
                start.host_start_time_ms.get()
            }
            _ => unreachable!("a packetizer's stream_start_message is always a StreamStart"),
        };

        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let pump = spawn_pump(
            packetizer,
            network,
            handle.clone(),
            session_id.clone(),
            stream_id.clone(),
            host_start_time_ms,
            Arc::clone(&stop),
            Arc::clone(&paused),
        )?;

        Ok(Self {
            session_id,
            stream_id,
            handle,
            paused,
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
) -> Result<(), DesktopErrorDto> {
    let mut last_reported_position_ms: Option<u64> = None;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if paused.load(Ordering::Acquire) {
            thread::sleep(PAUSE_POLL_INTERVAL);
            continue;
        }
        match packetizer.recv_timeout(PUMP_RECV_TIMEOUT) {
            Ok(frame) => {
                if !wait_until_within_send_ahead_horizon(&frame, network, stop) {
                    break;
                }
                report_position_if_due(
                    &frame,
                    host_start_time_ms,
                    &mut last_reported_position_ms,
                    handle,
                    &stream_id,
                );
                drop(network.broadcast_playback_frame(frame));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
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
        })))
    });
    let stopped_result = if ended_naturally {
        handle.submit_audio_event(AudioEvent::EndOfStream {
            stream_id: ending_stream_id,
        })
    } else {
        handle.submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped))
    }
    .map_err(DesktopErrorDto::from);
    packetizer_result.and(broadcast_result).and(stopped_result)
}

/// Submits a throttled `PositionAdvanced` event for one emitted audio frame.
///
/// Position is computed from the frame's own presentation time against the
/// stream's start -- the authoritative timeline this stream is scheduled
/// against -- rather than from wall-clock elapsed time, which would drift
/// under pause or send-ahead bursting. A failed submission is advisory-grade,
/// the same severity as the audio broadcast just below this call in the
/// caller, and is handled the same way: retried on the next due frame rather
/// than treated as a reason to stop the stream.
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
        .saturating_sub(host_start_time_ms);
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

/// Blocks, in short stop-responsive increments, until `frame`'s presentation
/// time is no more than [`SEND_AHEAD_HORIZON_MS`] ahead of the transport's
/// current time. Non-audio frames (there are none on this path today, but
/// [`StreamingPacketizeHandle::recv_timeout`] returns `ProtocolFrame`) pass
/// through immediately. Returns `false` if `stop` fired while waiting, so
/// the caller can exit without sending a stale frame.
fn wait_until_within_send_ahead_horizon(
    frame: &ProtocolFrame,
    network: &Arc<DesktopHostNetworkControl>,
    stop: &AtomicBool,
) -> bool {
    let ProtocolFrame::Audio(datagram) = frame else {
        return true;
    };
    let presentation_time_ms = datagram.host_presentation_time_ms.get();
    loop {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        let Ok(now) = network.transport_now() else {
            return true;
        };
        let lead_ms = presentation_time_ms.saturating_sub(now.get());
        if lead_ms <= SEND_AHEAD_HORIZON_MS {
            return true;
        }
        thread::sleep(SEND_AHEAD_POLL_INTERVAL);
    }
}
