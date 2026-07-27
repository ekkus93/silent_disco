#![allow(unsafe_code)]

use std::{
    collections::{BTreeMap, btree_map::Entry},
    path::PathBuf,
    ptr::null_mut,
    sync::{Arc, Mutex, OnceLock},
};

use jni::{
    JNIEnv,
    objects::{JByteArray, JObject, JString},
    sys::{jbyteArray, jint, jlong, jstring},
};
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

struct Entry {
    store: P2Store,
    recent: Option<Vec<RecentSessionRecord>>,
    trusted: Option<Vec<TrustedHostRecord>>,
    invitation: Option<ValidatedQrInvitation>,
}

#[derive(Default)]
struct Registry {
    next_handle: u64,
    entries: BTreeMap<u64, Arc<Mutex<Entry>>>,
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
    let entry = Arc::new(Mutex::new(Entry {
        store,
        recent: None,
        trusted: None,
        invitation: None,
    }));
    let mut registry = registry().lock().map_err(|_| Status::RegistryPoisoned)?;
    let handle = registry.next_handle;
    if handle == 0 || handle > MAX_HANDLE {
        return Err(Status::InvalidHandle);
    }
    match registry.entries.entry(handle) {
        Entry::Vacant(slot) => {
            slot.insert(entry);
        }
        Entry::Occupied(_) => return Err(Status::InvalidHandle),
    }
    registry.next_handle = handle.checked_add(1).ok_or(Status::InvalidHandle)?;
    i64::try_from(handle).map_err(|_| Status::InvalidHandle)
}

fn entry(handle: jlong) -> Result<Arc<Mutex<Entry>>, Status> {
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

fn with_entry<T>(handle: jlong, action: impl FnOnce(&mut Entry) -> Result<T, Status>) -> Result<T, Status> {
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

fn optional_java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<Option<String>, Status> {
    if value.is_null() {
        Ok(None)
    } else {
        java_string(env, value).map(Some)
    }
}

fn java_bytes(env: &mut JNIEnv<'_>, value: &JByteArray<'_>) -> Result<Vec<u8>, Status> {
    env.convert_byte_array(value).map_err(|_| Status::Conversion)
}

fn new_string(env: &JNIEnv<'_>, value: &str) -> jstring {
    env.new_string(value).map_or(null_mut(), JString::into_raw)
}

fn new_bytes(env: &JNIEnv<'_>, value: &[u8]) -> jbyteArray {
    env.byte_array_from_slice(value)
        .map_or(null_mut(), JByteArray::into_raw)
}

fn status(result: Result<(), Status>) -> jint {
    result.map_or_else(Status::code, |()| Status::Success.code())
}

fn recent_values(handle: jlong) -> Result<Vec<RecentSessionRecord>, Status> {
    with_entry(handle, |entry| entry.recent.clone().ok_or(Status::CacheUnavailable))
}

fn recent_value(handle: jlong, index: jint) -> Result<RecentSessionRecord, Status> {
    let index = usize::try_from(index).map_err(|_| Status::InvalidArgument)?;
    recent_values(handle)?
        .get(index)
        .cloned()
        .ok_or(Status::InvalidArgument)
}

fn trusted_values(handle: jlong) -> Result<Vec<TrustedHostRecord>, Status> {
    with_entry(handle, |entry| entry.trusted.clone().ok_or(Status::CacheUnavailable))
}

fn trusted_value(handle: jlong, index: jint) -> Result<TrustedHostRecord, Status> {
    let index = usize::try_from(index).map_err(|_| Status::InvalidArgument)?;
    trusted_values(handle)?
        .get(index)
        .cloned()
        .ok_or(Status::InvalidArgument)
}

fn invitation(handle: jlong) -> Result<ValidatedQrInvitation, Status> {
    with_entry(handle, |entry| entry.invitation.clone().ok_or(Status::CacheUnavailable))
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
            entry.store.record_session(&value).map_err(|error| map_error(&error))?;
            entry.recent = None;
            Ok(())
        })
    })();
    status(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2LoadRecentStatus(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    now_ms: jlong,
    max_age_ms: jlong,
    limit: jint,
) -> jint {
    let result = (|| {
        let now_ms = u64::try_from(now_ms).map_err(|_| Status::InvalidArgument)?;
        let max_age_ms = if max_age_ms == 0 {
            DEFAULT_RECENT_MAX_AGE_MS
        } else {
            u64::try_from(max_age_ms).map_err(|_| Status::InvalidArgument)?
        };
        let limit = u32::try_from(limit).map_err(|_| Status::InvalidArgument)?;
        with_entry(handle, |entry| {
            let values = entry
                .store
                .list_recent_listener_sessions(now_ms, max_age_ms, limit)
                .map_err(|error| map_error(&error))?;
            entry.recent = Some(values);
            Ok(())
        })
    })();
    status(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentCount(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    recent_values(handle)
        .and_then(|values| i32::try_from(values.len()).map_err(|_| Status::InvalidArgument))
        .unwrap_or_else(Status::code)
}

macro_rules! recent_string_getter {
    ($name:ident, $field:expr) => {
        #[must_use]
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(
            env: JNIEnv<'_>,
            _receiver: JObject<'_>,
            handle: jlong,
            index: jint,
        ) -> jstring {
            recent_value(handle, index)
                .map(|value| $field(value))
                .map_or(null_mut(), |value: String| new_string(&env, &value))
        }
    };
}

recent_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentSessionId,
    |value: RecentSessionRecord| value.session_id
);
recent_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentSessionName,
    |value: RecentSessionRecord| value.session_name
);
recent_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentHostName,
    |value: RecentSessionRecord| value.host_name
);
recent_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentHostFingerprint,
    |value: RecentSessionRecord| value.host_fingerprint.unwrap_or_default()
);

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentStartedAtMs(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jlong {
    recent_value(handle, index)
        .and_then(|value| i64::try_from(value.started_at_ms).map_err(|_| Status::InvalidArgument))
        .unwrap_or(-1)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentEndedAtMs(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jlong {
    recent_value(handle, index)
        .and_then(|value| i64::try_from(value.ended_at_ms).map_err(|_| Status::InvalidArgument))
        .unwrap_or(-1)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedRecentOutcome(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jint {
    recent_value(handle, index).map_or_else(Status::code, |value| match value.outcome {
        RecentSessionOutcome::Completed => 1,
        RecentSessionOutcome::Cancelled => 2,
        RecentSessionOutcome::Failed => 3,
    })
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2PrepareUnsignedQr(
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
        prepare_unsigned_qr(&input).map_err(|error| map_error(&error))
    })();
    result.map_or(null_mut(), |value| new_string(&env, &value))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2FinalizeQr(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    unsigned_json: JString<'_>,
    signature_base64url: JString<'_>,
) -> jstring {
    let result = java_string(&mut env, &unsigned_json)
        .and_then(|unsigned| {
            java_string(&mut env, &signature_base64url).and_then(|signature| {
                finalize_qr(&unsigned, &signature).map_err(|error| map_error(&error))
            })
        });
    result.map_or(null_mut(), |value| new_string(&env, &value))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2Fingerprint(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    public_key_der: JByteArray<'_>,
) -> jstring {
    java_bytes(&mut env, &public_key_der)
        .map(|value| public_key_fingerprint(&value))
        .map_or(null_mut(), |value| new_string(&env, &value))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidateQr(
    mut env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    payload: JString<'_>,
    now_ms: jlong,
) -> jint {
    let result = java_string(&mut env, &payload)
        .and_then(|payload| {
            let now_ms = u64::try_from(now_ms).map_err(|_| Status::InvalidArgument)?;
            with_entry(handle, |entry| {
                let value = entry
                    .store
                    .validate_and_consume_qr(&payload, now_ms)
                    .map_err(|error| map_error(&error))?;
                entry.invitation = Some(value);
                Ok(())
            })
        });
    status(result)
}

macro_rules! invitation_string_getter {
    ($name:ident, $field:expr) => {
        #[must_use]
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(
            env: JNIEnv<'_>,
            _receiver: JObject<'_>,
            handle: jlong,
        ) -> jstring {
            invitation(handle)
                .map(|value| $field(value))
                .map_or(null_mut(), |value: String| new_string(&env, &value))
        }
    };
}

invitation_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedSessionId,
    |value: ValidatedQrInvitation| value.session_id
);
invitation_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedSessionName,
    |value: ValidatedQrInvitation| value.session_name
);
invitation_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedHostName,
    |value: ValidatedQrInvitation| value.host_name
);
invitation_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedFingerprint,
    |value: ValidatedQrInvitation| value.host_fingerprint
);
invitation_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedApprovalMode,
    |value: ValidatedQrInvitation| value.approval_mode
);
invitation_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedInviteCode,
    |value: ValidatedQrInvitation| value.invite_code.unwrap_or_default()
);

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedPublicKeyDer(
    env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jbyteArray {
    invitation(handle)
        .map_or(null_mut(), |value| new_bytes(&env, &value.host_public_key_der))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedIssuedAtMs(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jlong {
    invitation(handle)
        .and_then(|value| i64::try_from(value.issued_at_ms).map_err(|_| Status::InvalidArgument))
        .unwrap_or(-1)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2ValidatedExpiresAtMs(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jlong {
    invitation(handle)
        .and_then(|value| i64::try_from(value.expires_at_ms).map_err(|_| Status::InvalidArgument))
        .unwrap_or(-1)
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
            let host = TrustedHostRecord {
                fingerprint: invitation.host_fingerprint,
                display_name: invitation.host_name,
                public_key_der: invitation.host_public_key_der,
                first_verified_ms: verified_at_ms,
                last_verified_ms: verified_at_ms,
            };
            entry.store.trust_host(&host).map_err(|error| map_error(&error))?;
            entry.trusted = None;
            Ok(())
        })
    })();
    status(result)
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2LoadTrustedStatus(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    status(with_entry(handle, |entry| {
        let values = entry.store.list_trusted_hosts().map_err(|error| map_error(&error))?;
        entry.trusted = Some(values);
        Ok(())
    }))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedTrustedCount(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
) -> jint {
    trusted_values(handle)
        .and_then(|values| i32::try_from(values.len()).map_err(|_| Status::InvalidArgument))
        .unwrap_or_else(Status::code)
}

macro_rules! trusted_string_getter {
    ($name:ident, $field:expr) => {
        #[must_use]
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(
            env: JNIEnv<'_>,
            _receiver: JObject<'_>,
            handle: jlong,
            index: jint,
        ) -> jstring {
            trusted_value(handle, index)
                .map(|value| $field(value))
                .map_or(null_mut(), |value: String| new_string(&env, &value))
        }
    };
}

trusted_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedTrustedFingerprint,
    |value: TrustedHostRecord| value.fingerprint
);
trusted_string_getter!(
    Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedTrustedDisplayName,
    |value: TrustedHostRecord| value.display_name
);

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedTrustedPublicKeyDer(
    env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jbyteArray {
    trusted_value(handle, index).map_or(null_mut(), |value| new_bytes(&env, &value.public_key_der))
}

#[must_use]
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_ekkus_silentdisco_core_rust_P2RustBridge_nativeP2CachedTrustedLastVerifiedMs(
    _env: JNIEnv<'_>,
    _receiver: JObject<'_>,
    handle: jlong,
    index: jint,
) -> jlong {
    trusted_value(handle, index)
        .and_then(|value| i64::try_from(value.last_verified_ms).map_err(|_| Status::InvalidArgument))
        .unwrap_or(-1)
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
            let deleted = entry
                .store
                .delete_trusted_host(&fingerprint)
                .map_err(|error| map_error(&error))?;
            entry.trusted = None;
            Ok(deleted)
        })
    });
    match result {
        Ok(true) => Status::Success.code(),
        Ok(false) => Status::NotFound.code(),
        Err(error) => error.code(),
    }
}
