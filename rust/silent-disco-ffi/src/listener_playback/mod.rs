//! Rust-owned listener playback runtime: one scheduler, one render ring, and
//! one dedicated pump thread per stream.
//!
//! Kotlin/Swift submit arriving packets and clock-offset updates through this
//! handle and hand [`FfiListenerPlaybackHandle::engine_token`] to native audio
//! setup exactly once. Everything between an arriving packet and the render
//! ring — ordering, concealment, presentation-time pacing, and PCM conversion
//! — happens here, so no platform layer decides when audio plays or what
//! covers a gap.
//!
//! Split across sibling files by responsibility: [`error`] is the runtime's
//! own error type; [`pump`] is the pump thread's internals (tuning constants,
//! shared state, the tick/drain loop); [`runtime`] is the owning handle
//! callers hold per stream; [`ffi_types`] are the `UniFFI` wire-shape DTOs;
//! [`ffi_convert`] bridges core types and those DTOs; [`ffi_handle`] is the
//! foreign-facing `#[uniffi::export]` surface.

mod error;
mod ffi_convert;
mod ffi_handle;
mod ffi_types;
mod pump;
mod runtime;

#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tests;

pub use error::ListenerPlaybackError;
pub use ffi_handle::FfiListenerPlaybackHandle;
pub use ffi_types::{
    FfiAudioPacket, FfiListenerPlaybackConfig, FfiListenerPlaybackError, FfiPlaybackDiagnostics,
    FfiPlaybackPhase, FfiSyncConfidence, FfiSyncSampleOutcome,
};
pub use runtime::ListenerPlaybackRuntime;
