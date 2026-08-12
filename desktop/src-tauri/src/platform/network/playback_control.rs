//! Playback stream control: starting/pausing/resuming/stopping the active
//! broadcast stream against the host session established by
//! [`super::host_control`], plus the live decode/packetizer diagnostics
//! readers for that stream.

use super::DesktopNetworkError;
use super::host_control::DesktopHostNetworkControl;
use crate::dto::DesktopErrorDto;
use crate::platform::playback_streamer::DesktopPlaybackStreamer;
use silent_disco_core::domain::{MonotonicMillis, PlaybackState};
use silent_disco_core::protocol::{ControlMessage, Pause, ProtocolFrame, StreamStart};
use silent_disco_core::runtime::{AudioEvent, CoreActorHandle};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Live queue-diagnostics readers for one active playback stream.
pub(crate) struct StreamDiagnostics {
    pub(crate) decode: silent_disco_core::audio::DecodeStatisticsReader,
    pub(crate) packetize: silent_disco_core::audio::PacketizeStatisticsReader,
}

/// A point-in-time read of [`StreamDiagnostics`], safe to hand to a DTO
/// builder without exposing the underlying reader types.
pub(crate) struct StreamDiagnosticsSnapshot {
    pub(crate) decode: silent_disco_core::audio::DecodeStatistics,
    /// `(queued_packets, queue_capacity, backpressure_events, emitted_packets)`,
    /// matching [`silent_disco_core::audio::StreamingPacketizeHandle::statistics`]'s
    /// own tuple order.
    pub(crate) packetize: (usize, usize, u64, u64),
}

impl DesktopHostNetworkControl {
    /// Live decode/packetizer queue diagnostics for the current or most
    /// recently active stream (Block 35.1 "decoder/source queues"/
    /// "packetizer") -- `None` if no stream has ever started for this
    /// binding, or if no host session is currently active at all.
    #[must_use]
    pub(crate) fn stream_diagnostics_snapshot(&self) -> Option<StreamDiagnosticsSnapshot> {
        let state = self.state.lock().ok()?;
        let active = state.active.as_ref()?;
        let diagnostics = active.stream_diagnostics.as_ref()?;
        Some(StreamDiagnosticsSnapshot {
            decode: diagnostics.decode.snapshot(),
            packetize: diagnostics.packetize.snapshot(),
        })
    }

    /// Resolves the current staged/decoded/packetized source into an active
    /// playback stream, transitioning the actor to `Playing` and starting
    /// the real-time broadcast pump. See [`DesktopPlaybackStreamer::start`].
    ///
    /// # Errors
    ///
    /// Returns a structured error when no host session is active, playback
    /// is already active and still running, or the actor rejects the
    /// `Playing` transition.
    pub(crate) fn start_playback(
        self: &Arc<Self>,
        packetizer: silent_disco_core::audio::StreamingPacketizeHandle,
        session_id: silent_disco_core::domain::SessionId,
        stream_id: silent_disco_core::domain::StreamId,
        handle: CoreActorHandle,
        decode_diagnostics: silent_disco_core::audio::DecodeStatisticsReader,
        packetize_diagnostics: silent_disco_core::audio::PacketizeStatisticsReader,
    ) -> Result<(), DesktopErrorDto> {
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| DesktopNetworkError::poisoned().dto())?;
            let Some(active) = state.active.as_mut() else {
                return Err(DesktopNetworkError::unavailable(
                    "starting playback requires an active desktop host session",
                )
                .dto());
            };
            match &active.playback {
                Some(playback) if !playback.is_finished() => {
                    return Err(DesktopNetworkError::invalid_state(
                        "playback is already active for this host session",
                    )
                    .dto());
                }
                _ => {
                    // A previous stream that ended by failing must surface here
                    // rather than being buried by starting the next one.
                    if let Some(finished) = active.playback.take() {
                        finished.join()?;
                    }
                }
            }
        }
        let streamer = DesktopPlaybackStreamer::start(
            packetizer,
            session_id,
            stream_id,
            Arc::clone(self),
            handle,
        )?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "desktop host session ended while playback was starting",
            )
            .dto());
        };
        active.playback = Some(streamer);
        active.stream_diagnostics = Some(StreamDiagnostics {
            decode: decode_diagnostics,
            packetize: packetize_diagnostics,
        });
        Ok(())
    }

    /// Pauses the active playback stream after a validated actor transition.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no playback is active or the actor
    /// rejects the `Paused` transition (e.g. not currently playing).
    pub(crate) fn pause_playback(&self) -> Result<(), DesktopErrorDto> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "pausing playback requires an active desktop host session",
            )
            .dto());
        };
        let Some(playback) = active.playback.as_ref() else {
            return Err(DesktopNetworkError::invalid_state("no playback is active").dto());
        };
        playback
            .handle
            .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Paused))
            .map_err(DesktopErrorDto::from)?;
        let host_pause_time_ms = active.runtime.observed_at();
        active
            .runtime
            .broadcast_frame(ProtocolFrame::Control(ControlMessage::Pause(Pause {
                session_id: playback.session_id.clone(),
                stream_id: playback.stream_id.clone(),
                host_pause_time_ms,
            })))
            .map_err(DesktopNetworkError::dto)?;
        playback
            .paused_at_ms
            .store(host_pause_time_ms.get(), Ordering::Release);
        playback.paused.store(true, Ordering::Release);
        Ok(())
    }

    /// Resumes the active, paused playback stream after a validated actor
    /// transition, re-broadcasting `StreamStart` with a presentation anchor
    /// shifted forward by this pause's duration, so a listener that missed
    /// frames while paused re-anchors its own presentation-time expectations
    /// to match the same shift the pump now applies to every subsequent
    /// audio frame (see [`super::super::playback_streamer`]'s pause-offset
    /// accounting). Reusing the same `stream_id` lets the listener apply
    /// this in place rather than tearing down and reopening its audio
    /// engine.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no playback is active or the actor
    /// rejects the `Playing` transition (e.g. not currently paused).
    pub(crate) fn resume_playback(&self) -> Result<(), DesktopErrorDto> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "resuming playback requires an active desktop host session",
            )
            .dto());
        };
        let Some(playback) = active.playback.as_ref() else {
            return Err(DesktopNetworkError::invalid_state("no playback is active").dto());
        };
        playback
            .handle
            .submit_audio_event(AudioEvent::PlaybackStateChanged(PlaybackState::Playing))
            .map_err(DesktopErrorDto::from)?;

        // A resume issued while already playing is a stale/duplicate command
        // today's actor checks still accept -- it must stay a pure no-op
        // beyond the transition above. `paused_at_ms` is still its "never
        // paused" sentinel of zero in that case, so computing an offset from
        // it would fabricate a bogus multi-<x>-long "pause" out of nothing
        // and corrupt the anchor instead of fixing it; broadcasting here
        // would also contend with the pump's own real-time frames on the
        // same bounded queue for no reason.
        if !playback.paused.swap(false, Ordering::AcqRel) {
            return Ok(());
        }

        let now = active.runtime.observed_at();
        let paused_at_ms = playback.paused_at_ms.swap(0, Ordering::AcqRel);
        let elapsed_ms = now.get().saturating_sub(paused_at_ms);
        let total_offset_ms = playback
            .accumulated_pause_offset_ms
            .fetch_add(elapsed_ms, Ordering::AcqRel)
            .saturating_add(elapsed_ms);
        let reanchored_start = StreamStart {
            host_start_time_ms: MonotonicMillis::new(
                playback
                    .stream_start
                    .host_start_time_ms
                    .get()
                    .saturating_add(total_offset_ms),
            ),
            ..playback.stream_start.clone()
        };
        active
            .runtime
            .broadcast_frame(ProtocolFrame::Control(ControlMessage::StreamStart(
                reanchored_start,
            )))
            .map_err(DesktopNetworkError::dto)?;

        Ok(())
    }

    /// Signals the active playback stream to stop and blocks until its pump
    /// thread performs the `Stop` broadcast, the `Stopped` actor transition,
    /// and exits.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no playback is active.
    pub(crate) fn stop_playback(&self) -> Result<(), DesktopErrorDto> {
        let playback = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| DesktopNetworkError::poisoned().dto())?;
            let Some(active) = state.active.as_mut() else {
                return Err(DesktopNetworkError::unavailable(
                    "stopping playback requires an active desktop host session",
                )
                .dto());
            };
            active
                .playback
                .take()
                .ok_or_else(|| DesktopNetworkError::invalid_state("no playback is active").dto())?
        };
        playback.request_stop();
        playback.join()
    }

    #[cfg(test)]
    pub(in crate::platform) fn stop_transport_worker_for_test(&self) -> Result<(), DesktopErrorDto> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_mut() else {
            return Err(DesktopNetworkError::unavailable(
                "stopping the test transport worker requires an active host session",
            )
            .dto());
        };
        active
            .runtime
            .stop_worker_for_test()
            .map_err(DesktopNetworkError::dto)
    }

    /// Returns the transport worker's current monotonic time, the same
    /// clock basis used for sync responses -- callers computing a playback
    /// timestamp (e.g. `host_start_time_ms`) must use this, not a fresh
    /// clock, so presentation times remain comparable to sync samples.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no host session is active.
    pub(crate) fn transport_now(&self) -> Result<MonotonicMillis, DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_ref() else {
            return Err(DesktopNetworkError::unavailable(
                "desktop host network endpoint is not active",
            )
            .dto());
        };
        Ok(active.runtime.observed_at())
    }

    /// Enqueues one control/sync/audio frame for the host transport worker
    /// to broadcast. Used by the playback pump thread, which is never
    /// already holding this control's state lock.
    ///
    /// # Errors
    ///
    /// Returns a structured error when no host session is active or the
    /// worker's broadcast queue is full/unavailable.
    pub(crate) fn broadcast_playback_frame(
        &self,
        frame: ProtocolFrame,
    ) -> Result<(), DesktopErrorDto> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        let Some(active) = state.active.as_ref() else {
            return Err(DesktopNetworkError::unavailable(
                "desktop host network endpoint is not active",
            )
            .dto());
        };
        active
            .runtime
            .broadcast_frame(frame)
            .map_err(DesktopNetworkError::dto)
    }

    /// Reports whether a playback stream is already running for the active
    /// host session, without mutating any actor or network state. Callers
    /// starting a new stream must consult this *before* submitting any
    /// actor transition (e.g. `Buffering`) -- otherwise a duplicate/stale
    /// Start command still visibly corrupts the authoritative snapshot with
    /// an `Error` state even though the real, already-running stream is
    /// untouched and the duplicate is correctly rejected underneath.
    pub(crate) fn playback_is_active(&self) -> Result<bool, DesktopErrorDto> {
        let state = self.state
            .lock()
            .map_err(|_| DesktopNetworkError::poisoned().dto())?;
        Ok(state
            .active
            .as_ref()
            .and_then(|active| active.playback.as_ref())
            .is_some_and(|playback| !playback.is_finished()))
    }
}
