// Only re-exported at this `records::` path because
// `actor_runtime/state/mod.rs` still reaches them via
// `crate::runtime::records::{CoreDiagnostic, DiagnosticField, SessionAdvertisement}`.
// Every other `types` item this file's `contract` submodule used to pull in
// through this re-export now imports directly from `crate::runtime::{...}`.
pub use super::types::{CoreDiagnostic, DiagnosticField, SessionAdvertisement};
use super::{StorageEffect, TransportEffect};
use crate::error::CoreError;

#[allow(
    clippy::large_enum_variant,
    reason = "the stable contract contains a full immutable snapshot; queue storage boxes notifications"
)]
#[path = "records.rs"]
mod contract;

pub use contract::{
    AudioEvent, AudioOutputInfo, AudioOutputRequest, CommandReceipt, CoreActorInput, CoreCommand,
    CoreCommandRequest, CoreSnapshot, DiscoveryRequest, MAX_CAPABILITY_REQUESTS,
    MAX_CONNECTED_LISTENERS, MAX_DISCOVERED_SESSIONS, MAX_EXPORT_ID_BYTES,
    MAX_PENDING_JOIN_REQUESTS, MAX_STORAGE_TRUSTED_DEVICES, NetworkEstablishmentRequest,
    PermissionCapability, PlatformEffect, PlatformEffectRequest, PlatformEvent,
    PlatformOperationCompletion, RecoverableAction, RuntimeContractError, StorageCompletion,
    StorageEvent, TransportEvent, current_protocol_version,
};

/// Output delivered by the notification dispatcher.
///
/// Platform, transport, and storage effects remain distinct so their adapters
/// cannot accidentally acknowledge the wrong operation family.
#[allow(
    clippy::large_enum_variant,
    reason = "CoreNotification is a stable semantic contract; queue storage boxes notifications"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum CoreNotification {
    Snapshot(CoreSnapshot),
    Effect(PlatformEffect),
    TransportEffect(TransportEffect),
    StorageEffect(StorageEffect),
    Error(CoreError),
    Diagnostic(CoreDiagnostic),
}
