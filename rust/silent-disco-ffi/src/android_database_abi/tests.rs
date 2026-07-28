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

    assert_eq!(delete_trusted_device(handle, &device.device_id), Ok(true));
    assert_eq!(
        cached_trusted_devices(handle),
        Err(AndroidDatabaseStatus::CachedTrustedDevicesUnavailable)
    );
    load_trusted_devices(handle).expect("empty trusted devices reload");
    assert!(
        cached_trusted_devices(handle)
            .expect("cache present")
            .is_empty()
    );

    close_database(handle).expect("database closes");
    remove_database(&path);
}
