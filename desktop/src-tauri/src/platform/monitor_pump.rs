//! Presentation-time-scheduled pump feeding one desktop host session's local
//! monitor render ring (Block 34), reusing the exact same
//! `PlaybackScheduler`/`PlaybackPump` machinery the shared Rust core already
//! uses for listener playback (`rust/silent-disco-ffi/src/listener_playback.rs`)
//! -- "the same scheduled Rust timeline" the block's acceptance criterion
//! asks for, not a second, ad hoc pacing loop.
//!
//! A desktop monitor differs from a listener in exactly one respect: there
//! is no second device and no clock gap to estimate. The host is pacing its
//! own local decode against its own local clock, so the usual NTP-style
//! sync-sample estimation never applies here -- [`DesktopMonitorPump::start`]
//! locks the pump's clock offset at a fixed `0.0` once, immediately, and
//! never touches it again. Every other property (bounded write-lead,
//! prefill, target-depth pacing, jitter-buffer/concealment machinery) comes
//! along unchanged; it simply never has anything to do, since packets never
//! actually arrive out of order or late relative to the host's own decode.

use silent_disco_core::audio::{
    PlaybackPump, PlaybackPumpConfig, PlaybackPumpConfigError, PlaybackScheduler,
    RenderRingProducer, SchedulerConfig, SchedulerConfigError,
};
use silent_disco_core::protocol::AudioDatagram;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Matches `rust/silent-disco-ffi/src/listener_playback.rs`'s own tick
/// interval -- the same real-time pacing granularity a listener uses,
/// reused rather than independently chosen.
const PUMP_TICK_INTERVAL: Duration = Duration::from_millis(10);
/// Same bound as the listener pump: caps one wake-up's work so a
/// pathologically short packet duration cannot spin this thread forever.
const MAX_FRAMES_PER_TICK: usize = 512;

/// Failure while starting a monitor pump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MonitorPumpStartError {
    Scheduler(SchedulerConfigError),
    Pump(PlaybackPumpConfigError),
    ThreadSpawnFailed(String),
}

impl fmt::Display for MonitorPumpStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheduler(error) => {
                write!(formatter, "invalid monitor scheduler config: {error}")
            }
            Self::Pump(error) => write!(formatter, "invalid monitor pump config: {error}"),
            Self::ThreadSpawnFailed(message) => {
                write!(
                    formatter,
                    "failed to start the monitor pump thread: {message}"
                )
            }
        }
    }
}

impl std::error::Error for MonitorPumpStartError {}

/// One active monitor stream's real-time-paced pump thread.
pub(crate) struct DesktopMonitorPump {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DesktopMonitorPump {
    /// Builds a scheduler+pump for one stream and starts a dedicated thread
    /// draining `datagrams` into `producer` at real presentation cadence.
    ///
    /// `now_ms` reads the same clock the datagrams' own
    /// `host_presentation_time_ms` was computed against (the desktop host's
    /// transport clock) -- not an independently-chosen one, so the pump's
    /// notion of "now" never drifts from the timeline the packets were
    /// scheduled onto. A `now_ms` read that fails (e.g. the transport is
    /// shutting down) is treated as "nothing is due yet" rather than a
    /// fatal pump error; the next successful read catches up.
    ///
    /// # Errors
    ///
    /// Returns [`MonitorPumpStartError`] if the scheduler/pump
    /// configuration is invalid or the thread fails to start.
    pub(crate) fn start(
        scheduler_config: SchedulerConfig,
        producer: RenderRingProducer,
        datagrams: Receiver<AudioDatagram>,
        now_ms: impl Fn() -> Option<u64> + Send + 'static,
    ) -> Result<Self, MonitorPumpStartError> {
        let scheduler = PlaybackScheduler::new(scheduler_config, 0.0)
            .map_err(MonitorPumpStartError::Scheduler)?;
        let mut pump = PlaybackPump::new(scheduler, producer, PlaybackPumpConfig::default())
            .map_err(MonitorPumpStartError::Pump)?;
        // Locks the pump immediately at a fixed zero offset -- see the
        // module doc comment for why a desktop monitor never estimates or
        // updates a clock-offset the way a real cross-device listener does.
        let _ = pump.apply_sync_offset(0.0);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("silent-disco-desktop-monitor-pump".to_owned())
            .spawn(move || run_pump(pump, &datagrams, &stop_for_thread, now_ms))
            .map_err(|error| MonitorPumpStartError::ThreadSpawnFailed(error.to_string()))?;

        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Signals the pump thread to stop and blocks until it has exited --
    /// the monitor's render-ring producer is dropped only once this
    /// returns, so a caller that acquires a fresh lease immediately
    /// afterward never races the outgoing one.
    pub(crate) fn stop(mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| "local monitor pump thread panicked during shutdown".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for DesktopMonitorPump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            // Normal owners call `stop()`, which propagates the join failure.
            // Reaching Drop with a panicked worker is therefore a lifecycle
            // invariant violation. Fail loudly instead of reducing that
            // operational failure to a stderr-only message. During an
            // unrelated panic, do not trigger a second panic while unwinding.
            assert!(
                std::thread::panicking(),
                "local monitor pump thread panicked during implicit shutdown"
            );
        }
    }
}

#[cfg(test)]
mod drop_tests {
    use super::DesktopMonitorPump;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    #[test]
    fn implicit_drop_fails_loudly_when_the_monitor_worker_panicked() {
        let worker = thread::spawn(|| panic!("injected monitor pump panic"));
        let pump = DesktopMonitorPump {
            stop: Arc::new(AtomicBool::new(false)),
            thread: Some(worker),
        };

        let dropped = catch_unwind(AssertUnwindSafe(|| drop(pump)));
        assert!(
            dropped.is_err(),
            "a panicked monitor worker must not become a log-only Drop failure"
        );
    }

    #[test]
    fn implicit_drop_accepts_a_clean_monitor_worker_exit() {
        let worker = thread::spawn(|| {});
        let pump = DesktopMonitorPump {
            stop: Arc::new(AtomicBool::new(false)),
            thread: Some(worker),
        };

        let dropped = catch_unwind(AssertUnwindSafe(|| drop(pump)));
        assert!(
            dropped.is_ok(),
            "a clean implicit join must remain harmless"
        );
    }
}

fn run_pump(
    mut pump: PlaybackPump,
    datagrams: &Receiver<AudioDatagram>,
    stop: &AtomicBool,
    now_ms: impl Fn() -> Option<u64>,
) {
    while !stop.load(Ordering::Acquire) {
        while let Ok(datagram) = datagrams.try_recv() {
            // A rejected packet (e.g. arrived before this pump's own
            // startup buffer target) simply is not monitored this instant
            // -- never fatal, and never affects the host's own network
            // broadcast, which does not go through this pump at all.
            drop(pump.submit_packet(datagram));
        }
        if let Some(now) = now_ms() {
            drain_due_frames(&mut pump, now);
        }
        thread::sleep(PUMP_TICK_INTERVAL);
    }
}

/// Advances `pump` until nothing further is due, mirroring
/// `listener_playback.rs`'s own `drain_due_frames` exactly (same reasoning:
/// one `tick` releases at most one frame, so draining once per wake-up
/// silently caps throughput at one packet per [`PUMP_TICK_INTERVAL`] the
/// moment a packet carries less audio than that interval).
fn drain_due_frames(pump: &mut PlaybackPump, now_ms: u64) -> usize {
    use silent_disco_core::audio::PumpTick;
    let mut released = 0;
    for _ in 0..MAX_FRAMES_PER_TICK {
        match pump.tick(now_ms) {
            PumpTick::Queued { .. } | PumpTick::FlushedPending { .. } => released += 1,
            _ => break,
        }
    }
    released
}
