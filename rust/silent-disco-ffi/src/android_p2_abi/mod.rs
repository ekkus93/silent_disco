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
    result.map_or_else(Status::code, |()| Status::Success.code())
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

include!("exports_core.rs");
include!("exports_trust.rs");
