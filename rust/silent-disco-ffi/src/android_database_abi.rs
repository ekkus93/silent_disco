#![allow(unsafe_code)]

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

fn cached_trusted_device(
    handle: i64,
    index: jint,
) -> Result<TrustedDevice, AndroidDatabaseStatus> {
    let index = usize::try_from(index).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?;
    cached_trusted_devices(handle)?
        .get(index)
        .cloned()
        .ok_or(AndroidDatabaseStatus::InvalidArgument)
}

fn delete_trusted_device(
    handle: i64,
    device_id: DeviceId,
) -> Result<bool, AndroidDatabaseStatus> {
    with_database_entry(handle, |entry| {
        let deleted = entry
            .worker
            .as_ref()
            .ok_or(AndroidDatabaseStatus::InvalidHandle)?
            .client()
            .delete_trusted_device(&device_id)
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

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseOpen(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    path: JString<'_>,
) -> jlong {
    java_string(&mut env, &path)
        .map(PathBuf::from)
        .and_then(open_database)
        .unwrap_or_else(|status| i64::from(status.code()))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseClose(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    status_code(close_database(handle))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseImportLegacy(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    version: jint,
    sync_sample_window: jint,
    sync_cadence_ms: jlong,
    startup_buffer_ms: jlong,
    late_packet_threshold_ms: jlong,
    hard_resync_threshold_ms: jlong,
    sync_drift_threshold_ms: jdouble,
    scan_window_ms: jlong,
    trusted_device_ids: JObjectArray<'_>,
    imported_at_ms: jlong,
) -> jint {
    let result = (|| {
        let settings = settings_from_jni(JniSettingsFields {
            sync_sample_window,
            sync_cadence_ms,
            startup_buffer_ms,
            late_packet_threshold_ms,
            hard_resync_threshold_ms,
            sync_drift_threshold_ms,
            scan_window_ms,
            updated_at_ms: imported_at_ms,
        })?;
        let imported_at_ms =
            u64::try_from(imported_at_ms).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?;
        let trusted_devices = java_string_array(&mut env, &trusted_device_ids)?
            .into_iter()
            .map(|value| {
                let device_id =
                    DeviceId::new(value).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?;
                Ok(TrustedDevice {
                    display_name: device_id.as_str().to_owned(),
                    device_id,
                    public_key: None,
                    private_key_ref: None,
                    trust_state: TrustState::Trusted,
                    first_seen_ms: imported_at_ms,
                    last_seen_ms: imported_at_ms,
                    updated_at_ms: imported_at_ms,
                })
            })
            .collect::<Result<Vec<_>, AndroidDatabaseStatus>>()?;
        let value = LegacyAndroidImport {
            version: u32::try_from(version).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            settings,
            trusted_devices,
            imported_at_ms,
        };
        with_database_entry(handle, |entry| {
            let client = entry
                .worker
                .as_ref()
                .ok_or(AndroidDatabaseStatus::InvalidHandle)?
                .client();
            client
                .import_legacy_android(&value)
                .map_err(|error| map_storage_error(&error))
        })
    })();
    match result {
        Ok(LegacyImportOutcome::Imported) => AndroidDatabaseStatus::Success.code(),
        Ok(LegacyImportOutcome::AlreadyImported) => AndroidDatabaseStatus::AlreadyImported.code(),
        Err(status) => status.code(),
    }
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseLoadSettingsStatus(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    let result = with_database_entry(handle, |entry| {
        let client = entry
            .worker
            .as_ref()
            .ok_or(AndroidDatabaseStatus::InvalidHandle)?
            .client();
        let settings = client
            .load_settings()
            .map_err(|error| map_storage_error(&error))?;
        entry.cached_settings.clone_from(&settings);
        Ok(settings.is_some())
    });
    match result {
        Ok(true) => AndroidDatabaseStatus::Success.code(),
        Ok(false) => AndroidDatabaseStatus::NotFound.code(),
        Err(status) => status.code(),
    }
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

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedSyncSampleWindow(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    cached_settings(handle).map_or(-1, |settings| i32::from(settings.tuning.sync_sample_window))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedSyncDriftThresholdBits(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jlong {
    cached_settings(handle).map_or_else(
        |_| i64::from_ne_bytes(f64::NAN.to_bits().to_ne_bytes()),
        |settings| {
            i64::from_ne_bytes(
                settings
                    .tuning
                    .sync_drift_threshold_ms
                    .to_bits()
                    .to_ne_bytes(),
            )
        },
    )
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseSaveSettings(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    sync_sample_window: jint,
    sync_cadence_ms: jlong,
    startup_buffer_ms: jlong,
    late_packet_threshold_ms: jlong,
    hard_resync_threshold_ms: jlong,
    sync_drift_threshold_ms: jdouble,
    scan_window_ms: jlong,
    updated_at_ms: jlong,
) -> jint {
    let result = settings_from_jni(JniSettingsFields {
        sync_sample_window,
        sync_cadence_ms,
        startup_buffer_ms,
        late_packet_threshold_ms,
        hard_resync_threshold_ms,
        sync_drift_threshold_ms,
        scan_window_ms,
        updated_at_ms,
    })
    .and_then(|settings| {
        with_database_entry(handle, |entry| {
            let client = entry
                .worker
                .as_ref()
                .ok_or(AndroidDatabaseStatus::InvalidHandle)?
                .client();
            client
                .save_settings(&settings)
                .map_err(|error| map_storage_error(&error))?;
            entry.cached_settings = Some(settings);
            Ok(())
        })
    });
    status_code(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseUpsertTrusted(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    device_id: JString<'_>,
    display_name: JString<'_>,
    observed_at_ms: jlong,
) -> jint {
    let result = (|| {
        let device_id = DeviceId::new(java_string(&mut env, &device_id)?)
            .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?;
        let observed_at_ms =
            u64::try_from(observed_at_ms).map_err(|_| AndroidDatabaseStatus::InvalidArgument)?;
        let device = TrustedDevice {
            device_id,
            display_name: java_string(&mut env, &display_name)?,
            public_key: None,
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: observed_at_ms,
            last_seen_ms: observed_at_ms,
            updated_at_ms: observed_at_ms,
        };
        with_database_entry(handle, |entry| {
            entry
                .worker
                .as_ref()
                .ok_or(AndroidDatabaseStatus::InvalidHandle)?
                .client()
                .upsert_trusted_device(&device)
                .map_err(|error| map_storage_error(&error))?;
            entry.cached_trusted_devices = None;
            Ok(())
        })
    })();
    status_code(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseLoadTrustedDevicesStatus(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    status_code(load_trusted_devices(handle))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedTrustedDeviceCount(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    match cached_trusted_devices(handle).and_then(|devices| {
        i32::try_from(devices.len()).map_err(|_| AndroidDatabaseStatus::InvalidArgument)
    }) {
        Ok(count) => count,
        Err(status) => status.code(),
    }
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedTrustedDeviceId(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jstring {
    cached_trusted_device(handle, index)
        .and_then(|device| {
            env.new_string(device.device_id.as_str())
                .map_err(|_| AndroidDatabaseStatus::JniConversionFailed)
        })
        .map_or_else(|_| null_mut(), JString::into_raw)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedTrustedDisplayName(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jstring {
    cached_trusted_device(handle, index)
        .and_then(|device| {
            env.new_string(device.display_name)
                .map_err(|_| AndroidDatabaseStatus::JniConversionFailed)
        })
        .map_or_else(|_| null_mut(), JString::into_raw)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseCachedTrustedLastSeenMs(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jlong {
    match cached_trusted_device(handle, index).and_then(|device| {
        i64::try_from(device.last_seen_ms).map_err(|_| AndroidDatabaseStatus::InvalidArgument)
    }) {
        Ok(value) => value,
        Err(status) => i64::from(status.code()),
    }
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseDeleteTrusted(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    device_id: JString<'_>,
) -> jint {
    let result = java_string(&mut env, &device_id)
        .and_then(|value| DeviceId::new(value).map_err(|_| AndroidDatabaseStatus::InvalidArgument))
        .and_then(|device_id| delete_trusted_device(handle, device_id));
    match result {
        Ok(true) => AndroidDatabaseStatus::Success.code(),
        Ok(false) => AndroidDatabaseStatus::NotFound.code(),
        Err(status) => status.code(),
    }
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustDatabaseBridge_nativeDatabaseIsTrusted(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    device_id: JString<'_>,
) -> jint {
    let result = java_string(&mut env, &device_id)
        .and_then(|value| DeviceId::new(value).map_err(|_| AndroidDatabaseStatus::InvalidArgument))
        .and_then(|device_id| {
            with_database_entry(handle, |entry| {
                entry
                    .worker
                    .as_ref()
                    .ok_or(AndroidDatabaseStatus::InvalidHandle)?
                    .client()
                    .get_trusted_device(&device_id)
                    .map(|device| {
                        device.is_some_and(|value| value.trust_state == TrustState::Trusted)
                    })
                    .map_err(|error| map_storage_error(&error))
            })
        });
    match result {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(status) => status.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AndroidDatabaseStatus, cached_trusted_device, cached_trusted_devices, close_database,
        delete_trusted_device, load_trusted_devices, open_database, with_database_entry,
    };
    use silent_disco_core::{
        domain::{DeviceId, TrustState, TuningSettings},
        storage::{
            LEGACY_ANDROID_IMPORT_VERSION, LegacyAndroidImport, LegacyImportOutcome, StoredSettings,
            TrustedDevice,
        },
    };
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_PATH: AtomicU64 = AtomicU64::new(1);

    fn test_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "silent-disco-ffi-database-{}-{}.sqlite3",
            std::process::id(),
            NEXT_PATH.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn remove_database(path: &PathBuf) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(PathBuf::from(format!("{}-wal", path.display())));
        let _ = fs::remove_file(PathBuf::from(format!("{}-shm", path.display())));
    }

    #[test]
    fn registry_opens_imports_loads_and_closes_database() {
        let path = test_path();
        let handle = open_database(path.clone()).expect("database opens");
        let value = LegacyAndroidImport {
            version: LEGACY_ANDROID_IMPORT_VERSION,
            settings: StoredSettings {
                tuning: TuningSettings::default(),
                updated_at_ms: 10,
            },
            trusted_devices: Vec::new(),
            imported_at_ms: 10,
        };
        let outcome = with_database_entry(handle, |entry| {
            entry
                .worker
                .as_ref()
                .expect("worker present")
                .client()
                .import_legacy_android(&value)
                .map_err(|error| super::map_storage_error(&error))
        });
        assert_eq!(outcome, Ok(LegacyImportOutcome::Imported));
        close_database(handle).expect("database closes");
        assert_eq!(
            close_database(handle),
            Err(AndroidDatabaseStatus::InvalidHandle)
        );
        remove_database(&path);
    }

    #[test]
    fn trusted_device_cache_lists_and_deletes_authoritatively() {
        let path = test_path();
        let handle = open_database(path.clone()).expect("database opens");
        let device = TrustedDevice {
            device_id: DeviceId::new("listener-cache").expect("valid device id"),
            display_name: "Listener phone".to_owned(),
            public_key: None,
            private_key_ref: None,
            trust_state: TrustState::Trusted,
            first_seen_ms: 10,
            last_seen_ms: 20,
            updated_at_ms: 20,
        };
        with_database_entry(handle, |entry| {
            entry
                .worker
                .as_ref()
                .expect("worker present")
                .client()
                .upsert_trusted_device(&device)
                .map_err(|error| super::map_storage_error(&error))
        })
        .expect("device stored");

        load_trusted_devices(handle).expect("trusted devices load");
        assert_eq!(cached_trusted_devices(handle), Ok(vec![device.clone()]));
        assert_eq!(cached_trusted_device(handle, 0), Ok(device.clone()));
        assert_eq!(
            cached_trusted_device(handle, 1),
            Err(AndroidDatabaseStatus::InvalidArgument)
        );

        assert_eq!(
            delete_trusted_device(handle, device.device_id.clone()),
            Ok(true)
        );
        assert_eq!(
            cached_trusted_devices(handle),
            Err(AndroidDatabaseStatus::CachedTrustedDevicesUnavailable)
        );
        load_trusted_devices(handle).expect("empty trusted devices reload");
        assert!(cached_trusted_devices(handle).expect("cache present").is_empty());

        close_database(handle).expect("database closes");
        remove_database(&path);
    }
}
