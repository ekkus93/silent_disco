//! Drives one [`PlaybackScheduler`](crate::audio::PlaybackScheduler) into one
//! [`RenderRingProducer`](crate::audio::RenderRingProducer), converting each
//! scheduled frame from interleaved PCM16 into the ring's interleaved
//! float32 format.
//!
//! This is the non-real-time half of playback: it runs on an ordinary worker
//! thread and may allocate, while the real-time audio callback only ever
//! reads from the ring through the narrow C ABI. Frames the ring cannot
//! accept immediately are held and retried, never discarded — silently
//! dropping the unwritten remainder of a partial write is audible as a
//! corrupted note rather than a clean gap.
//!
//! Split across sibling files by responsibility: [`config`] is
//! [`PlaybackPump`]'s tuning and its construction-validation failures;
//! [`pump`] is the [`PlaybackPump`] struct itself plus construction and
//! trivial accessors; [`sync`] is the clock-offset gate that decides when
//! playback may start at all; [`scheduling`] is the tick-driven pacing loop
//! that decides when a frame moves toward the ring; [`conversion`] is the
//! PCM16-to-float32 conversion; [`recording`] is the optional debug capture
//! of exactly what was released toward the ring; [`diagnostics`] is the
//! point-in-time snapshot used to tell where audio went missing or wrong.
//!
//! [`PlaybackPump`]'s fields are `pub(super)` rather than private, since its
//! `impl` is spread across every sibling file above — a mechanical, non-
//! behavioral consequence of the split, not a widening of its public API.

mod config;
mod conversion;
mod diagnostics;
mod pump;
mod recording;
mod scheduling;
mod sync;

#[cfg(test)]
mod tests;

pub use config::{
    DEFAULT_MAX_PREFILL_MS, DEFAULT_WRITE_LEAD_MS, PlaybackPumpConfig, PlaybackPumpConfigError,
    PlaybackPumpConfigErrorKind,
};
pub use diagnostics::PlaybackDiagnostics;
pub use pump::PlaybackPump;
pub use scheduling::PumpTick;
pub use sync::SyncApplyOutcome;
