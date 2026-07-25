#![allow(unsafe_code)]

use core::ffi::c_void;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

use crate::{
    FfiSyncError, FfiSyncEstimator, FfiSyncEstimatorConfig, FfiSyncExchange, FfiSyncObservation,
    FfiSyncSnapshot,
};

/// ABI contract implemented by the current native library.
///
/// A `u16` keeps conversion to both the C `u32` result and JNI `i32` result
/// statically infallible.
pub const CORE_ABI_VERSION: u16 = 1;

const MAX_JNI_HANDLE: u64 = 9_223_372_036_854_775_807;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AndroidSyncStatus {
    Accepted = 0,
    Rejected = 1,
    InvalidConfiguration = -1,
    InvalidHandle = -2,
    NegativeTimestamp = -3,
    LocalClockMovedBackwards = -4,
    HostClockMovedBackwards = -5,
    HostProcessingExceedsRoundTrip = -6,
    CorrelationIdExhausted = -7,
    RegistryPoisoned = -8,
    HandleExhausted = -9,
    SampleCountOverflow = -10,
    DuplicateCorrelationId = -11,
    PendingProbeLimitReached = -12,
    StaleCorrelationId = -13,
    CorrelationTimestampMismatch = -14,
    NoObservation = -15,
}

impl AndroidSyncStatus {
    const fn code(self) -> i32 {
        self as i32
    }
}

impl From<FfiSyncError> for AndroidSyncStatus {
    fn from(error: FfiSyncError) -> Self {
        match error {
            FfiSyncError::InvalidConfiguration => Self::InvalidConfiguration,
            FfiSyncError::CorrelationIdExhausted => Self::CorrelationIdExhausted,
            FfiSyncError::SampleCountOverflow => Self::SampleCountOverflow,
            FfiSyncError::DuplicateCorrelationId => Self::DuplicateCorrelationId,
            FfiSyncError::PendingProbeLimitReached => Self::PendingProbeLimitReached,
            FfiSyncError::StaleCorrelationId => Self::StaleCorrelationId,
            FfiSyncError::CorrelationTimestampMismatch => Self::CorrelationTimestampMismatch,
            FfiSyncError::LocalClockMovedBackwards => Self::LocalClockMovedBackwards,
            FfiSyncError::HostClockMovedBackwards => Self::HostClockMovedBackwards,
            FfiSyncError::HostProcessingExceedsRoundTrip => Self::HostProcessingExceedsRoundTrip,
        }
    }
}

#[derive(Debug)]
struct SyncRegistry {
    next_handle: u64,
    estimators: BTreeMap<u64, FfiSyncEstimator>,
}

impl Default for SyncRegistry {
    fn default() -> Self {
        Self {
            next_handle: 1,
            estimators: BTreeMap::new(),
        }
    }
}

static SYNC_REGISTRY: OnceLock<Mutex<SyncRegistry>> = OnceLock::new();

fn sync_registry() -> &'static Mutex<SyncRegistry> {
    SYNC_REGISTRY.get_or_init(|| Mutex::new(SyncRegistry::default()))
}

fn with_estimator<R>(
    handle: i64,
    action: impl FnOnce(&mut FfiSyncEstimator) -> Result<R, AndroidSyncStatus>,
) -> Result<R, AndroidSyncStatus> {
    let handle = u64::try_from(handle).map_err(|_| AndroidSyncStatus::InvalidHandle)?;
    if handle == 0 {
        return Err(AndroidSyncStatus::InvalidHandle);
    }
    let mut registry = sync_registry()
        .lock()
        .map_err(|_| AndroidSyncStatus::RegistryPoisoned)?;
    let estimator = registry
        .estimators
        .get_mut(&handle)
        .ok_or(AndroidSyncStatus::InvalidHandle)?;
    action(estimator)
}

fn create_estimator(max_samples: i32, max_accepted_rtt_ms: f64) -> Result<i64, AndroidSyncStatus> {
    let max_samples =
        u32::try_from(max_samples).map_err(|_| AndroidSyncStatus::InvalidConfiguration)?;
    let estimator = FfiSyncEstimator::new(FfiSyncEstimatorConfig {
        max_samples,
        max_accepted_rtt_ms,
    })
    .map_err(AndroidSyncStatus::from)?;
    let mut registry = sync_registry()
        .lock()
        .map_err(|_| AndroidSyncStatus::RegistryPoisoned)?;
    let handle = registry.next_handle;
    if handle == 0 || handle > MAX_JNI_HANDLE {
        return Err(AndroidSyncStatus::HandleExhausted);
    }
    let next_handle = handle
        .checked_add(1)
        .ok_or(AndroidSyncStatus::HandleExhausted)?;
    let jni_handle = i64::try_from(handle).map_err(|_| AndroidSyncStatus::HandleExhausted)?;
    if registry.estimators.insert(handle, estimator).is_some() {
        return Err(AndroidSyncStatus::HandleExhausted);
    }
    registry.next_handle = next_handle;
    Ok(jni_handle)
}

fn destroy_estimator(handle: i64) -> Result<(), AndroidSyncStatus> {
    let handle = u64::try_from(handle).map_err(|_| AndroidSyncStatus::InvalidHandle)?;
    if handle == 0 {
        return Err(AndroidSyncStatus::InvalidHandle);
    }
    let mut registry = sync_registry()
        .lock()
        .map_err(|_| AndroidSyncStatus::RegistryPoisoned)?;
    registry
        .estimators
        .remove(&handle)
        .map(|_| ())
        .ok_or(AndroidSyncStatus::InvalidHandle)
}

fn exchange_from_jni(
    t1_local_send_ms: i64,
    t2_host_receive_ms: i64,
    t3_host_send_ms: i64,
    t4_local_receive_ms: i64,
) -> Result<FfiSyncExchange, AndroidSyncStatus> {
    Ok(FfiSyncExchange {
        t1_local_send_ms: u64::try_from(t1_local_send_ms)
            .map_err(|_| AndroidSyncStatus::NegativeTimestamp)?,
        t2_host_receive_ms: u64::try_from(t2_host_receive_ms)
            .map_err(|_| AndroidSyncStatus::NegativeTimestamp)?,
        t3_host_send_ms: u64::try_from(t3_host_send_ms)
            .map_err(|_| AndroidSyncStatus::NegativeTimestamp)?,
        t4_local_receive_ms: u64::try_from(t4_local_receive_ms)
            .map_err(|_| AndroidSyncStatus::NegativeTimestamp)?,
    })
}

fn observe_estimator(
    handle: i64,
    exchange: FfiSyncExchange,
) -> Result<FfiSyncObservation, AndroidSyncStatus> {
    with_estimator(handle, |estimator| {
        estimator.observe(exchange).map_err(AndroidSyncStatus::from)
    })
}

fn last_observation(handle: i64) -> Result<FfiSyncObservation, AndroidSyncStatus> {
    with_estimator(handle, |estimator| {
        estimator
            .last_observation()
            .ok_or(AndroidSyncStatus::NoObservation)
    })
}

fn snapshot(handle: i64) -> Result<FfiSyncSnapshot, AndroidSyncStatus> {
    with_estimator(handle, |estimator| {
        estimator.snapshot().map_err(AndroidSyncStatus::from)
    })
}

fn f64_bits(value: f64) -> i64 {
    i64::from_ne_bytes(value.to_bits().to_ne_bytes())
}

fn invalid_f64_bits() -> i64 {
    f64_bits(f64::NAN)
}

fn status_or_error(result: Result<(), AndroidSyncStatus>) -> i32 {
    match result {
        Ok(()) => AndroidSyncStatus::Accepted.code(),
        Err(status) => status.code(),
    }
}

/// Returns the stable ABI contract version exposed to non-Rust callers.
#[must_use]
#[unsafe(no_mangle)]
pub extern "C" fn silent_disco_core_abi_version() -> u32 {
    u32::from(CORE_ABI_VERSION)
}

/// JNI entry point used only by the Android platform bridge and smoke tests.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeAbiVersion(
    _environment: *mut c_void,
    _receiver: *mut c_void,
) -> i32 {
    i32::from(CORE_ABI_VERSION)
}

/// Creates a Rust-owned synchronization estimator and returns a positive handle.
/// Negative values are stable error statuses and zero is never a valid handle.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorCreate(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    max_samples: i32,
    max_accepted_rtt_ms: f64,
) -> i64 {
    create_estimator(max_samples, max_accepted_rtt_ms)
        .unwrap_or_else(|status| i64::from(status.code()))
}

/// Processes one four-timestamp exchange. Zero means accepted, one means a
/// valid high-RTT rejection, and negative values are explicit failures.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorObserve(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
    t1_local_send_ms: i64,
    t2_host_receive_ms: i64,
    t3_host_send_ms: i64,
    t4_local_receive_ms: i64,
) -> i32 {
    let result = exchange_from_jni(
        t1_local_send_ms,
        t2_host_receive_ms,
        t3_host_send_ms,
        t4_local_receive_ms,
    )
    .and_then(|exchange| observe_estimator(handle, exchange));
    match result {
        Ok(observation) if observation.accepted => AndroidSyncStatus::Accepted.code(),
        Ok(_) => AndroidSyncStatus::Rejected.code(),
        Err(status) => status.code(),
    }
}

/// Validates that a latest observation exists before its primitive fields are read.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastStatus(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i32 {
    match last_observation(handle) {
        Ok(_) => AndroidSyncStatus::Accepted.code(),
        Err(status) => status.code(),
    }
}

/// Returns raw IEEE-754 bits after `nativeSyncEstimatorLastStatus` succeeds.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastRttBits(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i64 {
    last_observation(handle).map_or_else(
        |_| invalid_f64_bits(),
        |observation| f64_bits(observation.round_trip_time_ms),
    )
}

/// Returns raw IEEE-754 bits after `nativeSyncEstimatorLastStatus` succeeds.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastOffsetBits(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i64 {
    last_observation(handle).map_or_else(
        |_| invalid_f64_bits(),
        |observation| f64_bits(observation.offset_ms),
    )
}

/// Validates that the estimator snapshot can be represented by the FFI record.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotStatus(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i32 {
    match snapshot(handle) {
        Ok(_) => AndroidSyncStatus::Accepted.code(),
        Err(status) => status.code(),
    }
}

macro_rules! snapshot_f64_export {
    ($function_name:ident, $field:ident) => {
        /// Returns raw IEEE-754 bits after snapshot status validation succeeds.
        #[must_use]
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $function_name(
            _environment: *mut c_void,
            _receiver: *mut c_void,
            handle: i64,
        ) -> i64 {
            snapshot(handle)
                .map(|value| f64_bits(value.$field))
                .unwrap_or_else(|_| invalid_f64_bits())
        }
    };
}

snapshot_f64_export!(
    Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotOffsetBits,
    offset_ms
);
snapshot_f64_export!(
    Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotRttBits,
    round_trip_time_ms
);
snapshot_f64_export!(
    Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotJitterBits,
    jitter_ms
);
snapshot_f64_export!(
    Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotSkewBits,
    skew_ppm
);

/// Returns the stable synchronization-confidence code after snapshot validation.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotConfidenceCode(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i32 {
    snapshot(handle).map_or_else(
        AndroidSyncStatus::code,
        |value| i32::from(value.confidence_code),
    )
}

/// Returns the accepted sample count after snapshot validation.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotAcceptedCount(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i32 {
    snapshot(handle)
        .and_then(|value| {
            i32::try_from(value.accepted_sample_count)
                .map_err(|_| AndroidSyncStatus::SampleCountOverflow)
        })
        .unwrap_or_else(AndroidSyncStatus::code)
}

/// Destroys one estimator. Destroying an unknown or already-destroyed handle is
/// an explicit invalid-handle failure rather than a synthetic success.
#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorDestroy(
    _environment: *mut c_void,
    _receiver: *mut c_void,
    handle: i64,
) -> i32 {
    status_or_error(destroy_estimator(handle))
}

#[cfg(test)]
mod tests {
    use super::{
        AndroidSyncStatus,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorCreate,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorDestroy,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastOffsetBits,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastRttBits,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastStatus,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorObserve,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotAcceptedCount,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotConfidenceCode,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotOffsetBits,
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotStatus,
    };

    const TOLERANCE: f64 = 1.0e-9;

    fn decode_f64(bits: i64) -> f64 {
        f64::from_bits(u64::from_ne_bytes(bits.to_ne_bytes()))
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= TOLERANCE,
            "expected {actual} to be within {TOLERANCE} of {expected}"
        );
    }

    fn create_test_estimator() -> i64 {
        let handle = Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorCreate(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            12,
            200.0,
        );
        assert!(handle > 0);
        handle
    }

    fn observe_test_sample(handle: i64, t1: i64, t2: i64, t3: i64, t4: i64) -> i32 {
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorObserve(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            handle,
            t1,
            t2,
            t3,
            t4,
        )
    }

    fn destroy_test_estimator(handle: i64) -> i32 {
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorDestroy(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            handle,
        )
    }

    fn latest_status(handle: i64) -> i32 {
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastStatus(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            handle,
        )
    }

    fn latest_rtt(handle: i64) -> f64 {
        decode_f64(
            Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastRttBits(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                handle,
            ),
        )
    }

    fn latest_offset(handle: i64) -> f64 {
        decode_f64(
            Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorLastOffsetBits(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                handle,
            ),
        )
    }

    fn snapshot_status(handle: i64) -> i32 {
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotStatus(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            handle,
        )
    }

    fn snapshot_offset(handle: i64) -> f64 {
        decode_f64(
            Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotOffsetBits(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                handle,
            ),
        )
    }

    fn snapshot_confidence(handle: i64) -> i32 {
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotConfidenceCode(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            handle,
        )
    }

    fn snapshot_accepted_count(handle: i64) -> i32 {
        Java_com_ekkus_silentdisco_core_rust_RustCoreBridge_nativeSyncEstimatorSnapshotAcceptedCount(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            handle,
        )
    }

    #[test]
    fn static_jni_exports_drive_the_rust_estimator_without_fallbacks() {
        let handle = create_test_estimator();
        assert_eq!(
            observe_test_sample(handle, 1_000, 1_012, 1_014, 1_026),
            AndroidSyncStatus::Accepted.code()
        );
        assert_eq!(latest_status(handle), AndroidSyncStatus::Accepted.code());
        assert_close(latest_rtt(handle), 24.0);
        assert_close(latest_offset(handle), 0.0);
        assert_eq!(
            observe_test_sample(handle, 2_000, 2_015, 2_017, 2_022),
            AndroidSyncStatus::Accepted.code()
        );
        assert_eq!(
            observe_test_sample(handle, 3_000, 3_100, 3_101, 3_400),
            AndroidSyncStatus::Rejected.code()
        );
        assert_eq!(snapshot_status(handle), AndroidSyncStatus::Accepted.code());
        assert_close(snapshot_offset(handle), 5.0);
        assert_eq!(snapshot_confidence(handle), 5);
        assert_eq!(snapshot_accepted_count(handle), 2);
        assert_eq!(destroy_test_estimator(handle), AndroidSyncStatus::Accepted.code());
        assert_eq!(destroy_test_estimator(handle), AndroidSyncStatus::InvalidHandle.code());
    }

    #[test]
    fn static_jni_export_rejects_negative_timestamps_explicitly() {
        let handle = create_test_estimator();
        assert_eq!(
            observe_test_sample(handle, -1, 0, 0, 0),
            AndroidSyncStatus::NegativeTimestamp.code()
        );
        assert_eq!(destroy_test_estimator(handle), AndroidSyncStatus::Accepted.code());
    }
}
