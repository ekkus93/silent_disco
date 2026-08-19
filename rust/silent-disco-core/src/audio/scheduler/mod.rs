//! Maps a bounded jitter buffer and silence-concealment policy onto a
//! host-presentation timeline, producing render-ready PCM frames for exactly
//! one stream. See [`PlaybackScheduler`] for the full contract.
//!
//! Split across sibling files by responsibility: [`config`] is the tuning
//! constants and [`SchedulerConfig`]; [`errors`] is the construction-failure
//! taxonomy; [`types`] is the diagnostics/output types (`PlaybackPhase`,
//! `BufferHealth`, `ScheduledFrame`, `SchedulerPoll`, `OffsetUpdateOutcome`)
//! plus the private lifecycle state; [`engine`] is [`PlaybackScheduler`]
//! itself — construction/validation, the tick/presentation-time/drift-resync
//! engine, drain and waveform-continuity handling, clock-offset/lifecycle
//! controls, and diagnostics accessors, kept as one file since those pieces
//! share private mutable state and are only safely auditable together.

mod config;
mod engine;
mod errors;
mod types;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod buffering_tests;
#[cfg(test)]
mod concealment_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod resync_tests;
#[cfg(test)]
mod timing_tests;

// `DEFAULT_REBUFFER_TARGET_MS`, `DEFAULT_CONCEALMENT_BRIDGE_MS`,
// `DEFAULT_CONCEALMENT_SKIP_THRESHOLD_MS`, and `DEFAULT_REORDER_WINDOW_MS`
// are internal-only tuning defaults consumed by `SchedulerConfig::new`
// itself; they are deliberately not re-exported here (`audio/mod.rs` never
// re-exported them either).
pub use config::{
    DEFAULT_CONCEALMENT_SKIP_THRESHOLD_PACKETS, DEFAULT_HARD_RESYNC_THRESHOLD_MS,
    DEFAULT_HIGH_WATER_MS, DEFAULT_LOW_WATER_MS, DEFAULT_STARTUP_BUFFER_TARGET_MS, SchedulerConfig,
};
pub use engine::PlaybackScheduler;
pub use errors::{SchedulerConfigError, SchedulerConfigErrorKind};
pub use types::{BufferHealth, OffsetUpdateOutcome, PlaybackPhase, ScheduledFrame, SchedulerPoll};

// The test files below are direct children of this module (matching the
// production file's own sibling layout), so from their perspective `super`
// names this module. `host_to_local_ms` is `pub(super)` on `engine` (i.e.
// visible throughout this module's subtree), and this alias keeps it
// reachable at the exact `super::scheduler::host_to_local_ms(...)` path the
// pre-split flat `scheduler_tests.rs` used, without rewriting those call
// sites.
#[cfg(test)]
use engine as scheduler;
