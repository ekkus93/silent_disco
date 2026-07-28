use super::{
    AndroidDatabaseStatus, DeviceId, JNIEnv, JObject, JObjectArray, JString, JniSettingsFields,
    LegacyAndroidImport, LegacyImportOutcome, PathBuf, TrustState, TrustedDevice, cached_settings,
    cached_trusted_device, cached_trusted_devices, close_database, delete_trusted_device,
    java_string, java_string_array, jdouble, jint, jlong, jstring, load_trusted_devices,
    map_storage_error, null_mut, open_database, settings_from_jni, status_code,
    with_database_entry,
};

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
    env: JNIEnv<'_>,
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
    env: JNIEnv<'_>,
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
        .and_then(|device_id| delete_trusted_device(handle, &device_id));
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
