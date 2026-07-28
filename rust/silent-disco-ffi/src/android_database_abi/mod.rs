#![allow(unsafe_code)]

mod exports;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, btree_map::Entry},
    path::PathBuf,
    ptr::null_mut,
    sync::{Arc, Mutex, OnceLock},
};

use jni::{
    JNIEnv,
    objects::{JObject, JObjectArray, JString},
    sys::{jdouble, jint, jlong, jstring},
};
use silent_disco_core::{
    domain::{DeviceId, TrustState, TuningSettings},
    storage::{
        DatabaseConfig, DatabaseWorker, LegacyAndroidImport, LegacyImportOutcome, StorageError,
        StorageErrorKind, StoredSettings, TrustedDevice,
    },
};

const MAX_JNI_HANDLE: u64 = i64::MAX as u64;
const MAX_LEGACY_TRUSTED_DEVICES: i32 = 256;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AndroidDatabaseStatus {
    Success = 0,
    NotFound = 1,
    AlreadyImported = 2,
    InvalidArgument = -100,
    InvalidHandle = -101,
    OpenFailed = -102,
    MigrationFailed = -103,
    IntegrityFailed = -104,
    Busy = -105,
    TransactionFailed = -106,
    QueryFailed = -107,
    ConstraintViolation = -108,
    CloseFailed = -109,
    WorkerUnavailable = -110,
    RegistryPoisoned = -111,
    HandleExhausted = -112,
    JniConversionFailed = -113,
    CachedSettingsUnavailable = -114,
    CachedTrustedDevicesUnavailable = -115,
}

impl AndroidDatabaseStatus {
    const fn code(self) -> i32 {
        self as i32
    }
}

fn map_storage_error(error: &StorageError) -> AndroidDatabaseStatus {
    match error.kind {
        StorageErrorKind::InvalidConfiguration => AndroidDatabaseStatus::InvalidArgument,
        StorageErrorKind::Open | StorageErrorKind::Pragma | StorageErrorKind::ThreadStart => {
            AndroidDatabaseStatus::OpenFailed
        }
        StorageErrorKind::Migration => AndroidDatabaseStatus::MigrationFailed,
        StorageErrorKind::Corruption | StorageErrorKind::WorkerPanicked => {
            AndroidDatabaseStatus::IntegrityFailed
        }
        StorageErrorKind::Busy | StorageErrorKind::QueueFull => AndroidDatabaseStatus::Busy,
        StorageErrorKind::Transaction => AndroidDatabaseStatus::TransactionFailed,
        StorageErrorKind::Query => AndroidDatabaseStatus::QueryFailed,
        StorageErrorKind::Constraint => AndroidDatabaseStatus::ConstraintViolation,
        StorageErrorKind::Close => AndroidDatabaseStatus::CloseFailed,
        StorageErrorKind::WorkerStopped
        | StorageErrorKind::ReplyDisconnected
        | StorageErrorKind::ShutdownInProgress => AndroidDatabaseStatus::WorkerUnavailable,
    }
}

struct DatabaseEntry {
    worker: Option<DatabaseWorker>,
    cached_settings: Option<StoredSettings>,
    cached_trusted_devices: Option<Vec<TrustedDevice>>,
}

#[derive(Default)]
struct DatabaseRegistry {
    next_handle: u64,
    entries: BTreeMap<u64, Arc<Mutex<DatabaseEntry>>>,
}

static DATABASE_REGISTRY: OnceLock<Mutex<DatabaseRegistry>> = OnceLock::new();

fn registry() -> &'static Mutex<DatabaseRegistry> {
    DATABASE_REGISTRY.get_or_init(|| {
        Mutex::new(DatabaseRegistry {
            next_handle: 1,
            entries: BTreeMap::new(),
        })
    })
}

fn open_database(path: PathBuf) -> Result<i64, AndroidDatabaseStatus> {
    let worker = DatabaseWorker::start(
        DatabaseConfig::new(path).map_err(|error| map_storage_error(&error))?,
    )
    .map_err(|error| map_storage_error(&error))?;
    let entry = Arc::new(Mutex::new(DatabaseEntry {
        worker: Some(worker),
        cached_settings: None,
        cached_trusted_devices: None,
    }));
    let mut registry = registry()
        .lock()
        .map_err(|_| AndroidDatabaseStatus::RegistryPoisoned)?;
    let handle = registry.next_handle;
    if handle == 0 || handle > MAX_JNI_HANDLE {
        return Err(AndroidDatabaseStatus::HandleExhausted);
    }
    let next_handle = handle
        .checked_add(1)
        .ok_or(AndroidDatabaseStatus::HandleExhausted)?;
    match registry.entries.entry(handle) {
        Entry::Vacant(slot) => {
            slot.insert(entry);
        }
        Entry::Occupied(_) => return Err(AndroidDatabaseStatus::HandleExhausted),
    }
    registry.next_handle = next_handle;
    i64::try_from(handle).map_err(|_| AndroidDatabaseStatus::HandleExhausted)
}

fn entry_for_handle(handle: i64) -> Result<Arc<Mutex<DatabaseEntry>>, AndroidDatabaseStatus> {
    let handle = u64::try_from(handle).map_err(|_| AndroidDatabaseStatus::InvalidHandle)?;
    if handle == 0 {
        return Err(AndroidDatabaseStatus::InvalidHandle);
    }
    registry()
        .lock()
        .map_err(|_| AndroidDatabaseStatus::RegistryPoisoned)?
        .entries
        .get(&handle)
        .cloned()
        .ok_or(AndroidDatabaseStatus::InvalidHandle)
}

fn with_database_entry<T>(
    handle: i64,
    action: impl FnOnce(&mut DatabaseEntry) -> Result<T, AndroidDatabaseStatus>,
) -> Result<T, AndroidDatabaseStatus> {
    let entry = entry_for_handle(handle)?;
    let mut entry = entry
        .lock()
        .map_err(|_| AndroidDatabaseStatus::RegistryPoisoned)?;
    if entry.worker.is_none() {
        return Err(AndroidDatabaseStatus::InvalidHandle);
    }
    action(&mut entry)
}

fn load_trusted_devices(handle: i64) -> Result<(), AndroidDatabaseStatus> {
    with_database_entry(handle, |entry| {
        let devices = entry
            .worker
            .as_ref()
            .ok_or(AndroidDatabaseStatus::InvalidHandle)?
            .client()
            .list_trusted_devices()
            .map_err(|error| map_storage_error(&error))?;
        entry.cached_trusted_devices = Some(devices);
        Ok(())
    })
}

fn cached_trusted_devices(handle: i64) -> Result<Vec<TrustedDevice>, AndroidDatabaseStatus> {
    with_database_entry(handle, |entry| {
        entry
            .cached_trusted_devices
            .clone()
            .ok_or(AndroidDatabaseStatus::CachedTrustedDevicesUnavailable)
    })
}

fn cached_trusted_device(handle: i64, index: jint) -> Result<TrustedDevice, AndroidDatabaseStatus> {
    let index = usize::try_from(index).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?;
    cached_trusted_devices(handle)?
        .get(index)
        .cloned()
        .ok_or(AndroidDatabaseStatus::InvalidArgument)
}

fn delete_trusted_device(handle: i64, device_id: &DeviceId) -> Result<bool, AndroidDatabaseStatus> {
    with_database_entry(handle, |entry| {
        let deleted = entry
            .worker
            .as_ref()
            .ok_or(AndroidDatabaseStatus::InvalidHandle)?
            .client()
            .delete_trusted_device(device_id)
            .map_err(|error| map_storage_error(&error))?;
        entry.cached_trusted_devices = None;
        Ok(deleted)
    })
}

fn close_database(handle: i64) -> Result<(), AndroidDatabaseStatus> {
    let handle = u64::try_from(handle).map_err(|_| AndroidDatabaseStatus::InvalidHandle)?;
    if handle == 0 {
        return Err(AndroidDatabaseStatus::InvalidHandle);
    }
    let entry = registry()
        .lock()
        .map_err(|_| AndroidDatabaseStatus::RegistryPoisoned)?
        .entries
        .remove(&handle)
        .ok_or(AndroidDatabaseStatus::InvalidHandle)?;
    let worker = entry
        .lock()
        .map_err(|_| AndroidDatabaseStatus::RegistryPoisoned)?
        .worker
        .take()
        .ok_or(AndroidDatabaseStatus::InvalidHandle)?;
    worker
        .stop_and_join()
        .map_err(|error| map_storage_error(&error))
}

fn java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<String, AndroidDatabaseStatus> {
    env.get_string(value)
        .map(Into::into)
        .map_err(|_| AndroidDatabaseStatus::JniConversionFailed)
}

fn java_string_array(
    env: &mut JNIEnv<'_>,
    values: &JObjectArray<'_>,
) -> Result<Vec<String>, AndroidDatabaseStatus> {
    let length = env
        .get_array_length(values)
        .map_err(|_| AndroidDatabaseStatus::JniConversionFailed)?;
    if !(0..=MAX_LEGACY_TRUSTED_DEVICES).contains(&length) {
        return Err(AndroidDatabaseStatus::InvalidArgument);
    }
    let mut decoded = Vec::with_capacity(
        usize::try_from(length).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
    );
    for index in 0..length {
        let object = env
            .get_object_array_element(values, index)
            .map_err(|_| AndroidDatabaseStatus::JniConversionFailed)?;
        let string = JString::from(object);
        decoded.push(java_string(env, &string)?);
        env.delete_local_ref(string)
            .map_err(|_| AndroidDatabaseStatus::JniConversionFailed)?;
    }
    Ok(decoded)
}

#[derive(Clone, Copy)]
struct JniSettingsFields {
    sync_sample_window: jint,
    sync_cadence_ms: jlong,
    startup_buffer_ms: jlong,
    late_packet_threshold_ms: jlong,
    hard_resync_threshold_ms: jlong,
    sync_drift_threshold_ms: jdouble,
    scan_window_ms: jlong,
    updated_at_ms: jlong,
}

fn settings_from_jni(fields: JniSettingsFields) -> Result<StoredSettings, AndroidDatabaseStatus> {
    Ok(StoredSettings {
        tuning: TuningSettings {
            sync_sample_window: u16::try_from(fields.sync_sample_window)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            sync_cadence_ms: u64::try_from(fields.sync_cadence_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            startup_buffer_ms: u64::try_from(fields.startup_buffer_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            late_packet_threshold_ms: u64::try_from(fields.late_packet_threshold_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            hard_resync_threshold_ms: u64::try_from(fields.hard_resync_threshold_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            sync_drift_threshold_ms: fields.sync_drift_threshold_ms,
            scan_window_ms: u64::try_from(fields.scan_window_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
        },
        updated_at_ms: u64::try_from(fields.updated_at_ms)
            .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
    })
}

fn status_code(result: Result<(), AndroidDatabaseStatus>) -> jint {
    result.map_or_else(AndroidDatabaseStatus::code, |()| {
        AndroidDatabaseStatus::Success.code()
    })
}

fn cached_settings(handle: jlong) -> Result<StoredSettings, AndroidDatabaseStatus> {
    with_database_entry(handle, |entry| {
        entry
            .cached_settings
            .clone()
            .ok_or(AndroidDatabaseStatus::CachedSettingsUnavailable)
    })
}

macro_rules! cached_jlong_export {
    ($function_name:ident, $field:expr) => {
        #[must_use]
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $function_name(
            _env: JNIEnv<'_>,
            _receiver: JObject<'_>,
            handle: jlong,
        ) -> jlong {
            cached_settings(handle)
                .and_then(|settings| {
                    i64::try_from($field(&settings))
                        .map_err(|_| AndroidDatabaseStatus::InvalidArgument)
                })
                .unwrap_or(-1)
        }
    };
}

cached_jlong_export!(
    Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedSyncCadenceMs,
    |settings: &StoredSettings| settings.tuning.sync_cadence_ms
);
cached_jlong_export!(
    Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedStartupBufferMs,
    |settings: &StoredSettings| settings.tuning.startup_buffer_ms
);
cached_jlong_export!(
    Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedLatePacketThresholdMs,
    |settings: &StoredSettings| settings.tuning.late_packet_threshold_ms
);
cached_jlong_export!(
    Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedHardResyncThresholdMs,
    |settings: &StoredSettings| settings.tuning.hard_resync_threshold_ms
);
cached_jlong_export!(
    Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedScanWindowMs,
    |settings: &StoredSettings| settings.tuning.scan_window_ms
);
cached_jlong_export!(
    Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedUpdatedAtMs,
    |settings: &StoredSettings| settings.updated_at_ms
);
