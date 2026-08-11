#[allow(
    dead_code,
    reason = "Desktop Block 25 consumes the bounded prepared PCM stream"
)]
pub(crate) mod audio_decode;
pub mod audio_device;
pub(crate) mod capabilities;
pub(crate) mod diagnostics;
pub mod diagnostics_export;
pub mod discovery;
pub mod effect_runner;
mod failure;
pub mod file_picker;
mod host_join_projection;
mod host_pending_handshake;
pub(crate) mod host_transport;
pub(crate) mod host_transport_events;
pub mod identity;
pub(crate) mod invitation;
pub mod invitation_identity;
mod mdns;
mod monitor;
mod monitor_pump;
pub mod network;
pub mod network_dto;
mod network_error;
#[allow(
    clippy::similar_names,
    reason = "canonical profile and profiles roots are distinct security boundaries"
)]
pub mod paths;
mod playback_streamer;
pub mod profile_lock;
pub mod profile_metadata;
pub(crate) mod render_ring;
pub(crate) mod source_staging;
pub(crate) mod source_staging_control;
pub(crate) mod start_playback;
pub(crate) mod storage_effect_runner;
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
mod cpal_spike_tests;
#[cfg(test)]
mod effect_runner_tests;
#[cfg(test)]
mod file_picker_tests;
#[cfg(test)]
mod host_transport_admission_tests;
#[cfg(test)]
mod host_transport_tests;
#[cfg(test)]
mod monitor_tests;
#[cfg(test)]
mod network_tests;
#[cfg(test)]
mod start_playback_tests;
#[cfg(test)]
mod storage_effect_runner_tests;
