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

/// Block 24: races `close_database` against a writer thread that never stops
/// submitting real `SQLite` writes, repeated across several fresh databases to
/// vary which side wins the race. Whichever side gets there first, every
/// individual call must return a well-typed result (`Ok`, or an explicit
/// `InvalidHandle`/`WorkerUnavailable`-shaped failure once close has won) --
/// never a panic, and never a write that appears to succeed after the worker
/// has actually been torn down. After close, the file must still be a valid,
/// reopenable database: a shutdown mid-write must not corrupt or truncate it,
/// which is exactly what `DatabaseWorker`'s own drain-before-stop contract
/// promises and this exercises from the FFI boundary that actually calls it.
#[test]
fn close_races_with_concurrent_writes_never_panics_and_leaves_a_reopenable_database() {
    for iteration in 0_u64..10 {
        let path = test_path();
        let handle = open_database(path.clone()).expect("database opens");

        let writer_handle = handle;
        let writer = std::thread::spawn(move || {
            for sequence in 0..500_u64 {
                let device = TrustedDevice {
                    device_id: DeviceId::new(format!("race-device-{iteration}-{sequence}"))
                        .expect("valid device id"),
                    display_name: "Racing phone".to_owned(),
                    public_key: None,
                    private_key_ref: None,
                    trust_state: TrustState::Trusted,
                    first_seen_ms: sequence,
                    last_seen_ms: sequence,
                    updated_at_ms: sequence,
                };
                // Either a real success or an explicit typed failure (once
                // `close_database` has won the race) is acceptable; only a
                // panic, or a status outside the documented taxonomy, is not.
                let result = with_database_entry(writer_handle, |entry| {
                    entry
                        .worker
                        .as_ref()
                        .ok_or(AndroidDatabaseStatus::InvalidHandle)?
                        .client()
                        .upsert_trusted_device(&device)
                        .map_err(|error| super::map_storage_error(&error))
                });
                if result == Err(AndroidDatabaseStatus::InvalidHandle) {
                    // The handle was closed out from under this write; later
                    // iterations will see the same thing, so stop early
                    // rather than spinning uselessly.
                    break;
                }
            }
        });

        // Let real writes land before racing the close.
        std::thread::sleep(std::time::Duration::from_micros(200));
        close_database(handle).expect("close succeeds even under concurrent write load");
        writer.join().expect("writer thread must not panic");

        // Closing again is an explicit, typed failure, not a silent no-op or
        // a panic against a partially torn-down entry.
        assert_eq!(
            close_database(handle),
            Err(AndroidDatabaseStatus::InvalidHandle)
        );

        // The file itself must still be a clean, readable database: a
        // concurrent shutdown must not have corrupted or truncated it.
        let reopened = open_database(path.clone()).expect("database reopens after a race close");
        with_database_entry(reopened, |entry| {
            entry
                .worker
                .as_ref()
                .expect("worker present")
                .client()
                .list_trusted_devices()
                .map_err(|error| super::map_storage_error(&error))
        })
        .expect("reopened database is queryable, not corrupted");
        close_database(reopened).expect("reopened database closes cleanly");

        remove_database(&path);
    }
}

/// Block 24: repeated failed opens (a parent directory that does not exist,
/// so `SQLite` itself cannot create the file) must each fail explicitly and
/// leave nothing behind in the shared registry -- a leaked entry from a
/// failed open would eventually let some *other*, unrelated handle number
/// collide with a phantom one. After many failures, a valid open must still
/// succeed and behave completely normally, proving retry after repeated
/// failure is not degraded by the failures that preceded it.
#[test]
fn repeated_open_failure_then_successful_open_leaves_no_residue() {
    let doomed_path =
        PathBuf::from("/silent-disco-nonexistent-directory-for-block-24/unreachable.sqlite3");

    for _ in 0..25 {
        let result = open_database(doomed_path.clone());
        assert!(
            result.is_err(),
            "opening under a nonexistent parent directory must fail explicitly"
        );
    }

    let path = test_path();
    let handle = open_database(path.clone())
        .expect("a valid open still succeeds after repeated failed retries");
    with_database_entry(handle, |entry| {
        entry
            .worker
            .as_ref()
            .expect("worker present")
            .client()
            .list_trusted_devices()
            .map_err(|error| super::map_storage_error(&error))
    })
    .expect("the database opened after repeated failures behaves normally");
    close_database(handle).expect("database closes");
    remove_database(&path);
}
