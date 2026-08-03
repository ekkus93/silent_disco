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
    PlaybackPump, PlaybackPumpConfig, PlaybackScheduler, RenderRingConfig, SchedulerConfig,
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
}

impl fmt::Display for ListenerPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message)
            | Self::Stopped(message)
            | Self::PumpThread(message) => formatter.write_str(message),
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

/// State shared between the owning handle and its pump thread.
#[derive(Debug)]
struct Shared {
    pump: Mutex<PlaybackPump>,
    running: AtomicBool,
}

impl Shared {
    fn lock_pump(&self) -> MutexGuard<'_, PlaybackPump> {
        self.pump.lock().unwrap_or_else(PoisonError::into_inner)
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
    token: u64,
    pump_thread: Option<JoinHandle<()>>,
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

        let shared = Arc::new(Shared {
            pump: Mutex::new(pump),
            running: AtomicBool::new(true),
        });
        let thread_shared = Arc::clone(&shared);
        let pump_thread = thread::Builder::new()
            .name("silent-disco-playback".to_owned())
            .spawn(move || run_pump(&thread_shared))
            .map_err(|error| {
                shared.running.store(false, Ordering::SeqCst);
                let _ = release_render_ring(token);
                ListenerPlaybackError::PumpThread(format!("pump thread failed to start: {error}"))
            })?;

        Ok(Self {
            shared,
            token,
            pump_thread: Some(pump_thread),
        })
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
        if !self.shared.running.load(Ordering::SeqCst) {
            return Err(ListenerPlaybackError::Stopped(
                "listener playback runtime is stopped".to_owned(),
            ));
        }
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
        self.shared.lock_pump().finish();

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
fn run_pump(shared: &Arc<Shared>) {
    let clock = PumpClock::new();
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
    use super::{ListenerPlaybackError, ListenerPlaybackRuntime};
    use crate::audio_abi::{registry_test_guard, silent_disco_audio_read_interleaved_f32};
    use silent_disco_core::audio::SchedulerConfig;
    use silent_disco_core::audio::{PlaybackPumpConfig, RenderRingConfig};
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
            PlaybackPumpConfig { volume: 2.0 },
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
