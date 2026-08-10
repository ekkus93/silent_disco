//! Deterministic virtual clock and scheduler for Lab Mode (Block 38).
//!
//! Builds on the shared core's own `TransportClock`/`ManualTransportClock`
//! (Block 38.1: "use the actual existing time abstractions where
//! present, do not duplicate them" -- production already has exactly the
//! platform-independent `fn now(&self) -> MonotonicMillis` trait Block
//! 38.1 calls for). This module adds only what production's manual clock
//! does not have: a deterministic event scheduler, per-node offset/drift
//! views, checked arithmetic, and explicit discontinuity injection
//! reserved for negative tests.
//!
//! Nothing here ever reads the OS clock (`std::time::Instant`/`SystemTime`)
//! -- virtual time only ever moves through an explicit [`LabClock::advance`]
//! call, so a Lab scenario can run a deterministic sync/scheduling test in
//! zero wall-clock time (Block 38.2 "no wall-clock sleep required for
//! deterministic scenarios").

use silent_disco_core::domain::MonotonicMillis;
use silent_disco_core::transport::{ManualTransportClock, TransportClock};
use std::cmp::Ordering as CmpOrdering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::Mutex;

/// Sanity bound on configured per-node drift (Block 38.2 "checked
/// arithmetic"): generous enough for any realistic or deliberately
/// pathological fault-injection scenario (real hardware clock drift is
/// typically single-digit-to-low-hundreds ppm), but rejected outright
/// rather than silently accepted and only discovered to be nonsensical
/// later.
const MAX_DRIFT_PPM: u64 = 100_000;

const PPM_DENOMINATOR: i128 = 1_000_000;

/// Bounded set of failures constructing or advancing a Lab clock (Block
/// 38.2 "checked arithmetic").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LabClockError {
    /// A configured per-node drift exceeded [`MAX_DRIFT_PPM`].
    DriftOutOfBounds,
    /// A requested manual advance would overflow the current virtual
    /// time; time is left completely unchanged (no partial advance, no
    /// wakeups run).
    AdvanceOverflow,
    /// The scheduler's own registration counter is exhausted -- would
    /// require scheduling `u64::MAX` wakeups in one [`LabClock`], not a
    /// realistic scenario.
    SchedulerExhausted,
    /// The scheduler's internal mutex was poisoned.
    SchedulerPoisoned,
}

impl std::fmt::Display for LabClockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DriftOutOfBounds => {
                write!(
                    formatter,
                    "configured drift exceeds the supported bound of {MAX_DRIFT_PPM} ppm"
                )
            }
            Self::AdvanceOverflow => {
                formatter.write_str("advancing virtual time by the requested amount would overflow")
            }
            Self::SchedulerExhausted => {
                formatter.write_str("Lab clock scheduler registration counter is exhausted")
            }
            Self::SchedulerPoisoned => {
                formatter.write_str("Lab clock scheduler mutex was poisoned")
            }
        }
    }
}

impl std::error::Error for LabClockError {}

struct ScheduledWakeup {
    deadline_ms: u64,
    sequence: u64,
    callback: Box<dyn FnOnce() + Send>,
}

impl PartialEq for ScheduledWakeup {
    fn eq(&self, other: &Self) -> bool {
        self.deadline_ms == other.deadline_ms && self.sequence == other.sequence
    }
}

impl Eq for ScheduledWakeup {}

impl PartialOrd for ScheduledWakeup {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledWakeup {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Equal deadlines break ties by registration order -- Block 38.3
        // "scheduler event order at equal timestamps" is deterministic,
        // never dependent on `BinaryHeap`'s own unspecified tie-breaking.
        self.deadline_ms
            .cmp(&other.deadline_ms)
            .then_with(|| self.sequence.cmp(&other.sequence))
    }
}

#[derive(Default)]
struct Scheduler {
    next_sequence: u64,
    pending: BinaryHeap<Reverse<ScheduledWakeup>>,
}

/// The base, whole-scenario virtual timeline (Block 38.2 "deterministic
/// initial time", "manual advance", "scheduled wakeups through the Lab
/// scheduler"). Every Lab node's own clock ([`LabNodeClock`]) is a
/// per-node offset/drift view over this one shared timeline -- advancing
/// it is the only way virtual time ever moves anywhere in a Lab
/// scenario.
pub(crate) struct LabClock {
    base: ManualTransportClock,
    scheduler: Mutex<Scheduler>,
}

impl LabClock {
    /// Creates a virtual timeline starting at a deterministic, caller-chosen
    /// instant (Block 38.2 "deterministic initial time").
    #[must_use]
    pub(crate) fn new(initial_ms: u64) -> Self {
        Self {
            base: ManualTransportClock::new(initial_ms),
            scheduler: Mutex::new(Scheduler::default()),
        }
    }

    #[must_use]
    pub(crate) fn now(&self) -> MonotonicMillis {
        self.base.now()
    }

    /// Schedules `callback` to run the next time virtual time reaches or
    /// passes `deadline_ms` (Block 38.2 "scheduled wakeups through the
    /// Lab scheduler"). A deadline at or before the current time runs on
    /// the very next [`Self::advance`] call (including a zero-length
    /// advance) -- it is never silently dropped or skipped.
    ///
    /// # Errors
    ///
    /// Returns [`LabClockError::SchedulerExhausted`] or
    /// [`LabClockError::SchedulerPoisoned`].
    pub(crate) fn schedule(
        &self,
        deadline_ms: u64,
        callback: impl FnOnce() + Send + 'static,
    ) -> Result<(), LabClockError> {
        let mut scheduler = self
            .scheduler
            .lock()
            .map_err(|_| LabClockError::SchedulerPoisoned)?;
        let sequence = scheduler.next_sequence;
        scheduler.next_sequence = scheduler
            .next_sequence
            .checked_add(1)
            .ok_or(LabClockError::SchedulerExhausted)?;
        scheduler.pending.push(Reverse(ScheduledWakeup {
            deadline_ms,
            sequence,
            callback: Box::new(callback),
        }));
        Ok(())
    }

    /// Moves virtual time forward by exactly `delta_ms` and then runs
    /// every wakeup now due, in deterministic (deadline, then
    /// registration order) order (Block 38.2 "manual advance", "checked
    /// arithmetic"; Block 38.3 "scheduler event order at equal
    /// timestamps"). Never sleeps.
    ///
    /// # Errors
    ///
    /// Returns [`LabClockError::AdvanceOverflow`] when `delta_ms` would
    /// overflow the current time -- time is left completely unchanged and
    /// no wakeup runs. Returns [`LabClockError::SchedulerPoisoned`] if a
    /// due wakeup's own scheduling of a further wakeup finds the
    /// scheduler mutex poisoned.
    pub(crate) fn advance(&self, delta_ms: u64) -> Result<MonotonicMillis, LabClockError> {
        let current = self.base.now().get();
        let next = current
            .checked_add(delta_ms)
            .ok_or(LabClockError::AdvanceOverflow)?;
        self.base.set(next);
        self.run_due(next)?;
        Ok(MonotonicMillis::new(next))
    }

    /// Explicitly injects a time discontinuity -- setting virtual time to
    /// any value, including one that moves it *backward* -- bypassing
    /// [`Self::advance`]'s normal monotonic, checked-overflow path
    /// entirely (Block 38.2 "explicit invalid-discontinuity injection
    /// only for negative tests"). Reserved for scenarios that
    /// deliberately test invalid-clock handling (e.g. a listener's sync
    /// estimator rejecting a bad sample); ordinary scenario advancement
    /// must always use [`Self::advance`] instead. No due wakeups are run
    /// by this call -- a discontinuity is not itself scenario progress.
    pub(crate) fn force_discontinuity(&self, new_now_ms: u64) {
        self.base.set(new_now_ms);
    }

    fn run_due(&self, now_ms: u64) -> Result<(), LabClockError> {
        loop {
            let due = {
                let mut scheduler = self
                    .scheduler
                    .lock()
                    .map_err(|_| LabClockError::SchedulerPoisoned)?;
                let is_due = matches!(
                    scheduler.pending.peek(),
                    Some(Reverse(wakeup)) if wakeup.deadline_ms <= now_ms
                );
                if is_due {
                    scheduler.pending.pop().map(|Reverse(wakeup)| wakeup)
                } else {
                    None
                }
            };
            let Some(wakeup) = due else {
                return Ok(());
            };
            (wakeup.callback)();
        }
    }
}

/// One Lab node's own view of the shared [`LabClock`], with an optional
/// fixed offset and/or drift applied (Block 38.2 "per-node offset",
/// "per-node drift in ppm"). Implements the shared core's own
/// [`TransportClock`], so a later Lab Mode block's virtual transport
/// wiring can use it exactly like production's `SystemTransportClock`/
/// `ManualTransportClock` -- no parallel Lab-only clock trait to
/// special-case around.
///
/// Drift is applied relative to the shared timeline's own origin
/// (virtual time zero), not this node's construction time: two nodes
/// configured with the same drift observe the exact same drifted value
/// at the same shared time, regardless of when each was created.
pub(crate) struct LabNodeClock {
    base: Arc<LabClock>,
    offset_ms: i64,
    drift_ppm: i64,
}

impl LabNodeClock {
    /// Creates a per-node clock view over `base`.
    ///
    /// # Errors
    ///
    /// Returns [`LabClockError::DriftOutOfBounds`] when `drift_ppm`'s
    /// magnitude exceeds [`MAX_DRIFT_PPM`].
    pub(crate) fn new(
        base: Arc<LabClock>,
        offset_ms: i64,
        drift_ppm: i64,
    ) -> Result<Self, LabClockError> {
        if drift_ppm.unsigned_abs() > MAX_DRIFT_PPM {
            return Err(LabClockError::DriftOutOfBounds);
        }
        Ok(Self {
            base,
            offset_ms,
            drift_ppm,
        })
    }

    #[must_use]
    pub(crate) fn offset_ms(&self) -> i64 {
        self.offset_ms
    }

    #[must_use]
    pub(crate) fn drift_ppm(&self) -> i64 {
        self.drift_ppm
    }
}

impl TransportClock for LabNodeClock {
    fn now(&self) -> MonotonicMillis {
        // Widened to i128 so this can never overflow for any realistic
        // (or even maximally pathological, given `MAX_DRIFT_PPM`'s bound)
        // combination of base time, offset, and drift; the result is then
        // clamped into `u64`'s valid range rather than wrapping or
        // panicking -- the same "clamp on conversion" discipline already
        // used by `SystemTransportClock::now()` in the shared core.
        let base_now = i128::from(self.base.now().get());
        let drifted = base_now * (PPM_DENOMINATOR + i128::from(self.drift_ppm)) / PPM_DENOMINATOR;
        let total = drifted + i128::from(self.offset_ms);
        let clamped = total.clamp(0, i128::from(u64::MAX));
        MonotonicMillis::new(u64::try_from(clamped).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests;
