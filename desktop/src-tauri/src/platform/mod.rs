#[allow(
    dead_code,
    reason = "Desktop Block 25 consumes the bounded prepared PCM stream"
)]
pub(crate) mod audio_decode;
pub mod audio_device;
pub(crate) mod capabilities;
pub mod diagnostics_export;
pub mod discovery;
pub mod effect_runner;
mod failure;
pub mod file_picker;
pub mod identity;
pub mod network;
pub mod network_dto;
mod network_error;
#[allow(
    clippy::similar_names,
    reason = "canonical profile and profiles roots are distinct security boundaries"
)]
pub mod paths;
pub mod profile_lock;
pub mod profile_metadata;
pub(crate) mod source_staging;
pub(crate) mod source_staging_control;
#[allow(
    clippy::unnested_or_patterns,
    reason = "the test keeps complete result variants visually separate"
)]
pub mod storage_inspection;

#[cfg(test)]
mod audio_decode_tests;
#[cfg(test)]
mod capability_tests;
#[cfg(test)]
mod effect_runner_tests;
#[cfg(test)]
mod file_picker_tests;
#[cfg(test)]
mod network_tests;
