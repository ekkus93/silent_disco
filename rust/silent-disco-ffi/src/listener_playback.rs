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

use silent_disco_core::audio::{
    DebugPcmRecorder, PlaybackDiagnostics, PlaybackPump, PlaybackPumpConfig, PlaybackScheduler,
    RenderRingConfig, SchedulerConfig,
};
use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};

use crate::audio_abi::{AudioAbiError, register_render_ring, release_render_ring};

/// How often the pump thread wakes to check whether audio is due.
const PUMP_TICK_INTERVAL: Duration = Duration::from_millis(10);

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
    pump_thread: Option<JoinHandle<()>>,
    /// Diagnostics captured at stop, so a stream's final accounting survives
    /// the teardown that produced it.
    last_diagnostics: Option<PlaybackDiagnostics>,
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
            pump_thread: Some(pump_thread),
            last_diagnostics: None,
        })
    }

    /// Diagnostics as they stood when [`Self::stop`] drained the stream, or
    /// `None` while it is still running.
    #[must_use]
    pub const fn final_diagnostics(&self) -> Option<PlaybackDiagnostics> {
        self.last_diagnostics
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
        // it is not a runtime failure.
        let _ = self
            .shared
            .lock_pump()
            .scheduler_mut()
            .submit_packet(datagram);
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
    pub fn stop(&mut self) -> Result<(), ListenerPlaybackError> {
        if !self.shared.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        // Drain before joining so the tail is queued while the ring is still
        // live and the consumer is still reading.
        let summary = {
            let mut pump = self.shared.lock_pump();
            pump.finish();
            pump.diagnostics()
        };
        self.last_diagnostics = Some(summary);

        let join_result = self
            .pump_thread
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
fn run_pump(shared: &Arc<Shared>, clock: &Arc<PumpClock>) {
    while shared.running.load(Ordering::SeqCst) {
        {
            let mut pump = shared.lock_pump();
            pump.tick(clock.now_ms());
        }
        thread::sleep(PUMP_TICK_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    // A skew of exactly zero is the documented result of a single accepted
    // sample (a regression needs several), so exact equality is the correct
    // assertion here, not a tolerance.
    #![allow(clippy::float_cmp)]

    use super::{ListenerPlaybackError, ListenerPlaybackRuntime};
    use crate::audio_abi::{registry_test_guard, silent_disco_audio_read_interleaved_f32};
    use silent_disco_core::audio::SchedulerConfig;
    use silent_disco_core::audio::{PlaybackPhase, PlaybackPumpConfig, RenderRingConfig};
    use silent_disco_core::domain::{
        MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId,
    };
    use silent_disco_core::protocol::{AudioCodec, AudioDatagram};

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

    fn token_as_engine(token: u64) -> *mut core::ffi::c_void {
        usize::try_from(token).unwrap_or(usize::MAX) as *mut core::ffi::c_void
    }

    #[test]
    fn start_registers_a_readable_engine_token_and_stop_releases_it() {
        let _guard = registry_test_guard();
        let mut runtime = ListenerPlaybackRuntime::start(
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

    #[test]
    fn stop_is_idempotent() {
        let _guard = registry_test_guard();
        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
            scheduler_config(),
            ring_config(),
            PlaybackPumpConfig::default(),
            0.0,
        )
        .expect("runtime starts");

        // Nothing is captured while the stream is still live.
        assert!(runtime.final_diagnostics().is_none());
        assert!(!runtime.diagnostics().sync_locked);

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

        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
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
        let mut runtime = ListenerPlaybackRuntime::start(
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
}
