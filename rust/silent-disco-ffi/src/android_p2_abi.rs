#![allow(unsafe_code)]

use std::{
    collections::BTreeMap,
    path::PathBuf,
    ptr::null_mut,
    sync::{Arc, Mutex, OnceLock},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use jni::{
    JNIEnv,
    objects::{JByteArray, JObject, JString},
    sys::{jint, jlong, jstring},
};
use serde_json::{Value, json};
use silent_disco_core::p2::{
    DEFAULT_RECENT_MAX_AGE_MS, P2Error, P2Store, QrInvitationInput, RecentSessionOutcome,
    RecentSessionRecord, RecentSessionRole, TrustedHostRecord, ValidatedQrInvitation, finalize_qr,
    prepare_unsigned_qr, public_key_fingerprint,
};

const MAX_HANDLE: u64 = i64::MAX as u64;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Success = 0,
    NotFound = 1,
    InvalidArgument = -200,
    InvalidHandle = -201,
    Storage = -202,
    InvalidQr = -203,
    ExpiredQr = -204,
    ReplayedQr = -205,
    InvalidSignature = -206,
    CacheUnavailable = -207,
    RegistryPoisoned = -208,
    Conversion = -209,
}

impl Status {
    const fn code(self) -> i32 {
        self as i32
    }
}

fn map_error(error: &P2Error) -> Status {
    match error {
        P2Error::InvalidArgument(_) => Status::InvalidArgument,
        P2Error::Storage(_) | P2Error::CorruptStore(_) => Status::Storage,
        P2Error::InvalidQr(_) => Status::InvalidQr,
        P2Error::ExpiredQr => Status::ExpiredQr,
        P2Error::ReplayedQr => Status::ReplayedQr,
        P2Error::InvalidSignature => Status::InvalidSignature,
    }
}

struct RegistryEntry {
    store: P2Store,
    invitation: Option<ValidatedQrInvitation>,
}

struct Registry {
    next_handle: u64,
    entries: BTreeMap<u64, Arc<Mutex<RegistryEntry>>>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next_handle: 1,
            entries: BTreeMap::new(),
        })
    })
}

fn open(path: PathBuf) -> Result<i64, Status> {
    let store = P2Store::open(path).map_err(|error| map_error(&error))?;
    let entry = Arc::new(Mutex::new(RegistryEntry {
        store,
        invitation: None,
    }));
    let mut registry = registry().lock().map_err(|_| Status::RegistryPoisoned)?;
    let handle = registry.next_handle;
    if handle == 0 || handle > MAX_HANDLE || registry.entries.contains_key(&handle) {
        return Err(Status::InvalidHandle);
    }
    registry.entries.insert(handle, entry);
    registry.next_handle = handle.checked_add(1).ok_or(Status::InvalidHandle)?;
    i64::try_from(handle).map_err(|_| Status::InvalidHandle)
}

fn entry(handle: jlong) -> Result<Arc<Mutex<RegistryEntry>>, Status> {
    let handle = u64::try_from(handle).map_err(|_| Status::InvalidHandle)?;
    if handle == 0 {
        return Err(Status::InvalidHandle);
    }
    registry()
        .lock()
        .map_err(|_| Status::RegistryPoisoned)?
        .entries
        .get(&handle)
        .cloned()
        .ok_or(Status::InvalidHandle)
}

fn with_entry<T>(
    handle: jlong,
    action: impl FnOnce(&mut RegistryEntry) -> Result<T, Status>,
) -> Result<T, Status> {
    let entry = entry(handle)?;
    let mut guard = entry.lock().map_err(|_| Status::RegistryPoisoned)?;
    action(&mut guard)
}

fn close(handle: jlong) -> Result<(), Status> {
    let handle = u64::try_from(handle).map_err(|_| Status::InvalidHandle)?;
    if handle == 0 {
        return Err(Status::InvalidHandle);
    }
    registry()
        .lock()
        .map_err(|_| Status::RegistryPoisoned)?
        .entries
        .remove(&handle)
        .map(|_| ())
        .ok_or(Status::InvalidHandle)
}

fn java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<String, Status> {
    env.get_string(value)
        .map(Into::into)
        .map_err(|_| Status::Conversion)
}

fn optional_java_string(
    env: &mut JNIEnv<'_>,
    value: &JString<'_>,
) -> Result<Option<String>, Status> {
    if value.is_null() {
        Ok(None)
    } else {
        java_string(env, value).map(Some)
    }
}

fn java_bytes(env: &mut JNIEnv<'_>, value: &JByteArray<'_>) -> Result<Vec<u8>, Status> {
    env.convert_byte_array(value)
        .map_err(|_| Status::Conversion)
}

fn new_string(env: &mut JNIEnv<'_>, value: &str) -> jstring {
    env.new_string(value).map_or(null_mut(), JString::into_raw)
}

fn envelope(result: Result<Value, Status>) -> String {
    match result {
        Ok(value) => json!({ "status": Status::Success.code(), "value": value }).to_string(),
        Err(error) => json!({ "status": error.code() }).to_string(),
    }
}

fn envelope_string(env: &mut JNIEnv<'_>, result: Result<Value, Status>) -> jstring {
    new_string(env, &envelope(result))
}

fn status(result: Result<(), Status>) -> jint {
    result.map_or_else(|error| error.code(), |()| Status::Success.code())
}

fn recent_json(value: &RecentSessionRecord) -> Value {
    json!({
        "sessionId": value.session_id,
        "sessionName": value.session_name,
        "hostName": value.host_name,
        "hostFingerprint": value.host_fingerprint,
        "startedAtMs": value.started_at_ms,
        "endedAtMs": value.ended_at_ms,
        "outcome": value.outcome.wire_name(),
    })
}

fn trusted_json(value: &TrustedHostRecord) -> Value {
    json!({
        "fingerprint": value.fingerprint,
        "displayName": value.display_name,
        "publicKeyDer": STANDARD.encode(&value.public_key_der),
        "lastVerifiedMs": value.last_verified_ms,
    })
}

fn invitation_json(value: &ValidatedQrInvitation) -> Value {
    json!({
        "sessionId": value.session_id,
        "sessionName": value.session_name,
        "hostName": value.host_name,
        "hostFingerprint": value.host_fingerprint,
        "hostPublicKeyDer": STANDARD.encode(&value.host_public_key_der),
        "approvalMode": value.approval_mode,
        "inviteCode": value.invite_code,
        "issuedAtMs": value.issued_at_ms,
        "expiresAtMs": value.expires_at_ms,
    })
}

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
