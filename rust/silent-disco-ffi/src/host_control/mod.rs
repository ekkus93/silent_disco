#[cfg(test)]
mod admission_flow_tests;
mod conversions;
mod handle;
mod handle_trust;
mod live_revocation;
pub(crate) use live_revocation::listener_revoked;
mod types;

pub use handle::FfiCoreHandle;
pub(crate) use handle_trust::trust_records_changed;
pub use types::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCommandReceipt,
    FfiCoreDiagnostic, FfiCoreError, FfiCoreNotification, FfiCoreObserver, FfiCoreSnapshot,
    FfiDeliveryReport, FfiHostDraft, FfiHostLifecycle, FfiJoinRequest, FfiJoinRequestInput,
    FfiListenerLifecycle, FfiListenerSummary, FfiPlatformCompletion, FfiPlatformEffect,
    FfiPlaybackState, FfiQueueEntry, FfiSessionAdvertisement, FfiStorageEffect,
    FfiSynchronizationSummary, FfiTransportEffect, FfiTransportState, FfiTrustState,
    FfiTuningPatch, FfiTuningSettings,
};
