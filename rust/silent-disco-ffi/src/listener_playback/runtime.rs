//! Playback engine lifecycle: the runtime one caller owns per stream, and the
//! outcome type returned from feeding it a correlated sync sample.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use silent_disco_core::audio::{
    DebugPcmRecorder, PlaybackDiagnostics, PlaybackPump, PlaybackPumpConfig, PlaybackScheduler,
    RenderRingConfig, SchedulerConfig,
};
use silent_disco_core::domain::SyncConfidence;
use silent_disco_core::protocol::AudioDatagram;
use silent_disco_core::sync::{
    ClockSyncEstimator, HostMonotonicMillis, LocalMonotonicMillis, SyncCorrelationId,
    SyncEstimatorConfig,
};

use crate::audio_abi::{register_render_ring, release_render_ring};

use super::error::ListenerPlaybackError;
use super::pump::{
    PumpClock, RING_DRAIN_POLL_INTERVAL, RING_DRAIN_STALL_LIMIT, RING_DRAIN_TIMEOUT, Shared,
    drain_due_frames, run_pump,
};

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
    pub fn submit_packet(&self, datagram: AudioDatagram) -> Result<(), ListenerPlaybackError> {
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
