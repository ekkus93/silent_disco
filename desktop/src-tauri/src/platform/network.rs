//! Desktop host network control, split by responsibility, following this
//! crate's own `start_playback_tests.rs` + `start_playback_tests/`
//! convention (a thin root file, no `mod.rs` rename needed since
//! `platform/mod.rs`'s `pub mod network;` line does not change):
//! - [`interfaces`]: OS interface enumeration and normalization
//!   (`InterfaceRecord`/`AddressRecord`/`NetworkInterfaceProvider`).
//! - [`classification`]: address classification predicates (loopback,
//!   link-local, VPN, container, private-LAN) and their precedence order.
//! - [`bind_selection`]: turning a classified snapshot into a concrete bind
//!   address, or an explicit, typed rejection reason.
//! - [`host_control`]: `DesktopHostNetworkControl`'s binding lifecycle
//!   (construct/bind/stop/shutdown/`Drop`) and monitor delegation.
//! - [`playback_control`]: starting/pausing/resuming/stopping the active
//!   broadcast stream, and its live decode/packetizer diagnostics.
//! - [`dto_bridge`]: translating internal state into the Tauri-facing DTOs.
//!
//! Every item below is re-exported at this module's own path (unchanged
//! from the pre-split flat file), so `network_tests.rs`, `app_state.rs`,
//! `platform::effect_runner`, `platform::start_playback`,
//! `platform::playback_streamer`, `platform::diagnostics`, and
//! `platform::start_playback_tests::{harness, manual::automation}` all keep
//! compiling unchanged.

mod bind_selection;
mod classification;
mod dto_bridge;
mod host_control;
mod interfaces;
mod playback_control;

pub(super) use super::network_error::{DesktopNetworkError, NetworkErrorKind};
pub(crate) use host_control::DesktopHostNetworkControl;
pub(super) use interfaces::{AddressRecord, InterfaceRecord, NetworkInterfaceProvider};
pub(crate) use playback_control::{StreamDiagnostics, StreamDiagnosticsSnapshot};

#[cfg(test)]
pub(super) use bind_selection::first_bindable_private_lan_address;
#[cfg(test)]
pub(super) use host_control::HostPorts as TestHostPorts;
