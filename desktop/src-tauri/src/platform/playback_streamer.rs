//! Owned desktop playback pump: drains a packetizer worker at real-time
//! cadence and forwards frames to the host transport worker.

use super::network::DesktopHostNetworkControl;
use crate::dto::DesktopErrorDto;
use silent_disco_core::audio::{DEFAULT_PACKET_DURATION_MS, StreamingPacketizeHandle};
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
    pump: Option<JoinHandle<()>>,
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

        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let pump = spawn_pump(
            packetizer,
            network,
            handle.clone(),
            session_id.clone(),
            stream_id.clone(),
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

    /// Blocks until the pump thread has exited. Call after [`Self::request_stop`].
    pub(super) fn join(mut self) {
        if let Some(pump) = self.pump.take() {
            drop(pump.join());
        }
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
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, DesktopErrorDto> {
    thread::Builder::new()
        .name("silent-disco-desktop-playback-pump".to_owned())
        .spawn(move || {
            run_pump(
                packetizer, &network, &handle, session_id, stream_id, &stop, &paused,
            );
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

fn run_pump(
    packetizer: StreamingPacketizeHandle,
    network: &Arc<DesktopHostNetworkControl>,
    handle: &CoreActorHandle,
    session_id: SessionId,
    stream_id: StreamId,
    stop: &AtomicBool,
    paused: &AtomicBool,
) {
    let packet_duration = Duration::from_millis(u64::from(DEFAULT_PACKET_DURATION_MS));
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
                drop(network.broadcast_playback_frame(frame));
                thread::sleep(packet_duration);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(packetizer.cancel_and_join());
    let host_stop_time_ms = network.transport_now().unwrap_or_default();
    drop(
        network.broadcast_playback_frame(ProtocolFrame::Control(ControlMessage::Stop(Stop {
            session_id,
            stream_id,
            host_stop_time_ms,
        }))),
    );
    drop(handle.submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Stopped)));
}
