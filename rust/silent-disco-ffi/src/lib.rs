#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

//! Binding shell for the shared Rust core.
//!
//! This crate contains no domain logic. Unsafe code is denied by default. A
//! module that requires unsafe export attributes may opt in only at the
//! smallest scope, while unsafe blocks and foreign-pointer dereferences remain
//! prohibited unless their invariants are documented explicitly.

uniffi::setup_scaffolding!();

mod android_abi;
mod android_database_abi;
mod android_p2_abi;
mod audio_abi;
mod audio_output;
mod host_control;
mod listener_transport;
mod sync;

use silent_disco_core::{CoreVersion, core_version, deterministic_smoke};

pub use android_abi::{CORE_ABI_VERSION, silent_disco_core_abi_version};
pub use audio_abi::AUDIO_ABI_VERSION;
pub use audio_output::{FfiAudioOutputError, FfiAudioOutputHandle};
pub use host_control::{
    FfiAppRole, FfiApprovalMode, FfiAudioSource, FfiBridgeError, FfiCommandReceipt,
    FfiCoreDiagnostic, FfiCoreError, FfiCoreHandle, FfiCoreNotification, FfiCoreObserver,
    FfiCoreSnapshot, FfiDeliveryReport, FfiHostDraft, FfiHostLifecycle, FfiJoinRequest,
    FfiJoinRequestInput, FfiListenerSummary, FfiPlatformCompletion, FfiPlatformEffect,
    FfiPlaybackState, FfiStorageEffect, FfiSynchronizationSummary, FfiTransportEffect,
    FfiTransportState, FfiTrustState, FfiTuningPatch, FfiTuningSettings,
};
pub use listener_transport::{
    FfiListenerDatagramRoutes, FfiListenerTransportCounters, FfiListenerTransportError,
    FfiListenerTransportEvent, FfiListenerTransportHandle, FfiManualHostEndpoint,
    parse_manual_host_endpoint,
};
pub use sync::{
    FfiSyncError, FfiSyncEstimator, FfiSyncEstimatorConfig, FfiSyncExchange, FfiSyncObservation,
    FfiSyncSnapshot,
};

/// Binding-facing version record. Domain version ownership remains in the core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FfiCoreVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl From<CoreVersion> for FfiCoreVersion {
    fn from(value: CoreVersion) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
            patch: value.patch,
        }
    }
}

#[must_use]
#[uniffi::export]
pub fn ffi_core_version() -> FfiCoreVersion {
    core_version().into()
}

#[must_use]
#[uniffi::export]
pub fn ffi_deterministic_smoke(input: u64) -> u64 {
    deterministic_smoke(input)
}

#[cfg(test)]
mod tests {
    use super::{
        CORE_ABI_VERSION, FfiCoreVersion, ffi_core_version, ffi_deterministic_smoke,
        silent_disco_core_abi_version,
    };

    #[test]
    fn delegates_version_to_core() {
        assert_eq!(
            ffi_core_version(),
            FfiCoreVersion {
                major: 0,
                minor: 1,
                patch: 0,
            }
        );
    }

    #[test]
    fn delegates_smoke_function_to_core() {
        assert_eq!(ffi_deterministic_smoke(7), ffi_deterministic_smoke(7));
    }

    #[test]
    fn exports_stable_abi_version() {
        assert_eq!(silent_disco_core_abi_version(), u32::from(CORE_ABI_VERSION));
        assert_eq!(CORE_ABI_VERSION, 1);
    }
}
