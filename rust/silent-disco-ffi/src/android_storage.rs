#![allow(unsafe_code)]
#![allow(clippy::too_many_lines)]

use std::{
    collections::{BTreeMap, btree_map::Entry},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Mutex, OnceLock},
};

use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::{jlong, jstring},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use silent_disco_core::{
    domain::{DeviceId, TrustState, TuningSettings},
    storage::{
        DatabaseClient, DatabaseConfig, DatabaseWorker, LegacyAndroidImport,
        LegacyImportDisposition, LegacyImportOutcome, StorageError, StoredSettings, TrustedDevice,
    },
};

const MAX_STORAGE_HANDLE: u64 = i64::MAX as u64;
const SERIALIZATION_FAILURE: &str = r#"{"ok":false,"result":null,"error":{"code":"bridge_serialization_failed","operation":"serialize_response","message":"Rust storage bridge could not serialize its response","retryable":false,"coreRemainsUsable":false,"schemaVersion":null}}"#;

struct StorageEntry {
    worker: DatabaseWorker,
    client: DatabaseClient,
}

struct StorageRegistry {
    next_handle: u64,
    entries: BTreeMap<u64, StorageEntry>,
}

impl Default for StorageRegistry {
    fn default() -> Self {
        Self {
            next_handle: 1,
            entries: BTreeMap::new(),
        }
    }
}

impl StorageRegistry {
    fn reserve_handle(&mut self) -> Result<u64, BridgeFailure> {
        let handle = self.next_handle;
        if handle == 0 || handle > MAX_STORAGE_HANDLE || self.entries.contains_key(&handle) {
            return Err(BridgeFailure::bridge(
                "bridge_handle_exhausted",
                "open_storage",
                "Rust storage handle space is exhausted",
                false,
                false,
            ));
        }
        self.next_handle = handle.checked_add(1).ok_or_else(|| {
            BridgeFailure::bridge(
                "bridge_handle_exhausted",
                "open_storage",
                "Rust storage handle space is exhausted",
                false,
                false,
            )
        })?;
        Ok(handle)
    }
}

static STORAGE_REGISTRY: OnceLock<Mutex<StorageRegistry>> = OnceLock::new();

fn storage_registry() -> &'static Mutex<StorageRegistry> {
    STORAGE_REGISTRY.get_or_init(|| Mutex::new(StorageRegistry::default()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeEnvelope<T> {
    ok: bool,
    result: Option<T>,
    error: Option<BridgeErrorDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeErrorDto {
    code: String,
    operation: String,
    message: String,
    retryable: bool,
    core_remains_usable: bool,
    schema_version: Option<u32>,
}

#[derive(Debug)]
struct BridgeFailure(BridgeErrorDto);

impl BridgeFailure {
    fn bridge(
        code: &str,
        operation: &str,
        message: impl Into<String>,
        retryable: bool,
        core_remains_usable: bool,
    ) -> Self {
        Self(BridgeErrorDto {
            code: code.to_owned(),
            operation: operation.to_owned(),
            message: message.into(),
            retryable,
            core_remains_usable,
            schema_version: None,
        })
    }

    fn invalid_request(operation: &str, message: impl Into<String>) -> Self {
        Self::bridge("bridge_invalid_request", operation, message, false, true)
    }

    fn invalid_handle(operation: &str) -> Self {
        Self::bridge(
            "bridge_invalid_handle",
            operation,
            "Rust storage handle is invalid or already closed",
            false,
            true,
        )
    }

    fn registry_poisoned(operation: &str) -> Self {
        Self::bridge(
            "bridge_registry_poisoned",
            operation,
            "Rust storage registry lock is poisoned",
            false,
            false,
        )
    }
}

impl From<StorageError> for BridgeFailure {
    fn from(error: StorageError) -> Self {
        Self(BridgeErrorDto {
            code: format!("storage_{}", error.kind.stable_name()),
            operation: error.operation.stable_name().to_owned(),
            message: error.message,
            retryable: error.retryable,
            core_remains_usable: error.core_remains_usable,
            schema_version: error.schema_version,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageOpenResultDto {
    handle: i64,
    schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsDto {
    sync_sample_window: u16,
    sync_cadence_ms: u64,
    startup_buffer_ms: u64,
    late_packet_threshold_ms: u64,
    hard_resync_threshold_ms: u64,
    sync_drift_threshold_ms: f64,
    scan_window_ms: u64,
    updated_at_ms: u64,
}

impl SettingsDto {
    fn into_model(self, operation: &str) -> Result<StoredSettings, BridgeFailure> {
        let model = StoredSettings {
            tuning: TuningSettings {
                sync_sample_window: self.sync_sample_window,
                sync_cadence_ms: self.sync_cadence_ms,
                startup_buffer_ms: self.startup_buffer_ms,
                late_packet_threshold_ms: self.late_packet_threshold_ms,
                hard_resync_threshold_ms: self.hard_resync_threshold_ms,
                sync_drift_threshold_ms: self.sync_drift_threshold_ms,
                scan_window_ms: self.scan_window_ms,
            },
            updated_at_ms: self.updated_at_ms,
        };
        model.validate().map_err(|error| {
            BridgeFailure::invalid_request(operation, format!("settings are invalid: {error}"))
        })?;
        Ok(model)
    }
}

impl From<StoredSettings> for SettingsDto {
    fn from(value: StoredSettings) -> Self {
        Self {
            sync_sample_window: value.tuning.sync_sample_window,
            sync_cadence_ms: value.tuning.sync_cadence_ms,
            startup_buffer_ms: value.tuning.startup_buffer_ms,
            late_packet_threshold_ms: value.tuning.late_packet_threshold_ms,
            hard_resync_threshold_ms: value.tuning.hard_resync_threshold_ms,
            sync_drift_threshold_ms: value.tuning.sync_drift_threshold_ms,
            scan_window_ms: value.tuning.scan_window_ms,
            updated_at_ms: value.updated_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedDeviceDto {
    device_id: String,
    display_name: String,
    public_key: Option<Vec<u8>>,
    private_key_ref: Option<String>,
    trust_state: String,
    first_seen_ms: u64,
    last_seen_ms: u64,
    updated_at_ms: u64,
}

impl TrustedDeviceDto {
    fn into_model(self, operation: &str) -> Result<TrustedDevice, BridgeFailure> {
        let device_id = DeviceId::new(self.device_id).map_err(|error| {
            BridgeFailure::invalid_request(
                operation,
                format!("device identifier is invalid: {error}"),
            )
        })?;
        let trust_state = TrustState::from_wire_name(&self.trust_state).map_err(|error| {
            BridgeFailure::invalid_request(operation, format!("trust state is invalid: {error}"))
        })?;
        let model = TrustedDevice {
            device_id,
            display_name: self.display_name,
            public_key: self.public_key,
            private_key_ref: self.private_key_ref,
            trust_state,
            first_seen_ms: self.first_seen_ms,
            last_seen_ms: self.last_seen_ms,
            updated_at_ms: self.updated_at_ms,
        };
        model.validate().map_err(|error| {
            BridgeFailure::invalid_request(
                operation,
                format!("trusted-device metadata is invalid: {error}"),
            )
        })?;
        Ok(model)
    }
}

impl From<TrustedDevice> for TrustedDeviceDto {
    fn from(value: TrustedDevice) -> Self {
        Self {
            device_id: value.device_id.as_str().to_owned(),
            display_name: value.display_name,
            public_key: value.public_key,
            private_key_ref: value.private_key_ref,
            trust_state: value.trust_state.wire_name().to_owned(),
            first_seen_ms: value.first_seen_ms,
            last_seen_ms: value.last_seen_ms,
            updated_at_ms: value.updated_at_ms,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyImportDto {
    version: u32,
    imported_at_ms: u64,
    settings: Option<SettingsDto>,
    trusted_devices: Vec<TrustedDeviceDto>,
}

impl LegacyImportDto {
    fn into_model(self) -> Result<LegacyAndroidImport, BridgeFailure> {
        let operation = "import_legacy_android_data";
        let settings = self
            .settings
            .map(|value| value.into_model(operation))
            .transpose()?;
        let trusted_devices = self
            .trusted_devices
            .into_iter()
            .map(|value| value.into_model(operation))
            .collect::<Result<Vec<_>, _>>()?;
        let model = LegacyAndroidImport {
            version: self.version,
            imported_at_ms: self.imported_at_ms,
            settings,
            trusted_devices,
        };
        model.validate().map_err(|error| {
            BridgeFailure::invalid_request(operation, format!("legacy import is invalid: {error}"))
        })?;
        Ok(model)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyImportOutcomeDto {
    disposition: &'static str,
    import_version: u32,
    completed_at_ms: u64,
    settings_imported: bool,
    trusted_device_count: u32,
}

impl From<LegacyImportOutcome> for LegacyImportOutcomeDto {
    fn from(value: LegacyImportOutcome) -> Self {
        Self {
            disposition: match value.disposition {
                LegacyImportDisposition::Imported => "imported",
                LegacyImportDisposition::AlreadyCompleted => "already_completed",
            },
            import_version: value.import_version,
            completed_at_ms: value.completed_at_ms,
            settings_imported: value.settings_imported,
            trusted_device_count: value.trusted_device_count,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteTrustedDeviceDto {
    device_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteTrustedDeviceResultDto {
    deleted: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CloseStorageResultDto {
    closed: bool,
}

fn execute<T: Serialize>(action: impl FnOnce() -> Result<T, BridgeFailure>) -> String {
    let envelope = match catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(result)) => BridgeEnvelope {
            ok: true,
            result: Some(result),
            error: None,
        },
        Ok(Err(error)) => BridgeEnvelope {
            ok: false,
            result: None,
            error: Some(error.0),
        },
        Err(_) => BridgeEnvelope {
            ok: false,
            result: None,
            error: Some(
                BridgeFailure::bridge(
                    "bridge_panic",
                    "ffi_boundary",
                    "Rust storage bridge contained an internal panic",
                    false,
                    false,
                )
                .0,
            ),
        },
    };
    serde_json::to_string(&envelope).unwrap_or_else(|_| SERIALIZATION_FAILURE.to_owned())
}

fn decode_request<T: DeserializeOwned>(json: &str, operation: &str) -> Result<T, BridgeFailure> {
    serde_json::from_str(json).map_err(|error| {
        BridgeFailure::invalid_request(operation, format!("request JSON is invalid: {error}"))
    })
}

fn open_storage(path: String) -> Result<StorageOpenResultDto, BridgeFailure> {
    let config = DatabaseConfig::new(path).map_err(BridgeFailure::from)?;
    let worker = DatabaseWorker::start(config).map_err(BridgeFailure::from)?;
    let client = worker.client();
    let schema_version = worker.initial_metadata().schema_version;
    let Ok(mut registry) = storage_registry().lock() else {
        let shutdown = worker.stop_and_join().map_err(BridgeFailure::from);
        return shutdown.and(Err(BridgeFailure::registry_poisoned("open_storage")));
    };
    let handle = match registry.reserve_handle() {
        Ok(handle) => handle,
        Err(error) => {
            drop(registry);
            worker.stop_and_join().map_err(BridgeFailure::from)?;
            return Err(error);
        }
    };
    match registry.entries.entry(handle) {
        Entry::Vacant(entry) => {
            entry.insert(StorageEntry { worker, client });
        }
        Entry::Occupied(_) => {
            drop(registry);
            worker.stop_and_join().map_err(BridgeFailure::from)?;
            return Err(BridgeFailure::bridge(
                "bridge_registry_corrupt",
                "open_storage",
                "Rust storage registry allocated a duplicate handle",
                false,
                false,
            ));
        }
    }
    Ok(StorageOpenResultDto {
        handle: i64::try_from(handle).map_err(|_| {
            BridgeFailure::bridge(
                "bridge_handle_exhausted",
                "open_storage",
                "Rust storage handle cannot be represented by JNI",
                false,
                false,
            )
        })?,
        schema_version,
    })
}

fn require_handle(handle: i64, operation: &str) -> Result<u64, BridgeFailure> {
    let handle = u64::try_from(handle).map_err(|_| BridgeFailure::invalid_handle(operation))?;
    if handle == 0 {
        return Err(BridgeFailure::invalid_handle(operation));
    }
    Ok(handle)
}

fn client_for(handle: i64, operation: &str) -> Result<DatabaseClient, BridgeFailure> {
    let handle = require_handle(handle, operation)?;
    let registry = storage_registry()
        .lock()
        .map_err(|_| BridgeFailure::registry_poisoned(operation))?;
    registry
        .entries
        .get(&handle)
        .map(|entry| entry.client.clone())
        .ok_or_else(|| BridgeFailure::invalid_handle(operation))
}

fn import_legacy(handle: i64, request_json: &str) -> Result<LegacyImportOutcomeDto, BridgeFailure> {
    let operation = "import_legacy_android_data";
    let import = decode_request::<LegacyImportDto>(request_json, operation)?.into_model()?;
    client_for(handle, operation)?
        .import_legacy_android_data(&import)
        .map(LegacyImportOutcomeDto::from)
        .map_err(BridgeFailure::from)
}

fn load_settings(handle: i64) -> Result<Option<SettingsDto>, BridgeFailure> {
    let operation = "load_settings";
    client_for(handle, operation)?
        .load_settings()
        .map(|value| value.map(SettingsDto::from))
        .map_err(BridgeFailure::from)
}

fn save_settings(handle: i64, request_json: &str) -> Result<SettingsDto, BridgeFailure> {
    let operation = "save_settings";
    let request = decode_request::<SettingsDto>(request_json, operation)?;
    let model = request.clone().into_model(operation)?;
    client_for(handle, operation)?
        .save_settings(&model)
        .map_err(BridgeFailure::from)?;
    Ok(request)
}

fn list_trusted_devices(handle: i64) -> Result<Vec<TrustedDeviceDto>, BridgeFailure> {
    let operation = "list_trusted_devices";
    client_for(handle, operation)?
        .list_trusted_devices()
        .map(|devices| devices.into_iter().map(TrustedDeviceDto::from).collect())
        .map_err(BridgeFailure::from)
}

fn upsert_trusted_device(
    handle: i64,
    request_json: &str,
) -> Result<TrustedDeviceDto, BridgeFailure> {
    let operation = "upsert_trusted_device";
    let request = decode_request::<TrustedDeviceDto>(request_json, operation)?;
    let model = request.clone().into_model(operation)?;
    client_for(handle, operation)?
        .upsert_trusted_device(&model)
        .map_err(BridgeFailure::from)?;
    Ok(request)
}

fn delete_trusted_device(
    handle: i64,
    request_json: &str,
) -> Result<DeleteTrustedDeviceResultDto, BridgeFailure> {
    let operation = "delete_trusted_device";
    let request = decode_request::<DeleteTrustedDeviceDto>(request_json, operation)?;
    let device_id = DeviceId::new(request.device_id).map_err(|error| {
        BridgeFailure::invalid_request(operation, format!("device identifier is invalid: {error}"))
    })?;
    let deleted = client_for(handle, operation)?
        .delete_trusted_device(&device_id)
        .map_err(BridgeFailure::from)?;
    Ok(DeleteTrustedDeviceResultDto { deleted })
}

fn close_storage(handle: i64) -> Result<CloseStorageResultDto, BridgeFailure> {
    let operation = "close_storage";
    let handle = require_handle(handle, operation)?;
    let entry = {
        let mut registry = storage_registry()
            .lock()
            .map_err(|_| BridgeFailure::registry_poisoned(operation))?;
        registry
            .entries
            .remove(&handle)
            .ok_or_else(|| BridgeFailure::invalid_handle(operation))?
    };
    entry.worker.stop_and_join().map_err(BridgeFailure::from)?;
    Ok(CloseStorageResultDto { closed: true })
}

fn read_jstring(
    environment: &mut JNIEnv<'_>,
    value: &JString<'_>,
    operation: &str,
) -> Result<String, BridgeFailure> {
    environment
        .get_string(value)
        .map(Into::into)
        .map_err(|error| {
            BridgeFailure::invalid_request(
                operation,
                format!("JNI string could not be read: {error}"),
            )
        })
}

fn write_response(environment: &mut JNIEnv<'_>, response: String) -> jstring {
    environment
        .new_string(response)
        .map_or(core::ptr::null_mut(), jni::objects::JString::into_raw)
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageOpen(
    mut environment: JNIEnv<'_>,
    _receiver: JClass<'_>,
    path: JString<'_>,
) -> jstring {
    let response =
        execute(|| read_jstring(&mut environment, &path, "open_storage").and_then(open_storage));
    write_response(&mut environment, response)
}

macro_rules! handle_only_export {
    ($function_name:ident, $operation:ident) => {
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $function_name(
            mut environment: JNIEnv<'_>,
            _receiver: JClass<'_>,
            handle: jlong,
        ) -> jstring {
            let response = execute(|| $operation(handle));
            write_response(&mut environment, response)
        }
    };
}

macro_rules! handle_json_export {
    ($function_name:ident, $operation_name:literal, $operation:ident) => {
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $function_name(
            mut environment: JNIEnv<'_>,
            _receiver: JClass<'_>,
            handle: jlong,
            request: JString<'_>,
        ) -> jstring {
            let response = execute(|| {
                read_jstring(&mut environment, &request, $operation_name)
                    .and_then(|json| $operation(handle, &json))
            });
            write_response(&mut environment, response)
        }
    };
}

handle_json_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageImportLegacy,
    "import_legacy_android_data",
    import_legacy
);
handle_only_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageLoadSettings,
    load_settings
);
handle_json_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageSaveSettings,
    "save_settings",
    save_settings
);
handle_only_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageListTrustedDevices,
    list_trusted_devices
);
handle_json_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageUpsertTrustedDevice,
    "upsert_trusted_device",
    upsert_trusted_device
);
handle_json_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageDeleteTrustedDevice,
    "delete_trusted_device",
    delete_trusted_device
);
handle_only_export!(
    Java_com_ekkus_silentdisco_core_rust_RustStorageNativeBridge_nativeStorageClose,
    close_storage
);

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use serde_json::Value;
    use silent_disco_core::storage::ANDROID_LEGACY_IMPORT_VERSION;

    use super::{
        LegacyImportDto, SettingsDto, TrustedDeviceDto, close_storage, decode_request, execute,
        import_legacy, list_trusted_devices, load_settings, open_storage, save_settings,
        upsert_trusted_device,
    };

    static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);

    struct TestPath(PathBuf);

    impl TestPath {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "silent-disco-ffi-{name}-{}-{sequence}.sqlite3",
                std::process::id()
            ));
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(format!("{}-wal", path.display()));
            let _ = fs::remove_file(format!("{}-shm", path.display()));
            Self(path)
        }
    }

    impl Drop for TestPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(format!("{}-wal", self.0.display()));
            let _ = fs::remove_file(format!("{}-shm", self.0.display()));
        }
    }

    fn settings(cadence: u64, timestamp: u64) -> SettingsDto {
        SettingsDto {
            sync_sample_window: 12,
            sync_cadence_ms: cadence,
            startup_buffer_ms: 400,
            late_packet_threshold_ms: 40,
            hard_resync_threshold_ms: 120,
            sync_drift_threshold_ms: 18.0,
            scan_window_ms: 3_000,
            updated_at_ms: timestamp,
        }
    }

    fn trusted_device(timestamp: u64) -> TrustedDeviceDto {
        TrustedDeviceDto {
            device_id: "ffi-device".to_owned(),
            display_name: "FFI Device".to_owned(),
            public_key: Some(vec![0, 127, 128, 255]),
            private_key_ref: None,
            trust_state: "trusted".to_owned(),
            first_seen_ms: timestamp,
            last_seen_ms: timestamp,
            updated_at_ms: timestamp,
        }
    }

    #[test]
    fn typed_bridge_imports_persists_and_reopens() {
        let path = TestPath::new("roundtrip");
        let opened = open_storage(path.0.to_string_lossy().into_owned()).expect("open storage");
        let import = serde_json::json!({
            "version": ANDROID_LEGACY_IMPORT_VERSION,
            "importedAtMs": 10_000,
            "settings": settings(2_250, 10_000),
            "trustedDevices": [trusted_device(10_000)],
        });
        let imported =
            import_legacy(opened.handle, &import.to_string()).expect("import legacy data");
        assert_eq!(imported.disposition, "imported");
        assert_eq!(
            load_settings(opened.handle).expect("load settings"),
            Some(settings(2_250, 10_000))
        );
        assert_eq!(
            list_trusted_devices(opened.handle).expect("list trusted devices"),
            vec![trusted_device(10_000)]
        );
        save_settings(
            opened.handle,
            &serde_json::to_string(&settings(3_000, 20_000)).expect("serialize settings"),
        )
        .expect("save settings");
        let mut updated_device = trusted_device(20_000);
        updated_device.first_seen_ms = 10_000;
        updated_device.display_name = "Updated FFI Device".to_owned();
        upsert_trusted_device(
            opened.handle,
            &serde_json::to_string(&updated_device).expect("serialize device"),
        )
        .expect("upsert trusted device");
        close_storage(opened.handle).expect("close storage");

        let reopened = open_storage(path.0.to_string_lossy().into_owned()).expect("reopen storage");
        assert_eq!(
            load_settings(reopened.handle).expect("reload settings"),
            Some(settings(3_000, 20_000))
        );
        assert_eq!(
            list_trusted_devices(reopened.handle).expect("reload trusted devices"),
            vec![updated_device]
        );
        close_storage(reopened.handle).expect("close reopened storage");
    }

    #[test]
    fn strict_json_and_error_envelope_reject_unknown_fields() {
        let error = decode_request::<LegacyImportDto>(
            r#"{"version":1,"importedAtMs":1,"settings":null,"trustedDevices":[],"unexpected":true}"#,
            "import_legacy_android_data",
        )
        .expect_err("unknown field must fail");
        assert_eq!(error.0.code, "bridge_invalid_request");

        let response = execute::<Value>(|| Err(error));
        let envelope: Value = serde_json::from_str(&response).expect("valid response JSON");
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["result"], Value::Null);
        assert_eq!(envelope["error"]["code"], "bridge_invalid_request");
        assert_eq!(envelope["error"]["coreRemainsUsable"], true);
    }
}
