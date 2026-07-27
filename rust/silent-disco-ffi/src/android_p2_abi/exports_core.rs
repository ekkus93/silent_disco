#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2Open(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    path: JString<'_>,
) -> jlong {
    java_string(&mut env, &path)
        .map(PathBuf::from)
        .and_then(open)
        .unwrap_or_else(|error| i64::from(error.code()))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2Close(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    status(close(handle))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2RecordSession(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    session_id: JString<'_>,
    role: jint,
    session_name: JString<'_>,
    host_name: JString<'_>,
    host_fingerprint: JString<'_>,
    started_at_ms: jlong,
    ended_at_ms: jlong,
    outcome: jint,
) -> jint {
    let result = (|| {
        let role = match role {
            1 => RecentSessionRole::Host,
            2 => RecentSessionRole::Listener,
            _ => return Err(Status::InvalidArgument),
        };
        let outcome = match outcome {
            1 => RecentSessionOutcome::Completed,
            2 => RecentSessionOutcome::Cancelled,
            3 => RecentSessionOutcome::Failed,
            _ => return Err(Status::InvalidArgument),
        };
        let value = RecentSessionRecord {
            session_id: java_string(&mut env, &session_id)?,
            role,
            session_name: java_string(&mut env, &session_name)?,
            host_name: java_string(&mut env, &host_name)?,
            host_fingerprint: optional_java_string(&mut env, &host_fingerprint)?,
            started_at_ms: u64::try_from(started_at_ms).map_err(|_| Status::InvalidArgument)?,
            ended_at_ms: u64::try_from(ended_at_ms).map_err(|_| Status::InvalidArgument)?,
            outcome,
        };
        with_entry(handle, |entry| {
            entry
                .store
                .record_session(&value)
                .map_err(|error| map_error(&error))
        })
    })();
    status(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ListRecentJson(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    now_ms: jlong,
    max_age_ms: jlong,
    limit: jint,
) -> jstring {
    let result = (|| {
        let now_ms = u64::try_from(now_ms).map_err(|_| Status::InvalidArgument)?;
        let max_age_ms = if max_age_ms == 0 {
            DEFAULT_RECENT_MAX_AGE_MS
        } else {
            u64::try_from(max_age_ms).map_err(|_| Status::InvalidArgument)?
        };
        let limit = u32::try_from(limit).map_err(|_| Status::InvalidArgument)?;
        with_entry(handle, |entry| {
            entry
                .store
                .list_recent_listener_sessions(now_ms, max_age_ms, limit)
                .map(|values| Value::Array(values.iter().map(recent_json).collect()))
                .map_err(|error| map_error(&error))
        })
    })();
    envelope_string(&mut env, result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2PrepareUnsignedQrJson(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    session_id: JString<'_>,
    session_name: JString<'_>,
    host_name: JString<'_>,
    public_key_der: JByteArray<'_>,
    approval_mode: JString<'_>,
    invite_code: JString<'_>,
    issued_at_ms: jlong,
    expires_at_ms: jlong,
    nonce: JString<'_>,
) -> jstring {
    let result = (|| {
        let input = QrInvitationInput {
            session_id: java_string(&mut env, &session_id)?,
            session_name: java_string(&mut env, &session_name)?,
            host_name: java_string(&mut env, &host_name)?,
            host_public_key_der: java_bytes(&mut env, &public_key_der)?,
            approval_mode: java_string(&mut env, &approval_mode)?,
            invite_code: optional_java_string(&mut env, &invite_code)?,
            issued_at_ms: u64::try_from(issued_at_ms).map_err(|_| Status::InvalidArgument)?,
            expires_at_ms: u64::try_from(expires_at_ms).map_err(|_| Status::InvalidArgument)?,
            nonce: java_string(&mut env, &nonce)?,
        };
        prepare_unsigned_qr(&input)
            .map(Value::String)
            .map_err(|error| map_error(&error))
    })();
    envelope_string(&mut env, result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2FinalizeQrJson(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    unsigned_json: JString<'_>,
    signature_base64url: JString<'_>,
) -> jstring {
    let result = java_string(&mut env, &unsigned_json).and_then(|unsigned| {
        java_string(&mut env, &signature_base64url).and_then(|signature| {
            finalize_qr(&unsigned, &signature)
                .map(Value::String)
                .map_err(|error| map_error(&error))
        })
    });
    envelope_string(&mut env, result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2FingerprintJson(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    public_key_der: JByteArray<'_>,
) -> jstring {
    let result = java_bytes(&mut env, &public_key_der)
        .map(|bytes| Value::String(public_key_fingerprint(&bytes)));
    envelope_string(&mut env, result)
}
