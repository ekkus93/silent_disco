pub mod audio_device;
pub mod diagnostics_export;
pub mod discovery;
pub mod effect_runner;
mod failure;
pub mod file_picker;
pub mod identity;
#[allow(
    clippy::similar_names,
    reason = "canonical profile and profiles roots are distinct security boundaries"
)]
pub mod paths;
pub mod profile_lock;
pub mod profile_metadata;
#[allow(
    clippy::unnested_or_patterns,
    reason = "the test keeps complete result variants visually separate"
)]
pub mod storage_inspection;

#[cfg(test)]
mod effect_runner_tests;
#[cfg(test)]
mod file_picker_tests;
