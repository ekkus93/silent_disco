//! Presentation/actor contract records, split by concern into `records/`:
//! command intents ([`command`]), platform-effect requests/facts
//! ([`platform`]), transport/audio/storage facts ([`events`]), the
//! authoritative snapshot and actor input envelope ([`snapshot`]), and the
//! shared validation error type ([`errors`]).
//!
//! This file is itself loaded as the `contract` submodule of
//! `records_runtime.rs` (via `#[path = "records.rs"] mod contract;`), which
//! layers [`crate::runtime::records::CoreNotification`] on top and
//! re-exports everything below at `crate::runtime::records::*` and, from
//! there, `crate::runtime::*`.

#[path = "records/command.rs"]
mod command;
#[path = "records/errors.rs"]
mod errors;
#[path = "records/events.rs"]
mod events;
#[path = "records/platform.rs"]
mod platform;
#[path = "records/snapshot.rs"]
mod snapshot;
#[cfg(test)]
#[path = "records/tests.rs"]
mod tests;

pub use command::{CommandReceipt, CoreCommand, CoreCommandRequest};
pub use errors::RuntimeContractError;
pub use events::{AudioEvent, StorageCompletion, StorageEvent, TransportEvent};
pub use platform::{
    AudioOutputInfo, AudioOutputRequest, DiscoveryRequest, NetworkEstablishmentRequest,
    PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion,
};
pub use snapshot::{CoreActorInput, CoreSnapshot, RecoverableAction};

use crate::protocol::PROTOCOL_VERSION;

pub const MAX_DISCOVERED_SESSIONS: usize = 128;
pub const MAX_PENDING_JOIN_REQUESTS: usize = 128;
pub const MAX_CONNECTED_LISTENERS: usize = 256;
pub const MAX_CAPABILITY_REQUESTS: usize = 16;
pub const MAX_EXPORT_ID_BYTES: usize = 128;
pub const MAX_STORAGE_TRUSTED_DEVICES: usize = 1_024;

/// Returns the shared wire-protocol version used in session advertisements.
#[must_use]
pub const fn current_protocol_version() -> u16 {
    PROTOCOL_VERSION
}
