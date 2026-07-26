from pathlib import Path

path = Path("rust/silent-disco-ffi/src/android_database_abi.rs")
text = path.read_text()
text = text.replace("    fs,\n", "")
text = text.replace(
    "        StorageErrorKind, StoredSettings, TrustedDevice, LEGACY_ANDROID_IMPORT_VERSION,\n",
    "        StorageErrorKind, StoredSettings, TrustedDevice,\n",
)
old_function = '''fn settings_from_jni(
    sync_sample_window: jint,
    sync_cadence_ms: jlong,
    startup_buffer_ms: jlong,
    late_packet_threshold_ms: jlong,
    hard_resync_threshold_ms: jlong,
    sync_drift_threshold_ms: jdouble,
    scan_window_ms: jlong,
    updated_at_ms: jlong,
) -> Result<StoredSettings, AndroidDatabaseStatus> {
    Ok(StoredSettings {
        tuning: TuningSettings {
            sync_sample_window: u16::try_from(sync_sample_window)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            sync_cadence_ms: u64::try_from(sync_cadence_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            startup_buffer_ms: u64::try_from(startup_buffer_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            late_packet_threshold_ms: u64::try_from(late_packet_threshold_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            hard_resync_threshold_ms: u64::try_from(hard_resync_threshold_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
            sync_drift_threshold_ms,
            scan_window_ms: u64::try_from(scan_window_ms)
                .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
        },
        updated_at_ms: u64::try_from(updated_at_ms)
            .map_err(|_| AndroidDatabaseStatus::InvalidArgument)?,
    })
}
'''
new_function = '''struct JniSettingsFields {
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
'''
if text.count(old_function) != 1:
    raise SystemExit("settings_from_jni function shape changed")
text = text.replace(old_function, new_function)
old_call = '''settings_from_jni(
            sync_sample_window,
            sync_cadence_ms,
            startup_buffer_ms,
            late_packet_threshold_ms,
            hard_resync_threshold_ms,
            sync_drift_threshold_ms,
            scan_window_ms,
            imported_at_ms,
        )'''
new_call = '''settings_from_jni(JniSettingsFields {
            sync_sample_window,
            sync_cadence_ms,
            startup_buffer_ms,
            late_packet_threshold_ms,
            hard_resync_threshold_ms,
            sync_drift_threshold_ms,
            scan_window_ms,
            updated_at_ms: imported_at_ms,
        })'''
if text.count(old_call) != 1:
    raise SystemExit("legacy settings call shape changed")
text = text.replace(old_call, new_call)
old_save_call = '''settings_from_jni(
        sync_sample_window,
        sync_cadence_ms,
        startup_buffer_ms,
        late_packet_threshold_ms,
        hard_resync_threshold_ms,
        sync_drift_threshold_ms,
        scan_window_ms,
        updated_at_ms,
    )'''
new_save_call = '''settings_from_jni(JniSettingsFields {
        sync_sample_window,
        sync_cadence_ms,
        startup_buffer_ms,
        late_packet_threshold_ms,
        hard_resync_threshold_ms,
        sync_drift_threshold_ms,
        scan_window_ms,
        updated_at_ms,
    })'''
if text.count(old_save_call) != 1:
    raise SystemExit("save settings call shape changed")
text = text.replace(old_save_call, new_save_call)
text = text.replace(
    "        entry.cached_settings = settings.clone();",
    "        entry.cached_settings.clone_from(&settings);",
)
text = text.replace(
    '''    cached_settings(handle)
        .map(|settings| i32::from(settings.tuning.sync_sample_window))
        .unwrap_or(-1)
''',
    '''    cached_settings(handle).map_or(-1, |settings| {
        i32::from(settings.tuning.sync_sample_window)
    })
''',
)
text = text.replace(
    '''    cached_settings(handle)
        .map(|settings| i64::from_ne_bytes(settings.tuning.sync_drift_threshold_ms.to_bits().to_ne_bytes()))
        .unwrap_or_else(|_| i64::from_ne_bytes(f64::NAN.to_bits().to_ne_bytes()))
''',
    '''    cached_settings(handle).map_or_else(
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
''',
)
text = text.replace(
    "    use std::{fs, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};",
    "    use std::{fs, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};",
)
text = text.replace(
    "            LegacyAndroidImport, LegacyImportOutcome, StoredSettings,\n            LEGACY_ANDROID_IMPORT_VERSION,\n",
    "            LegacyAndroidImport, LegacyImportOutcome, StoredSettings,\n            LEGACY_ANDROID_IMPORT_VERSION,\n",
)
path.write_text(text)
