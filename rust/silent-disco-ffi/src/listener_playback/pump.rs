//! Pump-thread internals: tuning constants, the pump clock, the state
//! shared between the owning handle and its pump thread, and the per-tick
//! drain loop.
//!
//! Everything reachable from [`run_pump`] runs on the dedicated pump thread,
//! so nothing here may call into `UniFFI`, JNI, `SQLite`, networking, logging,
//! allocation-heavy code, or blocking synchronization beyond the mutexes
//! [`Shared`] already owns.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use silent_disco_core::audio::{PlaybackPump, PumpTick};
use silent_disco_core::sync::ClockSyncEstimator;

/// How often the pump thread wakes to check whether audio is due.
pub(super) const PUMP_TICK_INTERVAL: Duration = Duration::from_millis(10);
/// Safety bound on frames drained in one pump wake-up. The write lead and
/// ring depth cap are what actually stop the drain; this only prevents an
/// unbounded loop if both were ever misconfigured. Generous enough to cover a
/// full write lead of the shortest supported packets plus catch-up.
pub(super) const MAX_FRAMES_PER_TICK: usize = 512;
/// How often the stopping thread checks whether the render ring has drained.
pub(super) const RING_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// Overall ceiling on waiting for the ring to drain at stop. Comfortably above
/// a full ring at the supported sample rates, so it only ever bounds the case
/// where nothing is consuming.
pub(super) const RING_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the ring may fail to shrink before the consumer is treated as not
/// running, so a closed or failed output does not cost the full timeout.
pub(super) const RING_DRAIN_STALL_LIMIT: Duration = Duration::from_millis(150);

#[cfg(test)]
static ACTIVE_PUMP_THREADS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
pub(super) fn active_pump_threads_for_test() -> u64 {
    ACTIVE_PUMP_THREADS.load(Ordering::SeqCst)
}

#[cfg(test)]
struct ActivePumpThreadGuard;

#[cfg(test)]
impl ActivePumpThreadGuard {
    fn enter() -> Self {
        ACTIVE_PUMP_THREADS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for ActivePumpThreadGuard {
    fn drop(&mut self) {
        ACTIVE_PUMP_THREADS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Monotonic milliseconds since this runtime started, matching the local
/// timeline the caller's clock-offset estimates are expressed against.
#[derive(Debug)]
pub(super) struct PumpClock {
    origin: Instant,
}

impl PumpClock {
    pub(super) fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub(super) fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// State shared between the owning handle and its pump thread.
#[derive(Debug)]
pub(super) struct Shared {
    pub(super) pump: Mutex<PlaybackPump>,
    /// Owned here rather than by the platform: a listener that estimates its
    /// own offset or skew is duplicating domain logic, and doing it in the
    /// platform layer is what produced a physically impossible skew (and
    /// total silence) in the previous implementation.
    pub(super) estimator: Mutex<ClockSyncEstimator>,
    pub(super) running: AtomicBool,
    /// Number of pump-loop wake-ups completed or attempted. Monotonic and
    /// cheap enough to expose as a liveness signal without logging per tick.
    pub(super) tick_count: AtomicU64,
    /// Runtime-relative monotonic timestamp of the most recent pump wake-up.
    pub(super) last_tick_ms: AtomicU64,
    /// Panics caught at the pump-thread boundary instead of escaping silently.
    pub(super) contained_panics: AtomicU64,
    /// First terminal worker failure, retained for every later API call.
    pub(super) terminal_failure: OnceLock<String>,
    #[cfg(test)]
    pub(super) panic_next_tick: AtomicBool,
}

impl Shared {
    pub(super) fn lock_pump(&self) -> MutexGuard<'_, PlaybackPump> {
        self.pump.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn lock_estimator(&self) -> MutexGuard<'_, ClockSyncEstimator> {
        self.estimator
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
pub(super) fn run_pump(shared: &Arc<Shared>, clock: &Arc<PumpClock>) {
    while shared.running.load(Ordering::SeqCst) {
        let now_ms = clock.now_ms();
        shared.tick_count.fetch_add(1, Ordering::Relaxed);
        shared.last_tick_ms.store(now_ms, Ordering::Relaxed);
        #[cfg(test)]
        if shared.panic_next_tick.swap(false, Ordering::SeqCst) {
            panic!("injected playback pump panic");
        }
        {
            let mut pump = shared.lock_pump();
            drain_due_frames(&mut pump, now_ms);
        }
        thread::sleep(PUMP_TICK_INTERVAL);
    }
}

/// Runs the pump behind a containment boundary so an unexpected panic is a
/// retained runtime failure rather than a frozen diagnostics snapshot that
/// still looks like a live stream.
pub(super) fn run_pump_contained(shared: &Arc<Shared>, clock: &Arc<PumpClock>) {
    #[cfg(test)]
    let _active_thread = ActivePumpThreadGuard::enter();
    let outcome = catch_unwind(AssertUnwindSafe(|| run_pump(shared, clock)));
    if outcome.is_err() {
        shared.contained_panics.fetch_add(1, Ordering::SeqCst);
        let _ = shared
            .terminal_failure
            .set("playback pump thread panicked".to_owned());
        shared.running.store(false, Ordering::SeqCst);
    }
}

/// Advances `pump` until nothing further is due, returning how many frames
/// moved toward the ring.
///
/// One [`PlaybackPump::tick`] releases at most one frame, so draining exactly
/// one per wake-up caps throughput at one packet per [`PUMP_TICK_INTERVAL`].
/// That ceiling is invisible while packets are long and becomes a hard limit
/// below real time the moment they carry less audio than the tick interval.
pub(super) fn drain_due_frames(pump: &mut PlaybackPump, now_ms: u64) -> usize {
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
