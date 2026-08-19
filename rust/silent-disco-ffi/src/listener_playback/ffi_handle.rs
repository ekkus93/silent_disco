//! The `UniFFI` export surface: the foreign-facing handle wrapping one
//! [`ListenerPlaybackRuntime`], plus the crate-internal fast path used by
//! `listener_transport::handle`.

use std::sync::Arc;

use silent_disco_core::audio::{PlaybackPumpConfig, RenderRingConfig, SchedulerConfig};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};

use super::ffi_types::{
    FfiAudioPacket, FfiListenerPlaybackConfig, FfiListenerPlaybackError, FfiPlaybackDiagnostics,
    FfiSyncSampleOutcome,
};
use super::runtime::ListenerPlaybackRuntime;

/// Foreign-facing handle to one listener playback stream.
///
/// The platform submits arriving packets and raw sync exchanges and hands
/// [`FfiListenerPlaybackHandle::engine_token`] to native audio setup once.
/// Ordering, concealment, presentation-time pacing, clock estimation, and PCM
/// conversion all happen behind this boundary.
#[derive(Debug, uniffi::Object)]
pub struct FfiListenerPlaybackHandle {
    runtime: ListenerPlaybackRuntime,
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "uniffi::export requires owned parameters at the foreign boundary"
)]
#[uniffi::export]
impl FfiListenerPlaybackHandle {
    /// Starts a playback stream.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::InvalidConfiguration`] when any
    /// bound is rejected, or [`FfiListenerPlaybackError::PumpThread`] when the
    /// pump thread cannot be spawned.
    #[uniffi::constructor]
    pub fn open(config: FfiListenerPlaybackConfig) -> Result<Arc<Self>, FfiListenerPlaybackError> {
        let session_id = SessionId::new(&config.session_id).map_err(|error| {
            FfiListenerPlaybackError::InvalidConfiguration(format!("{error:?}"))
        })?;
        let stream_id = StreamId::new(&config.stream_id).map_err(|error| {
            FfiListenerPlaybackError::InvalidConfiguration(format!("{error:?}"))
        })?;
        let mut scheduler_config = SchedulerConfig::new(
            session_id,
            stream_id,
            config.sample_rate,
            config.host_start_time_ms,
            config.samples_per_packet,
            config.channels,
        );
        scheduler_config.startup_buffer_target_ms = config.startup_buffer_target_ms;
        scheduler_config.rebuffer_target_ms = config.rebuffer_target_ms;

        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config,
            RenderRingConfig {
                capacity_frames: usize::try_from(config.ring_capacity_frames).unwrap_or(usize::MAX),
                target_fill_frames: usize::try_from(config.ring_target_fill_frames)
                    .unwrap_or(usize::MAX),
            },
            PlaybackPumpConfig {
                volume: config.volume,
                write_lead_ms: config.write_lead_ms,
                max_prefill_ms: config.max_prefill_ms,
                target_depth_frames: usize::try_from(config.ring_target_fill_frames)
                    .unwrap_or(usize::MAX),
            },
            0.0,
        )?;
        Ok(Arc::new(Self { runtime }))
    }

    /// The opaque token to hand to native audio setup exactly once.
    #[must_use]
    pub fn engine_token(&self) -> i64 {
        i64::try_from(self.runtime.engine_token()).unwrap_or(i64::MAX)
    }

    /// Monotonic milliseconds on the timeline playback schedules against.
    /// Every local sync timestamp must come from here.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.runtime.now_ms()
    }

    /// Locks this runtime to a monotonic host clock sampled in the same process.
    ///
    /// Intended for the host's own local monitor. Remote listeners must continue
    /// using correlated sync probes so transport latency is measured rather than
    /// assumed away.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::Stopped`] once stopped.
    pub fn lock_same_process_host_clock(
        &self,
        host_monotonic_now_ms: u64,
    ) -> Result<(), FfiListenerPlaybackError> {
        self.runtime
            .lock_same_process_host_clock(host_monotonic_now_ms)?;
        Ok(())
    }

    /// Changes the gain used for subsequently converted frames.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::InvalidConfiguration`] for an invalid
    /// gain and `Stopped` once stopped.
    pub fn set_volume(&self, volume: f32) -> Result<(), FfiListenerPlaybackError> {
        self.runtime.set_volume(volume)?;
        Ok(())
    }

    /// Submits one arriving audio packet.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::Stopped`] once stopped.
    pub fn submit_packet(
        &self,
        packet: FfiAudioPacket,
        session_id: String,
        stream_id: String,
    ) -> Result<(), FfiListenerPlaybackError> {
        let datagram = AudioDatagram {
            session_id: SessionId::new(&session_id).map_err(|error| {
                FfiListenerPlaybackError::InvalidConfiguration(format!("{error:?}"))
            })?,
            stream_id: StreamId::new(&stream_id).map_err(|error| {
                FfiListenerPlaybackError::InvalidConfiguration(format!("{error:?}"))
            })?,
            sequence: PacketSequence::new(packet.sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: packet.sample_rate,
            channels: packet.channels,
            samples_per_packet: packet.samples_per_packet,
            first_sample_index: SampleIndex::new(packet.first_sample_index),
            host_presentation_time_ms: MonotonicMillis::new(packet.host_presentation_time_ms),
            payload: packet.payload,
        };
        self.runtime.submit_packet(datagram)?;
        Ok(())
    }

    /// Re-anchors the presentation-time base to `host_start_time_ms`,
    /// matching a host that just resumed from a pause and re-broadcast
    /// `StreamStart` with an updated anchor for the same stream. Applies in
    /// place; does not reopen the audio engine or reset the render ring.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::Stopped`] once stopped.
    pub fn reanchor_presentation_time(
        &self,
        host_start_time_ms: u64,
    ) -> Result<(), FfiListenerPlaybackError> {
        self.runtime
            .reanchor_presentation_time(host_start_time_ms)?;
        Ok(())
    }

    /// Registers one outbound sync probe before it is sent.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::Sync`] for a duplicate or
    /// over-capacity probe, or `Stopped` once stopped.
    pub fn begin_sync_probe(
        &self,
        correlation_id: u64,
        local_send_time_ms: u64,
    ) -> Result<(), FfiListenerPlaybackError> {
        self.runtime
            .begin_sync_probe(correlation_id, local_send_time_ms)?;
        Ok(())
    }

    /// Feeds one correlated four-timestamp sync response.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::Sync`] when the response does not
    /// correlate or its timestamps are impossible, or `Stopped` once stopped.
    pub fn observe_sync_response(
        &self,
        correlation_id: u64,
        echoed_local_send_time_ms: u64,
        host_receive_time_ms: u64,
        host_send_time_ms: u64,
        local_receive_time_ms: u64,
    ) -> Result<FfiSyncSampleOutcome, FfiListenerPlaybackError> {
        Ok(self
            .runtime
            .observe_sync_response(
                correlation_id,
                echoed_local_send_time_ms,
                host_receive_time_ms,
                host_send_time_ms,
                local_receive_time_ms,
            )?
            .into())
    }

    /// Current playback accounting.
    #[must_use]
    pub fn diagnostics(&self) -> FfiPlaybackDiagnostics {
        self.with_liveness(self.runtime.diagnostics().into())
    }

    /// Accounting as it stood when the stream was stopped, if it has been.
    #[must_use]
    pub fn final_diagnostics(&self) -> Option<FfiPlaybackDiagnostics> {
        self.runtime
            .final_diagnostics()
            .map(Into::into)
            .map(|diagnostics| self.with_liveness(diagnostics))
    }

    fn with_liveness(&self, mut diagnostics: FfiPlaybackDiagnostics) -> FfiPlaybackDiagnostics {
        let liveness = self.runtime.pump_liveness();
        diagnostics.pump_thread_tick_count = liveness.tick_count;
        diagnostics.pump_thread_last_tick_ms = liveness.last_tick_ms;
        diagnostics.contained_pump_panics = liveness.contained_panics;
        diagnostics
    }

    /// Captures released PCM to a WAV at `path` for offline analysis.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::DebugCapture`] when the file cannot
    /// be created, or `Stopped` once stopped.
    pub fn start_debug_capture(&self, path: String) -> Result<(), FfiListenerPlaybackError> {
        self.runtime.start_debug_capture(&path)?;
        Ok(())
    }

    /// First debug-capture failure, if capture stopped early.
    #[must_use]
    pub fn debug_capture_error(&self) -> Option<String> {
        self.runtime.debug_capture_error()
    }

    /// Drains the buffered tail, stops the pump, and releases the ring.
    /// Idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`FfiListenerPlaybackError::PumpThread`] when the pump thread
    /// ended by panicking.
    pub fn stop(&self) -> Result<(), FfiListenerPlaybackError> {
        self.runtime.stop()?;
        Ok(())
    }
}

/// Crate-internal surface, deliberately outside the `#[uniffi::export]`
/// block: these take shared-core types that have no foreign representation
/// and must not be exported.
impl FfiListenerPlaybackHandle {
    /// Submits an already-parsed core datagram straight into the runtime.
    ///
    /// The listener transport uses this to hand received audio to playback
    /// without a round trip through the foreign binding. The exported
    /// `submit_packet` exists for callers that only hold the wire fields;
    /// this one takes the datagram the transport already parsed, so
    /// forwarding costs no conversion and no payload copy.
    pub(crate) fn submit_core_datagram(
        &self,
        datagram: AudioDatagram,
    ) -> Result<(), FfiListenerPlaybackError> {
        self.runtime.submit_packet(datagram)?;
        Ok(())
    }
}
