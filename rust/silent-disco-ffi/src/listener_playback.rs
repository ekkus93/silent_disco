//! Rust-owned listener playback runtime: one scheduler, one render ring, and
//! one dedicated pump thread per stream.
//!
//! Kotlin/Swift submit arriving packets and clock-offset updates through this
//! handle and hand [`FfiListenerPlaybackHandle::engine_token`] to native audio
//! setup exactly once. Everything between an arriving packet and the render
//! ring — ordering, concealment, presentation-time pacing, and PCM conversion
//! — happens here, so no platform layer decides when audio plays or what
//! covers a gap.

use core::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use silent_disco_core::audio::PlaybackPhase;
use silent_disco_core::audio::{
    DebugPcmRecorder, PlaybackDiagnostics, PlaybackPump, PlaybackPumpConfig, PlaybackScheduler,
    PumpTick, RenderRingConfig, SchedulerConfig,
};
use silent_disco_core::domain::{
    MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId, SyncConfidence,
};
use silent_disco_core::protocol::{AudioCodec, AudioDatagram};
use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};

use crate::audio_abi::{AudioAbiError, register_render_ring, release_render_ring};

/// How often the pump thread wakes to check whether audio is due.
const PUMP_TICK_INTERVAL: Duration = Duration::from_millis(10);
/// Safety bound on frames drained in one pump wake-up. The write lead and
/// ring depth cap are what actually stop the drain; this only prevents an
/// unbounded loop if both were ever misconfigured. Generous enough to cover a
/// full write lead of the shortest supported packets plus catch-up.
const MAX_FRAMES_PER_TICK: usize = 512;
/// How often the stopping thread checks whether the render ring has drained.
const RING_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Overall ceiling on waiting for the ring to drain at stop. Comfortably above
/// a full ring at the supported sample rates, so it only ever bounds the case
/// where nothing is consuming.
const RING_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the ring may fail to shrink before the consumer is treated as not
/// running, so a closed or failed output does not cost the full timeout.
const RING_DRAIN_STALL_LIMIT: Duration = Duration::from_millis(150);

/// Explicit, distinguishable failure exposed to the platform binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenerPlaybackError {
    /// The requested scheduler, ring, or pump configuration was rejected.
    InvalidConfiguration(String),
    /// The handle was already stopped.
    Stopped(String),
    /// The pump thread could not be started, or ended abnormally.
    PumpThread(String),
    /// A sync probe or response was rejected by the estimator.
    Sync(String),
    /// The optional debug PCM capture could not be started.
    DebugCapture(String),
}

impl fmt::Display for ListenerPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Stopped(message)
            | Self::PumpThread(message)
            | Self::Sync(message)
            | Self::DebugCapture(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ListenerPlaybackError {}

impl From<AudioAbiError> for ListenerPlaybackError {
    fn from(error: AudioAbiError) -> Self {
        Self::InvalidConfiguration(format!("{error:?}"))
    }
}

/// Monotonic milliseconds since this runtime started, matching the local
/// timeline the caller's clock-offset estimates are expressed against.
#[derive(Debug)]
struct PumpClock {
    origin: Instant,
}

impl PumpClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// Outcome of feeding one correlated sync response to the runtime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncSampleOutcome {
    /// True when the sample's round-trip time was inside the acceptance
    /// window and it updated the estimate.
    pub accepted: bool,
    /// Current offset estimate, in milliseconds.
    pub offset_ms: f64,
    /// Current skew estimate, in parts per million.
    pub skew_ppm: f64,
    /// Round-trip time of the current estimate, in milliseconds.
    pub round_trip_time_ms: f64,
    /// Offset dispersion across the samples behind the current estimate.
    pub jitter_ms: f64,
    /// The estimator's own confidence in the current estimate.
    pub confidence: SyncConfidence,
    /// Accepted samples behind the current estimate.
    pub accepted_sample_count: usize,
    /// True once playback has a real offset and may start.
    pub sync_locked: bool,
}

/// State shared between the owning handle and its pump thread.
#[derive(Debug)]
struct Shared {
    pump: Mutex<PlaybackPump>,
    /// Owned here rather than by the platform: a listener that estimates its
    /// own offset or skew is duplicating domain logic, and doing it in the
    /// platform layer is what produced a physically impossible skew (and
    /// total silence) in the previous implementation.
    estimator: Mutex<ClockSyncEstimator>,
    running: AtomicBool,
}

impl Shared {
    fn lock_pump(&self) -> MutexGuard<'_, PlaybackPump> {
        self.pump.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_estimator(&self) -> MutexGuard<'_, ClockSyncEstimator> {
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// One live listener playback stream.
///
/// Owns the scheduler, the render ring's producer half, the token its consumer
/// half is registered under, and the pump thread driving one into the other.
/// [`Self::stop`] is explicit and idempotent; dropping without stopping is
/// handled but is not the intended path.
#[derive(Debug)]
pub struct ListenerPlaybackRuntime {
    shared: Arc<Shared>,
    clock: Arc<PumpClock>,
    token: u64,
    pump_thread: Mutex<Option<JoinHandle<()>>>,
    /// Diagnostics captured at stop, so a stream's final accounting survives
    /// the teardown that produced it.
    last_diagnostics: Mutex<Option<PlaybackDiagnostics>>,
}

impl ListenerPlaybackRuntime {
    /// Starts a runtime for one stream: registers a fresh render ring, builds
    /// the scheduler and pump, and spawns the pump thread.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::InvalidConfiguration`] when the ring,
    /// scheduler, or pump configuration is rejected, or
    /// [`ListenerPlaybackError::PumpThread`] when the pump thread cannot be
    /// spawned. A failure here leaves nothing registered or running.
    pub fn start(
        scheduler_config: SchedulerConfig,
        ring_config: RenderRingConfig,
        pump_config: PlaybackPumpConfig,
        initial_offset_ms: f64,
    ) -> Result<Self, ListenerPlaybackError> {
        let scheduler = PlaybackScheduler::new(scheduler_config, initial_offset_ms)
            .map_err(|error| ListenerPlaybackError::InvalidConfiguration(error.message))?;
        let (producer, token) = register_render_ring(ring_config)?;

        let pump = match PlaybackPump::new(scheduler, producer, pump_config) {
            Ok(pump) => pump,
            Err(error) => {
                // Nothing may stay registered behind a failed start.
                let _ = release_render_ring(token);
                return Err(ListenerPlaybackError::InvalidConfiguration(error.message));
            }
        };

        let estimator = match ClockSyncEstimator::new(SyncEstimatorConfig::default()) {
            Ok(estimator) => estimator,
            Err(error) => {
                let _ = release_render_ring(token);
                return Err(ListenerPlaybackError::InvalidConfiguration(format!(
                    "{error:?}"
                )));
            }
        };
        let shared = Arc::new(Shared {
            pump: Mutex::new(pump),
            estimator: Mutex::new(estimator),
            running: AtomicBool::new(true),
        });
        let clock = Arc::new(PumpClock::new());
        let thread_shared = Arc::clone(&shared);
        let thread_clock = Arc::clone(&clock);
        let pump_thread = thread::Builder::new()
            .name("silent-disco-playback".to_owned())
            .spawn(move || run_pump(&thread_shared, &thread_clock))
            .map_err(|error| {
                shared.running.store(false, Ordering::SeqCst);
                let _ = release_render_ring(token);
                ListenerPlaybackError::PumpThread(format!("pump thread failed to start: {error}"))
            })?;

        Ok(Self {
            shared,
            clock,
            token,
            pump_thread: Mutex::new(Some(pump_thread)),
            last_diagnostics: Mutex::new(None),
        })
    }

    /// Diagnostics as they stood when [`Self::stop`] drained the stream, or
    /// `None` while it is still running.
    #[must_use]
    pub fn final_diagnostics(&self) -> Option<PlaybackDiagnostics> {
        *self
            .last_diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The opaque token to hand to native audio setup exactly once, at stream
    /// start; never dereferenced by Rust (see `include/silent_disco_audio.h`).
    #[must_use]
    pub const fn engine_token(&self) -> u64 {
        self.token
    }

    /// Submits one arriving packet for ordering, concealment, and scheduling.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::Stopped`] once the runtime has been
    /// stopped.
    pub fn submit_packet(
        &self,
        datagram: silent_disco_core::protocol::AudioDatagram,
    ) -> Result<(), ListenerPlaybackError> {
        self.ensure_running()?;
        // A rejected packet is ordinary, expected traffic (a duplicate, a late
        // arrival, an out-of-window reorder) that the jitter buffer counts;
        // it is not a runtime failure. Routed through the pump rather than
        // straight to the scheduler so packets arriving before sync locks are
        // dropped instead of overflowing the reorder window.
        let _ = self.shared.lock_pump().submit_packet(datagram);
        Ok(())
    }

    /// Re-anchors this runtime's presentation-time base to
    /// `host_start_time_ms`, matching a host that re-broadcast `StreamStart`
    /// with an updated anchor after resuming from a pause (see
    /// [`silent_disco_core::audio::PlaybackScheduler::set_host_start_time_ms`]).
    /// Applies to the live scheduler in place -- no ring reset, no pump
    /// restart, no audible discontinuity beyond whatever the pause itself
    /// already caused.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::Stopped`] once the runtime has been
    /// stopped.
    pub fn reanchor_presentation_time(
        &self,
        host_start_time_ms: u64,
    ) -> Result<(), ListenerPlaybackError> {
        self.ensure_running()?;
        self.shared
            .lock_pump()
            .scheduler_mut()
            .set_host_start_time_ms(host_start_time_ms);
        Ok(())
    }

    /// Captures every frame this stream releases toward the ring to a WAV at
    /// `path`, for offline comparison against the diagnostics counters.
    ///
    /// Diagnostic instrumentation, off unless enabled. Enable it before
    /// playback starts; frames already released are not retroactively
    /// captured.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::DebugCapture`] when the file cannot be
    /// created, and [`ListenerPlaybackError::Stopped`] once the runtime has
    /// been stopped.
    pub fn start_debug_capture(&self, path: &str) -> Result<(), ListenerPlaybackError> {
        self.ensure_running()?;
        let mut pump = self.shared.lock_pump();
        let sample_rate = pump.sample_rate();
        let recorder = DebugPcmRecorder::create(path, sample_rate, 2)
            .map_err(|error| ListenerPlaybackError::DebugCapture(error.to_string()))?;
        pump.set_recorder(recorder);
        Ok(())
    }

    /// First debug-capture failure, if capture stopped early. A capture that
    /// silently truncated would make the recording it produced misleading.
    #[must_use]
    pub fn debug_capture_error(&self) -> Option<String> {
        self.shared
            .lock_pump()
            .recorder_error()
            .map(ToOwned::to_owned)
    }

    /// Everything needed to tell where audio went missing or wrong, without
    /// relying on a description of what it sounded like.
    #[must_use]
    pub fn diagnostics(&self) -> PlaybackDiagnostics {
        self.shared.lock_pump().diagnostics()
    }

    /// Monotonic milliseconds on the timeline this runtime schedules against.
    ///
    /// Every local timestamp handed to [`Self::begin_sync_probe`] and
    /// [`Self::observe_sync_response`] must come from here, so the sync
    /// estimate and the playback clock share one timeline.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Registers one outbound sync probe before it is sent.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::Sync`] for a duplicate correlation ID
    /// or when too many probes are already outstanding, and
    /// [`ListenerPlaybackError::Stopped`] once the runtime has been stopped.
    pub fn begin_sync_probe(
        &self,
        correlation_id: u64,
        local_send_time_ms: u64,
    ) -> Result<(), ListenerPlaybackError> {
        self.ensure_running()?;
        self.shared
            .lock_estimator()
            .begin_probe(
                SyncCorrelationId::new(correlation_id),
                LocalMonotonicMillis::new(local_send_time_ms),
            )
            .map_err(|error| ListenerPlaybackError::Sync(format!("{error:?}")))
    }

    /// Feeds one correlated four-timestamp sync response to the estimator and,
    /// when the sample is accepted, applies the resulting offset to playback.
    ///
    /// A rejected sample (its round-trip time outside the acceptance window)
    /// changes nothing: it must never reach the playback timeline, and it must
    /// never contribute to the skew estimate either.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::Sync`] when the response does not
    /// correlate to a registered probe or its timestamps are impossible, and
    /// [`ListenerPlaybackError::Stopped`] once the runtime has been stopped.
    pub fn observe_sync_response(
        &self,
        correlation_id: u64,
        echoed_local_send_time_ms: u64,
        host_receive_time_ms: u64,
        host_send_time_ms: u64,
        local_receive_time_ms: u64,
    ) -> Result<SyncSampleOutcome, ListenerPlaybackError> {
        self.ensure_running()?;
        let observation = self
            .shared
            .lock_estimator()
            .observe_response(
                SyncCorrelationId::new(correlation_id),
                LocalMonotonicMillis::new(echoed_local_send_time_ms),
                HostMonotonicMillis::new(host_receive_time_ms),
                HostMonotonicMillis::new(host_send_time_ms),
                LocalMonotonicMillis::new(local_receive_time_ms),
            )
            .map_err(|error| ListenerPlaybackError::Sync(format!("{error:?}")))?;

        let snapshot = observation.snapshot;
        let mut sync_locked = self.shared.lock_pump().is_sync_locked();
        if observation.accepted {
            let mut pump = self.shared.lock_pump();
            pump.apply_sync_offset(snapshot.offset_ms);
            sync_locked = pump.is_sync_locked();
        }
        Ok(SyncSampleOutcome {
            accepted: observation.accepted,
            offset_ms: snapshot.offset_ms,
            skew_ppm: snapshot.skew_ppm,
            round_trip_time_ms: snapshot.round_trip_time_ms,
            jitter_ms: snapshot.jitter_ms,
            confidence: snapshot.confidence,
            accepted_sample_count: snapshot.accepted_sample_count,
            sync_locked,
        })
    }

    fn ensure_running(&self) -> Result<(), ListenerPlaybackError> {
        if self.shared.running.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ListenerPlaybackError::Stopped(
                "listener playback runtime is stopped".to_owned(),
            ))
        }
    }

    /// Stops playback: drains whatever is still buffered into the ring so the
    /// stream's tail is not truncated, joins the pump thread, and releases the
    /// ring registration. Idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`ListenerPlaybackError::PumpThread`] when the pump thread
    /// ended by panicking. The ring is released either way; the failure is
    /// reported rather than swallowed.
    pub fn stop(&self) -> Result<(), ListenerPlaybackError> {
        if !self.shared.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        // Drain before joining so the tail is queued while the ring is still
        // live and the consumer is still reading.
        {
            let mut pump = self.shared.lock_pump();
            pump.finish();
        }
        // Then wait for the consumer to actually play what is queued. Stopping
        // here without waiting discards a full ring cushion -- roughly 400ms
        // at the configured depth, measured at 397ms on a device -- which is
        // audible as the stream ending abruptly, and which the debug capture
        // cannot show because it records frames on their way *into* the ring.
        self.await_ring_drain();
        let summary = {
            let pump = self.shared.lock_pump();
            pump.diagnostics()
        };
        *self
            .last_diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(summary);

        let join_result = self
            .pump_thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .map(JoinHandle::join)
            .transpose()
            .map_err(|_| {
                ListenerPlaybackError::PumpThread(
                    "playback pump thread ended by panicking".to_owned(),
                )
            });

        let release_result = release_render_ring(self.token).map_err(ListenerPlaybackError::from);
        join_result?;
        release_result
    }
}

impl ListenerPlaybackRuntime {
    /// Waits for the real-time consumer to play out whatever is still queued
    /// in the render ring, so a stopping stream ends on its own final sample
    /// rather than mid-note.
    ///
    /// Bounded two ways. The overall deadline caps the wait when the consumer
    /// is not running at all -- an output that failed to open, or was closed
    /// first -- since the ring would then never drain and stopping would hang.
    /// The stall bound ends it sooner in the same situation, without waiting
    /// out the full deadline in the common case. Either way the ring's
    /// remaining depth stays visible in the final diagnostics rather than
    /// being silently discarded.
    fn await_ring_drain(&self) {
        let deadline = Instant::now() + RING_DRAIN_TIMEOUT;
        let mut last_queued = usize::MAX;
        let mut stalled_for = Duration::ZERO;
        loop {
            let queued = {
                let mut pump = self.shared.lock_pump();
                // Anything the ring had no room for earlier still has to go in
                // as it drains, or the tail would be truncated a second way.
                drain_due_frames(&mut pump, self.clock.now_ms());
                pump.diagnostics().ring_queued_frames
            };
            if queued == 0 {
                return;
            }
            if queued < last_queued {
                stalled_for = Duration::ZERO;
            } else {
                stalled_for += RING_DRAIN_POLL_INTERVAL;
                if stalled_for >= RING_DRAIN_STALL_LIMIT {
                    return;
                }
            }
            last_queued = queued;
            if Instant::now() >= deadline {
                return;
            }
            thread::sleep(RING_DRAIN_POLL_INTERVAL);
        }
    }
}

impl Drop for ListenerPlaybackRuntime {
    fn drop(&mut self) {
        // Dropping without an explicit stop still has to release the ring and
        // reap the thread; a failure here cannot be returned, so it is not
        // silently discarded either — `stop` is the supported path precisely
        // so failures stay reportable.
        if self.shared.running.load(Ordering::SeqCst) {
            let _ = self.stop();
        }
    }
}

/// Pump thread body: advance playback until the runtime is stopped.
///
/// Each wake-up drains every frame that is currently due rather than exactly
/// one. A single frame per tick caps throughput at one packet per
/// [`PUMP_TICK_INTERVAL`] — 100 packets/second — which silently becomes a
/// hard ceiling below real time as soon as a packet carries less than 10ms of
/// audio. At the 5ms packet duration that ceiling is half real time: measured
/// on a device, the pump emitted 60-95 packets/second against the 200 the
/// stream required, the render ring sat at zero for 37 of 37 playing seconds,
/// and the jitter buffer backed up until arrivals fell outside the reorder
/// window entirely.
///
/// The pump stops on its own once it reaches its write lead or ring depth
/// cap, so this loop is bounded by that in practice; [`MAX_FRAMES_PER_TICK`]
/// only bounds the pathological case, and reaching it simply defers the
/// remainder to the next wake-up.
fn run_pump(shared: &Arc<Shared>, clock: &Arc<PumpClock>) {
    while shared.running.load(Ordering::SeqCst) {
        {
            let mut pump = shared.lock_pump();
            drain_due_frames(&mut pump, clock.now_ms());
        }
        thread::sleep(PUMP_TICK_INTERVAL);
    }
}

/// Advances `pump` until nothing further is due, returning how many frames
/// moved toward the ring.
///
/// One [`PlaybackPump::tick`] releases at most one frame, so draining exactly
/// one per wake-up caps throughput at one packet per [`PUMP_TICK_INTERVAL`].
/// That ceiling is invisible while packets are long and becomes a hard limit
/// below real time the moment they carry less audio than the tick interval.
fn drain_due_frames(pump: &mut PlaybackPump, now_ms: u64) -> usize {
    let mut released = 0;
    for _ in 0..MAX_FRAMES_PER_TICK {
        match pump.tick(now_ms) {
            // Productive: a frame moved toward the ring, so another may
            // already be due within this same wake-up.
            PumpTick::Queued { .. } | PumpTick::FlushedPending { .. } => released += 1,
            // Everything else means nothing more can happen until time passes,
            // the ring drains, or the stream's state changes.
            _ => break,
        }
    }
    released
}

#[cfg(test)]
mod tests {
    // A skew of exactly zero is the documented result of a single accepted
    // sample (a regression needs several), so exact equality is the correct
    // assertion here, not a tolerance.
    #![allow(clippy::float_cmp)]

    use super::{
        ListenerPlaybackError, ListenerPlaybackRuntime, PlaybackPump, PlaybackScheduler,
        drain_due_frames,
    };
    use crate::audio_abi::{
        register_render_ring, registry_test_guard, release_render_ring,
        silent_disco_audio_read_interleaved_f32,
    };
    use silent_disco_core::audio::SchedulerConfig;
    use silent_disco_core::audio::{PlaybackPhase, PlaybackPumpConfig, RenderRingConfig};
    use silent_disco_core::domain::{
        MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
    };
    use silent_disco_core::protocol::{AudioCodec, AudioDatagram};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    const PACKET_DURATION_MS: u32 = 20;
    const SAMPLES_PER_PACKET: u32 = 960;

    fn scheduler_config() -> SchedulerConfig {
        let mut config = SchedulerConfig::new(
            SessionId::new("session-runtime").expect("session id"),
            StreamId::new("stream-runtime").expect("stream id"),
            PACKET_DURATION_MS,
            0,
            SAMPLES_PER_PACKET,
            2,
        );
        config.startup_buffer_target_ms = 0;
        config
    }

    fn ring_config() -> RenderRingConfig {
        RenderRingConfig {
            capacity_frames: 48_000,
            target_fill_frames: 19_200,
        }
    }

    fn datagram(sequence: u64) -> AudioDatagram {
        AudioDatagram {
            session_id: SessionId::new("session-runtime").expect("session id"),
            stream_id: StreamId::new("stream-runtime").expect("stream id"),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: 48_000,
            channels: 2,
            samples_per_packet: SAMPLES_PER_PACKET,
            first_sample_index: SampleIndex::new(sequence * u64::from(SAMPLES_PER_PACKET)),
            host_presentation_time_ms: MonotonicMillis::new(
                sequence * u64::from(PACKET_DURATION_MS),
            ),
            payload: (0..SAMPLES_PER_PACKET * 2)
                .flat_map(|_| 16_384_i16.to_le_bytes())
                .collect(),
        }
    }

    /// Real-time throughput must not depend on how the stream is packetized.
    ///
    /// One `PlaybackPump::tick` releases at most one frame, so a pump thread
    /// that ticked once per 10ms wake-up could never exceed 100 packets per
    /// second. At 20ms packets that is 2x real time and invisible; at the 5ms
    /// production packet duration it is *half* real time, and a device run
    /// showed exactly that — 60-95 packets/second emitted against the 200 the
    /// stream needed, the ring at zero for every playing second, and the
    /// jitter buffer backing up until arrivals fell outside the reorder
    /// window. One wake-up therefore has to drain everything already due.
    #[test]
    fn one_pump_wake_up_drains_every_frame_already_due() {
        const SHORT_PACKET_MS: u32 = 5;
        const SHORT_SAMPLES: u32 = 240;
        const DUE_PACKETS: u64 = 40;

        let _guard = registry_test_guard();
        let mut config = SchedulerConfig::new(
            SessionId::new("session-drain").expect("session id"),
            StreamId::new("stream-drain").expect("stream id"),
            SHORT_PACKET_MS,
            0,
            SHORT_SAMPLES,
            2,
        );
        config.startup_buffer_target_ms = 0;
        let scheduler = PlaybackScheduler::new(config, 0.0).expect("valid scheduler");
        let (producer, token) = register_render_ring(ring_config()).expect("ring registered");
        let mut pump = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default())
            .expect("valid pump");
        // Nothing may play against a placeholder offset, so lock sync first.
        pump.apply_sync_offset(0.0);

        for sequence in 0..DUE_PACKETS {
            pump.scheduler_mut()
                .submit_packet(AudioDatagram {
                    session_id: SessionId::new("session-drain").expect("session id"),
                    stream_id: StreamId::new("stream-drain").expect("stream id"),
                    sequence: PacketSequence::new(sequence),
                    codec: AudioCodec::PcmS16Le,
                    sample_rate: 48_000,
                    channels: 2,
                    samples_per_packet: SHORT_SAMPLES,
                    first_sample_index: SampleIndex::new(sequence * u64::from(SHORT_SAMPLES)),
                    host_presentation_time_ms: MonotonicMillis::new(
                        sequence * u64::from(SHORT_PACKET_MS),
                    ),
                    payload: (0..SHORT_SAMPLES * 2)
                        .flat_map(|_| 16_384_i16.to_le_bytes())
                        .collect(),
                })
                .expect("accepted");
        }

        // Every packet above is due at this instant.
        let released = drain_due_frames(&mut pump, DUE_PACKETS * u64::from(SHORT_PACKET_MS));

        assert!(
            released >= usize::try_from(DUE_PACKETS).expect("fits"),
            "one wake-up released only {released} of {DUE_PACKETS} due frames, which caps \
             throughput below real time at short packet durations"
        );
        assert_eq!(pump.diagnostics().packets_emitted, DUE_PACKETS);
        release_render_ring(token).expect("ring released");
    }

    /// Locks sync on a near-zero offset so the helper datagrams, whose
    /// presentation times start at zero, are immediately due.
    fn lock_sync_at_zero_offset(runtime: &ListenerPlaybackRuntime) {
        let send = runtime.now_ms();
        runtime.begin_sync_probe(1, send).expect("probe registered");
        let receive = runtime.now_ms();
        let outcome = runtime
            .observe_sync_response(1, send, send, send, receive)
            .expect("a correlated response is not an error");
        assert!(outcome.sync_locked, "playback cannot start without sync");
    }

    fn token_as_engine(token: u64) -> *mut core::ffi::c_void {
        usize::try_from(token).unwrap_or(usize::MAX) as *mut core::ffi::c_void
    }

    #[test]
    fn start_registers_a_readable_engine_token_and_stop_releases_it() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        let token = runtime.engine_token();
        assert!(token > 0);

        // While running, the token resolves to a live engine.
        let mut output = [0.0_f32; 2];
        let mut frames_from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );
        assert!(
            status == 0 || status == 1,
            "expected OK or PARTIAL, got {status}"
        );

        runtime.stop().expect("stop succeeds");

        // After stopping, the same token must report stopping rather than
        // being silently reused or treated as unknown.
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, 2 /* STOPPING */);
    }

    /// Stopping must let the render ring play out, not discard it.
    ///
    /// The ring holds the write-ahead cushion by design — roughly 400ms, and
    /// measured at 397ms still queued on a device at stop. Closing the output
    /// with that queued throws away the end of the stream, which is audible as
    /// an abrupt cut-off and is invisible to the debug capture, since that
    /// records frames on their way *into* the ring.
    #[test]
    fn stopping_plays_out_what_is_still_queued_in_the_ring() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        let token = runtime.engine_token();
        lock_sync_at_zero_offset(&runtime);
        for sequence in 0..8 {
            runtime.submit_packet(datagram(sequence)).expect("accepted");
        }
        // Let the pump fill the ring before stopping.
        let filled = Instant::now() + Duration::from_millis(500);
        while runtime.diagnostics().ring_queued_frames == 0 && Instant::now() < filled {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            runtime.diagnostics().ring_queued_frames > 0,
            "nothing was queued, so this would pass without draining anything"
        );

        // A consumer standing in for the real-time audio callback.
        let consuming = Arc::new(AtomicBool::new(true));
        let consumer_flag = Arc::clone(&consuming);
        let consumer = thread::spawn(move || {
            let mut output = [0.0_f32; 512];
            while consumer_flag.load(Ordering::SeqCst) {
                let mut frames_read = 0_u32;
                let _ = silent_disco_audio_read_interleaved_f32(
                    token_as_engine(token),
                    output.as_mut_ptr(),
                    256,
                    2,
                    &raw mut frames_read,
                );
                thread::sleep(Duration::from_millis(2));
            }
        });

        runtime.stop().expect("stop succeeds");
        consuming.store(false, Ordering::SeqCst);
        consumer.join().expect("consumer thread ends");

        let final_diagnostics = runtime
            .final_diagnostics()
            .expect("diagnostics captured at stop");
        assert_eq!(
            final_diagnostics.ring_queued_frames, 0,
            "stop abandoned {} frames still queued in the ring",
            final_diagnostics.ring_queued_frames
        );
    }

    /// The drain must not turn into a hang when nothing is consuming — an
    /// output that failed to open, or one already closed.
    #[test]
    fn stopping_without_a_consumer_gives_up_instead_of_waiting_forever() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        lock_sync_at_zero_offset(&runtime);
        for sequence in 0..8 {
            runtime.submit_packet(datagram(sequence)).expect("accepted");
        }
        let filled = Instant::now() + Duration::from_millis(500);
        while runtime.diagnostics().ring_queued_frames == 0 && Instant::now() < filled {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(runtime.diagnostics().ring_queued_frames > 0);

        let started = Instant::now();
        runtime.stop().expect("stop succeeds");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "stop took {elapsed:?} with no consumer draining the ring"
        );
    }

    #[test]
    fn stop_is_idempotent() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");

        runtime.stop().expect("first stop succeeds");
        runtime
            .stop()
            .expect("a repeated stop is a no-op, not an error");
    }

    #[test]
    fn submitting_after_stop_is_an_explicit_failure_not_a_silent_no_op() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        runtime
            .submit_packet(datagram(0))
            .expect("accepted while running");
        runtime.stop().expect("stop succeeds");

        let error = runtime
            .submit_packet(datagram(1))
            .expect_err("a stopped runtime must reject further packets");
        assert!(matches!(error, ListenerPlaybackError::Stopped(_)));
    }

    #[test]
    fn an_accepted_sync_sample_unlocks_playback_and_a_rejected_one_does_not() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");

        // A sample whose round trip is far outside the acceptance window must
        // not reach the playback timeline -- nor the skew estimate, which is
        // exactly how a placeholder offset once poisoned it.
        runtime.begin_sync_probe(1, 0).expect("probe registered");
        let rejected = runtime
            .observe_sync_response(1, 0, 500_000, 500_001, 4_000)
            .expect("a correlated response is not an error");
        assert!(!rejected.accepted);
        assert!(!rejected.sync_locked);

        // A low-latency sample is accepted and unlocks playback.
        runtime
            .begin_sync_probe(2, 10_000)
            .expect("probe registered");
        let accepted = runtime
            .observe_sync_response(2, 10_000, 500_000, 500_002, 10_020)
            .expect("a correlated response is not an error");
        assert!(accepted.accepted);
        assert!(accepted.sync_locked);
        assert_eq!(accepted.accepted_sample_count, 1);
        // A single sample cannot support a skew regression yet.
        assert_eq!(accepted.skew_ppm, 0.0);

        runtime.stop().expect("stop succeeds");
    }

    #[test]
    fn an_uncorrelated_or_duplicate_sync_exchange_fails_explicitly() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");

        let unknown = runtime
            .observe_sync_response(99, 0, 1, 2, 3)
            .expect_err("a response with no registered probe must fail");
        assert!(matches!(unknown, ListenerPlaybackError::Sync(_)));

        runtime.begin_sync_probe(1, 0).expect("probe registered");
        let duplicate = runtime
            .begin_sync_probe(1, 0)
            .expect_err("a duplicate correlation id must fail");
        assert!(matches!(duplicate, ListenerPlaybackError::Sync(_)));

        runtime.stop().expect("stop succeeds");
    }

    #[test]
    fn sync_calls_after_stop_fail_explicitly() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        runtime.stop().expect("stop succeeds");

        assert!(matches!(
            runtime.begin_sync_probe(1, 0),
            Err(ListenerPlaybackError::Stopped(_))
        ));
        assert!(matches!(
            runtime.observe_sync_response(1, 0, 1, 2, 3),
            Err(ListenerPlaybackError::Stopped(_))
        ));
    }

    #[test]
    fn final_diagnostics_survive_the_teardown_that_produced_them() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");

        // Nothing is captured while the stream is still live.
        assert!(runtime.final_diagnostics().is_none());
        assert!(!runtime.diagnostics().sync_locked);

        // Packets are only accepted once a clock offset exists; before that
        // they are dropped rather than stranding the buffer.
        for sequence in 0..5 {
            runtime.submit_packet(datagram(sequence)).expect("no error");
        }
        assert_eq!(runtime.diagnostics().dropped_before_sync, 5);

        runtime.begin_sync_probe(1, 0).expect("probe registered");
        runtime
            .observe_sync_response(1, 0, 500_000, 500_001, 20)
            .expect("correlated response");
        for sequence in 0..5 {
            runtime.submit_packet(datagram(sequence)).expect("accepted");
        }
        assert_eq!(runtime.diagnostics().packets_accepted, 5);

        runtime.stop().expect("stop succeeds");

        // A stream's final accounting must outlive the teardown, or the one
        // moment worth reporting is the one moment it cannot be read.
        let summary = runtime
            .final_diagnostics()
            .expect("stopping captures a final summary");
        assert_eq!(summary.packets_accepted, 5);
        assert_eq!(summary.phase, PlaybackPhase::Stopped);
    }

    #[test]
    fn debug_capture_writes_a_playable_wav_for_the_streams_audio() {
        let _guard = registry_test_guard();
        let directory = std::env::temp_dir().join(format!(
            "silent-disco-runtime-capture-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temp directory");
        let path = directory.join("runtime-capture.wav");

        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");
        runtime
            .start_debug_capture(path.to_str().expect("utf-8 path"))
            .expect("capture starts");

        // Lock sync and feed audio so the pump actually releases frames.
        runtime.begin_sync_probe(1, 0).expect("probe registered");
        runtime
            .observe_sync_response(1, 0, 500_000, 500_001, 20)
            .expect("correlated response");
        for sequence in 0..5 {
            runtime.submit_packet(datagram(sequence)).expect("accepted");
        }
        runtime.stop().expect("stop succeeds");

        assert!(runtime.debug_capture_error().is_none());
        let bytes = std::fs::read(&path).expect("capture readable");
        assert_eq!(&bytes[0..4], b"RIFF");
        // The tail drained at stop is captured, so the file holds real audio
        // rather than just a header.
        assert!(
            bytes.len() > 44,
            "expected captured samples, got {} bytes",
            bytes.len()
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn debug_capture_fails_explicitly_when_the_path_cannot_be_written() {
        let _guard = registry_test_guard();
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");

        let error = runtime
            .start_debug_capture("/nonexistent-directory/capture.wav")
            .expect_err("an unwritable path must fail rather than silently not record");
        assert!(matches!(error, ListenerPlaybackError::DebugCapture(_)));

        runtime.stop().expect("stop succeeds");
    }

    #[test]
    fn a_rejected_ring_configuration_leaves_nothing_registered() {
        let _guard = registry_test_guard();
        let error = ListenerPlaybackRuntime::start(
            scheduler_config(),
            RenderRingConfig {
                capacity_frames: 1,
                target_fill_frames: 1,
            },
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect_err("an undersized ring must be rejected");

        assert!(matches!(
            error,
            ListenerPlaybackError::InvalidConfiguration(_)
        ));
    }

    #[test]
    fn a_rejected_pump_configuration_releases_the_ring_it_had_already_registered() {
        let _guard = registry_test_guard();
        let error = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig {
                volume: 2.0,
                ..PlaybackPumpConfig::default()
            },
            0.0,
        )
        .expect_err("an out-of-range volume must be rejected");

        assert!(matches!(
            error,
            ListenerPlaybackError::InvalidConfiguration(_)
        ));
        // A partially-completed start must not leave a registered engine
        // behind; the next start gets a fresh token that reads normally.
        let runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("a later start still succeeds");
        runtime.stop().expect("stop succeeds");
    }

    #[test]
    fn dropping_without_stopping_still_releases_the_token() {
        let _guard = registry_test_guard();
        let token = {
            let runtime = ListenerPlaybackRuntime::start(
                scheduler_config(),
                ring_config(),
                PlaybackPumpConfig::default(),
                0.0,
            )
            .expect("runtime starts");
            runtime.engine_token()
        };

        let mut output = [0.0_f32; 2];
        let mut frames_from_ring = 0_u32;
        let status = silent_disco_audio_read_interleaved_f32(
            token_as_engine(token),
            output.as_mut_ptr(),
            1,
            2,
            &raw mut frames_from_ring,
        );
        assert_eq!(status, 2 /* STOPPING */);
    }

    /// Block 24: repeatedly races `stop()` against a simulated Oboe callback
    /// thread (continuously reading the engine token) and a simulated
    /// network-arrival thread (continuously submitting packets), across many
    /// fresh runtimes. This is the scenario the architecture spec calls out
    /// by name: "stop and join the stream before releasing the Rust engine
    /// token" -- if that ordering were ever violated, the callback thread
    /// would read a released or reused token while still "active" from the
    /// platform's point of view.
    ///
    /// Every iteration must: never panic on any thread, have `stop()` return
    /// in bounded time regardless of how much concurrent traffic is in
    /// flight, and leave the token reporting `Stopping` (never
    /// `InvalidState`) once both threads have been joined.
    #[test]
    fn stop_races_repeatedly_against_a_simulated_audio_callback_and_packet_arrivals() {
        let _guard = registry_test_guard();
        for iteration in 0_u64..20 {
            let runtime = Arc::new(
                ListenerPlaybackRuntime::start(
                    scheduler_config(),
                    ring_config(),
                    PlaybackPumpConfig::default(),
                    0.0,
                )
                .expect("runtime starts"),
            );
            lock_sync_at_zero_offset(&runtime);
            let token = runtime.engine_token();

            let callback_running = Arc::new(AtomicBool::new(true));
            let callback_flag = Arc::clone(&callback_running);
            let callback_thread = thread::spawn(move || {
                let mut output = [0.0_f32; 512];
                while callback_flag.load(Ordering::SeqCst) {
                    let mut frames_read = 0_u32;
                    let status = silent_disco_audio_read_interleaved_f32(
                        token_as_engine(token),
                        output.as_mut_ptr(),
                        256,
                        2,
                        &raw mut frames_read,
                    );
                    assert_ne!(
                        status, -3, /* PANIC_CONTAINED */
                        "the real-time read path must never panic under a stop race"
                    );
                }
            });

            let arrivals_running = Arc::new(AtomicBool::new(true));
            let arrivals_flag = Arc::clone(&arrivals_running);
            let arrivals_runtime = Arc::clone(&runtime);
            let arrivals_thread = thread::spawn(move || {
                let mut sequence = iteration * 10_000;
                while arrivals_flag.load(Ordering::SeqCst) {
                    // Both outcomes (accepted, or rejected because the
                    // runtime already stopped) are legitimate; only a panic
                    // or hang here would be a bug.
                    let _ = arrivals_runtime.submit_packet(datagram(sequence));
                    sequence += 1;
                }
            });

            // Let both threads generate real concurrent load before racing
            // the stop itself.
            thread::sleep(Duration::from_millis(5));
            let stop_started = Instant::now();
            runtime
                .stop()
                .expect("stop succeeds even under concurrent load");
            let stop_elapsed = stop_started.elapsed();
            assert!(
                stop_elapsed < Duration::from_secs(3),
                "stop took {stop_elapsed:?} while racing a simulated audio callback"
            );

            arrivals_running.store(false, Ordering::SeqCst);
            arrivals_thread
                .join()
                .expect("arrivals thread must not panic");
            callback_running.store(false, Ordering::SeqCst);
            callback_thread
                .join()
                .expect("callback thread must not panic");

            let mut output = [0.0_f32; 2];
            let mut frames_from_ring = 0_u32;
            let status_after = silent_disco_audio_read_interleaved_f32(
                token_as_engine(token),
                output.as_mut_ptr(),
                1,
                2,
                &raw mut frames_from_ring,
            );
            assert_eq!(status_after, 2 /* STOPPING */);
        }
    }
}

/// Everything needed to start one listener playback stream, flattened for the
/// foreign binding.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FfiListenerPlaybackConfig {
    /// Session this stream belongs to.
    pub session_id: String,
    /// Stream identity; a new stream generation requires a new runtime.
    pub stream_id: String,
    /// Wire packet duration, matching the host packetizer.
    pub packet_duration_ms: u32,
    /// Host monotonic time at which sequence zero's slot began.
    pub host_start_time_ms: u64,
    /// Samples per channel per packet.
    pub samples_per_packet: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// Presentation buffer accumulated before playback first starts.
    pub startup_buffer_target_ms: u64,
    /// Presentation buffer rebuilt before playback resumes after a
    /// mid-stream rebuffer. Deliberately separate from the startup target:
    /// this one is the length of an audible hole.
    pub rebuffer_target_ms: u64,
    /// Render ring capacity, in frames.
    pub ring_capacity_frames: u32,
    /// Ring depth the pump holds as its jitter cushion, in frames.
    pub ring_target_fill_frames: u32,
    /// How far ahead of its deadline each frame is written.
    pub write_lead_ms: u64,
    /// Ceiling on the stream-start alignment prefill.
    pub max_prefill_ms: u64,
    /// Linear output gain, in `0.0..=1.0`.
    pub volume: f32,
}

/// One arriving audio packet, flattened for the foreign binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiAudioPacket {
    /// Wire sequence number.
    pub sequence: u64,
    /// Sample rate the packet was encoded at.
    pub sample_rate: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// Samples per channel in this packet.
    pub samples_per_packet: u32,
    /// First interleaved sample index this packet covers.
    pub first_sample_index: u64,
    /// Host monotonic presentation time for this packet.
    pub host_presentation_time_ms: u64,
    /// Interleaved PCM16LE payload.
    pub payload: Vec<u8>,
}

/// What the scheduler is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPlaybackPhase {
    /// Accumulating the startup presentation buffer.
    Buffering,
    /// Delivering frames against the presentation timeline.
    Playing,
    /// Paused, re-accumulating before playback resumes.
    AwaitingRebuffer,
    /// Stopped; no further frames will be produced.
    Stopped,
}

/// Playback accounting, flattened for the foreign binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiPlaybackDiagnostics {
    /// Scheduler phase.
    pub phase: FfiPlaybackPhase,
    /// True once a real clock-offset estimate has been applied.
    pub sync_locked: bool,
    /// Packets accepted into the jitter buffer.
    pub packets_accepted: u64,
    /// Packets emitted in order, including the tail drained at stop.
    pub packets_emitted: u64,
    /// Sequences abandoned without playing: concealed losses plus skipped gaps.
    pub sequences_skipped: u64,
    /// Packets that arrived after their slot had already played.
    pub late_rejections: u64,
    /// Duplicate packets rejected.
    pub duplicate_rejections: u64,
    /// Packets too far ahead to reorder.
    pub reorder_window_rejections: u64,
    /// Times the buffer adopted a far-ahead position after the stream moved
    /// beyond its reorder window.
    pub resynchronisations: u64,
    /// Packets discarded because they arrived before a clock offset existed.
    pub dropped_before_sync: u64,
    /// Frames synthesized to cover missing packets.
    pub concealed_packets: u64,
    /// Times the concealment bound forced a rebuffer.
    pub concealment_driven_rebuffers: u64,
    /// Times a clock-offset jump too large to splice forced a rebuffer.
    pub offset_driven_rebuffers: u64,
    /// Every hard resync regardless of cause -- `concealment_driven_rebuffers`
    /// `+ offset_driven_rebuffers` (A4.4).
    pub hard_resync_signals: u64,
    /// Buffered presentation-time span currently held.
    pub buffered_span_ms: u64,
    /// Frames currently queued in the render ring.
    pub ring_queued_frames: u64,
    /// Largest ring depth observed.
    pub ring_peak_queued_frames: u64,
    /// Frames converted but not yet accepted by the ring.
    pub pending_frames: u64,
    /// Alignment silence queued at stream start.
    pub prefill_frames: u64,
    /// Real-time reads that had to substitute silence.
    pub ring_underruns: u64,
    /// Frames of silence those reads substituted.
    pub ring_silence_filled_frames: u64,
    /// Producer writes that could not fit every frame.
    pub ring_full_events: u64,
}

/// The estimator's confidence in its current estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSyncConfidence {
    /// No sample has been accepted yet.
    Unknown,
    /// Accepted, but the round trip or dispersion is poor.
    Poor,
    /// Usable.
    Fair,
    /// Good.
    Good,
    /// Excellent.
    Excellent,
}

impl From<SyncConfidence> for FfiSyncConfidence {
    fn from(confidence: SyncConfidence) -> Self {
        match confidence {
            SyncConfidence::Unknown => Self::Unknown,
            SyncConfidence::Poor => Self::Poor,
            SyncConfidence::Fair => Self::Fair,
            SyncConfidence::Good => Self::Good,
            SyncConfidence::Excellent => Self::Excellent,
        }
    }
}

impl From<FfiSyncConfidence> for SyncConfidence {
    fn from(confidence: FfiSyncConfidence) -> Self {
        match confidence {
            FfiSyncConfidence::Unknown => Self::Unknown,
            FfiSyncConfidence::Poor => Self::Poor,
            FfiSyncConfidence::Fair => Self::Fair,
            FfiSyncConfidence::Good => Self::Good,
            FfiSyncConfidence::Excellent => Self::Excellent,
        }
    }
}

/// Result of feeding one correlated sync response, flattened for the binding.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct FfiSyncSampleOutcome {
    /// True when the sample updated the estimate.
    pub accepted: bool,
    /// Current offset estimate, in milliseconds.
    pub offset_ms: f64,
    /// Current skew estimate, in parts per million.
    pub skew_ppm: f64,
    /// Round-trip time behind the current estimate.
    pub round_trip_time_ms: f64,
    /// Offset dispersion across the samples behind the current estimate.
    pub jitter_ms: f64,
    /// The estimator's own confidence in the current estimate.
    pub confidence: FfiSyncConfidence,
    /// Accepted samples behind the current estimate.
    pub accepted_sample_count: u64,
    /// True once playback has a real offset and may start.
    pub sync_locked: bool,
}

impl From<PlaybackPhase> for FfiPlaybackPhase {
    fn from(phase: PlaybackPhase) -> Self {
        match phase {
            PlaybackPhase::Buffering => Self::Buffering,
            PlaybackPhase::Playing => Self::Playing,
            PlaybackPhase::AwaitingRebuffer => Self::AwaitingRebuffer,
            PlaybackPhase::Stopped => Self::Stopped,
        }
    }
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

impl From<PlaybackDiagnostics> for FfiPlaybackDiagnostics {
    fn from(diagnostics: PlaybackDiagnostics) -> Self {
        Self {
            phase: diagnostics.phase.into(),
            sync_locked: diagnostics.sync_locked,
            packets_accepted: diagnostics.packets_accepted,
            packets_emitted: diagnostics.packets_emitted,
            sequences_skipped: diagnostics.sequences_skipped,
            late_rejections: diagnostics.late_rejections,
            duplicate_rejections: diagnostics.duplicate_rejections,
            reorder_window_rejections: diagnostics.reorder_window_rejections,
            resynchronisations: diagnostics.resynchronisations,
            dropped_before_sync: diagnostics.dropped_before_sync,
            concealed_packets: diagnostics.concealed_packets,
            concealment_driven_rebuffers: diagnostics.concealment_driven_rebuffers,
            offset_driven_rebuffers: diagnostics.offset_driven_rebuffers,
            hard_resync_signals: diagnostics.hard_resync_signals,
            buffered_span_ms: diagnostics.buffered_span_ms,
            ring_queued_frames: to_u64(diagnostics.ring_queued_frames),
            ring_peak_queued_frames: to_u64(diagnostics.ring_peak_queued_frames),
            pending_frames: to_u64(diagnostics.pending_frames),
            prefill_frames: to_u64(diagnostics.prefill_frames),
            ring_underruns: diagnostics.ring_underruns,
            ring_silence_filled_frames: diagnostics.ring_silence_filled_frames,
            ring_full_events: diagnostics.ring_full_events,
        }
    }
}

impl From<SyncSampleOutcome> for FfiSyncSampleOutcome {
    fn from(outcome: SyncSampleOutcome) -> Self {
        Self {
            accepted: outcome.accepted,
            offset_ms: outcome.offset_ms,
            skew_ppm: outcome.skew_ppm,
            round_trip_time_ms: outcome.round_trip_time_ms,
            jitter_ms: outcome.jitter_ms,
            confidence: outcome.confidence.into(),
            accepted_sample_count: to_u64(outcome.accepted_sample_count),
            sync_locked: outcome.sync_locked,
        }
    }
}

/// Errors surfaced to the foreign binding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiListenerPlaybackError {
    /// The requested configuration was rejected.
    InvalidConfiguration(String),
    /// The handle was already stopped.
    Stopped(String),
    /// The pump thread failed to start or ended abnormally.
    PumpThread(String),
    /// A sync probe or response was rejected.
    Sync(String),
    /// The debug capture could not be started.
    DebugCapture(String),
}

impl fmt::Display for FfiListenerPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Stopped(message)
            | Self::PumpThread(message)
            | Self::Sync(message)
            | Self::DebugCapture(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FfiListenerPlaybackError {}

impl From<ListenerPlaybackError> for FfiListenerPlaybackError {
    fn from(error: ListenerPlaybackError) -> Self {
        match error {
            ListenerPlaybackError::InvalidConfiguration(message) => {
                Self::InvalidConfiguration(message)
            }
            ListenerPlaybackError::Stopped(message) => Self::Stopped(message),
            ListenerPlaybackError::PumpThread(message) => Self::PumpThread(message),
            ListenerPlaybackError::Sync(message) => Self::Sync(message),
            ListenerPlaybackError::DebugCapture(message) => Self::DebugCapture(message),
        }
    }
}

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
            config.packet_duration_ms,
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
        self.runtime.diagnostics().into()
    }

    /// Accounting as it stood when the stream was stopped, if it has been.
    #[must_use]
    pub fn final_diagnostics(&self) -> Option<FfiPlaybackDiagnostics> {
        self.runtime.final_diagnostics().map(Into::into)
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
    pub(crate) fn submit_core_datagram(&self, datagram: AudioDatagram) -> bool {
        self.runtime.submit_packet(datagram).is_ok()
    }
}
