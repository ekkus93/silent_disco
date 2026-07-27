#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidateQrJson(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    payload: JString<'_>,
    now_ms: jlong,
) -> jstring {
    let result = java_string(&mut env, &payload).and_then(|payload| {
        let now_ms = u64::try_from(now_ms).map_err(|_| Status::InvalidArgument)?;
        with_entry(handle, |entry| {
            let value = entry
                .store
                .validate_and_consume_qr(&payload, now_ms)
                .map_err(|error| map_error(&error))?;
            let result = invitation_json(&value);
            entry.invitation = Some(value);
            Ok(result)
        })
    });
    envelope_string(&mut env, result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2TrustValidatedHost(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    verified_at_ms: jlong,
) -> jint {
    let result = (|| {
        let verified_at_ms = u64::try_from(verified_at_ms).map_err(|_| Status::InvalidArgument)?;
        with_entry(handle, |entry| {
            let invitation = entry.invitation.clone().ok_or(Status::CacheUnavailable)?;
            entry
                .store
                .trust_host(&TrustedHostRecord {
                    fingerprint: invitation.host_fingerprint,
                    display_name: invitation.host_name,
                    public_key_der: invitation.host_public_key_der,
                    first_verified_ms: verified_at_ms,
                    last_verified_ms: verified_at_ms,
                })
                .map_err(|error| map_error(&error))
        })
    })();
    status(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ListTrustedJson(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jstring {
    let result = with_entry(handle, |entry| {
        entry
            .store
            .list_trusted_hosts()
            .map(|values| Value::Array(values.iter().map(trusted_json).collect()))
            .map_err(|error| map_error(&error))
    });
    envelope_string(&mut env, result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2DeleteTrusted(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    fingerprint: JString<'_>,
) -> jint {
    let result = java_string(&mut env, &fingerprint).and_then(|fingerprint| {
        with_entry(handle, |entry| {
            entry
                .store
                .delete_trusted_host(&fingerprint)
                .map_err(|error| map_error(&error))
        })
    });
    match result {
        Ok(true) => Status::Success.code(),
        Ok(false) => Status::NotFound.code(),
        Err(error) => error.code(),
    }
}
