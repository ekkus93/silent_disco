//! Rust-owned persistence and validation for optional P2 convenience features.
//!
//! This module deliberately keeps recent-session history, trusted-host keys, and
//! consumed invitation nonces out of Kotlin-owned state. The store uses a
//! dedicated app-private `SQLite` file so it never opens or competes for the
//! existing domain database connection.

use std::{
    error::Error,
    fmt,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{
    ecdsa::{Signature, VerifyingKey, signature::Verifier},
    pkcs8::DecodePublicKey,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::SessionId;

pub const P2_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_RECENT_MAX_AGE_MS: u64 = 30 * 24 * 60 * 60 * 1_000;
pub const MAX_RECENT_QUERY_LIMIT: u32 = 20;
pub const MAX_QR_LIFETIME_MS: u64 = 10 * 60 * 1_000;
pub const QR_CLOCK_SKEW_MS: u64 = 60 * 1_000;
const MAX_NAME_BYTES: usize = 256;
const MAX_PUBLIC_KEY_BYTES: usize = 1_024;
const MAX_QR_PAYLOAD_BYTES: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentSessionRole {
    Host,
    Listener,
}

impl RecentSessionRole {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Listener => "listener",
        }
    }

    fn parse(value: &str) -> Result<Self, P2Error> {
        match value {
            "host" => Ok(Self::Host),
            "listener" => Ok(Self::Listener),
            _ => Err(P2Error::CorruptStore(
                "unknown recent-session role".to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentSessionOutcome {
    Completed,
    Cancelled,
    Failed,
}

impl RecentSessionOutcome {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, P2Error> {
        match value {
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            _ => Err(P2Error::CorruptStore(
                "unknown recent-session outcome".to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSessionRecord {
    pub session_id: String,
    pub role: RecentSessionRole,
    pub session_name: String,
    pub host_name: String,
    pub host_fingerprint: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub outcome: RecentSessionOutcome,
}

impl RecentSessionRecord {
    fn validate(&self) -> Result<(), P2Error> {
        SessionId::new(self.session_id.clone())
            .map_err(|_| P2Error::InvalidArgument("invalid session identifier".to_owned()))?;
        validate_name(&self.session_name, "session name")?;
        validate_name(&self.host_name, "host name")?;
        validate_fingerprint(self.host_fingerprint.as_deref())?;
        if self.started_at_ms == 0 || self.ended_at_ms < self.started_at_ms {
            return Err(P2Error::InvalidArgument(
                "recent-session timestamps are invalid".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedHostRecord {
    pub fingerprint: String,
    pub display_name: String,
    pub public_key_der: Vec<u8>,
    pub first_verified_ms: u64,
    pub last_verified_ms: u64,
}

impl TrustedHostRecord {
    fn validate(&self) -> Result<(), P2Error> {
        validate_fingerprint(Some(&self.fingerprint))?;
        validate_name(&self.display_name, "trusted-host display name")?;
        if self.public_key_der.is_empty() || self.public_key_der.len() > MAX_PUBLIC_KEY_BYTES {
            return Err(P2Error::InvalidArgument(
                "trusted-host public key has an unsupported size".to_owned(),
            ));
        }
        if self.first_verified_ms == 0 || self.last_verified_ms < self.first_verified_ms {
            return Err(P2Error::InvalidArgument(
                "trusted-host timestamps are invalid".to_owned(),
            ));
        }
        let actual = public_key_fingerprint(&self.public_key_der);
        if actual != self.fingerprint {
            return Err(P2Error::InvalidArgument(
                "trusted-host fingerprint does not match its public key".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrInvitationInput {
    pub session_id: String,
    pub session_name: String,
    pub host_name: String,
    pub host_public_key_der: Vec<u8>,
    pub approval_mode: String,
    pub invite_code: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedQrInvitation {
    pub session_id: String,
    pub session_name: String,
    pub host_name: String,
    pub host_public_key_der: Vec<u8>,
    pub host_fingerprint: String,
    pub approval_mode: String,
    pub invite_code: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnsignedQrDocument {
    v: u8,
    alg: String,
    sid: String,
    sn: String,
    hn: String,
    pk: String,
    am: String,
    code: Option<String>,
    iat: u64,
    exp: u64,
    nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedQrDocument {
    v: u8,
    alg: String,
    sid: String,
    sn: String,
    hn: String,
    pk: String,
    am: String,
    code: Option<String>,
    iat: u64,
    exp: u64,
    nonce: String,
    sig: String,
}

impl From<SignedQrDocument> for UnsignedQrDocument {
    fn from(value: SignedQrDocument) -> Self {
        Self {
            v: value.v,
            alg: value.alg,
            sid: value.sid,
            sn: value.sn,
            hn: value.hn,
            pk: value.pk,
            am: value.am,
            code: value.code,
            iat: value.iat,
            exp: value.exp,
            nonce: value.nonce,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2Error {
    InvalidArgument(String),
    Storage(String),
    CorruptStore(String),
    InvalidQr(String),
    ExpiredQr,
    ReplayedQr,
    InvalidSignature,
}

impl fmt::Display for P2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(message) => write!(formatter, "invalid P2 argument: {message}"),
            Self::Storage(message) => write!(formatter, "P2 storage failure: {message}"),
            Self::CorruptStore(message) => write!(formatter, "P2 storage is corrupt: {message}"),
            Self::InvalidQr(message) => {
                write!(formatter, "invalid Silent Disco QR invitation: {message}")
            }
            Self::ExpiredQr => formatter.write_str("Silent Disco QR invitation has expired"),
            Self::ReplayedQr => {
                formatter.write_str("Silent Disco QR invitation was already used on this phone")
            }
            Self::InvalidSignature => {
                formatter.write_str("Silent Disco QR invitation signature is invalid")
            }
        }
    }
}

impl Error for P2Error {}

pub struct P2Store {
    connection: Connection,
}

impl P2Store {
    /// Opens the dedicated P2 store, applies migrations, and verifies database integrity.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid, the database cannot be created or opened,
    /// a migration fails, or the integrity check detects corruption.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, P2Error> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(P2Error::InvalidArgument(
                "database path is empty".to_owned(),
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                P2Error::Storage(format!("cannot create database directory: {error}"))
            })?;
        }
        let mut connection = Connection::open(path).map_err(|error| storage_error(&error))?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;\nPRAGMA journal_mode = WAL;\nPRAGMA synchronous = FULL;",
            )
            .map_err(|error| storage_error(&error))?;
        migrate(&mut connection)?;
        let quick_check: String = connection
            .query_row("PRAGMA quick_check;", [], |row| row.get(0))
            .map_err(|error| storage_error(&error))?;
        if quick_check != "ok" {
            return Err(P2Error::CorruptStore(format!(
                "SQLite quick_check returned {quick_check}"
            )));
        }
        Ok(Self { connection })
    }

    /// Persists or updates one authoritative recent-session record.
    ///
    /// # Errors
    ///
    /// Returns an error when the record is invalid or the transaction cannot be committed.
    pub fn record_session(&mut self, value: &RecentSessionRecord) -> Result<(), P2Error> {
        value.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error(&error))?;
        transaction
            .execute(
                "INSERT INTO recent_sessions (
                     session_id, role, session_name, host_name, host_fingerprint,
                     started_at_ms, ended_at_ms, outcome
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(session_id, role) DO UPDATE SET
                     session_name = excluded.session_name,
                     host_name = excluded.host_name,
                     host_fingerprint = excluded.host_fingerprint,
                     started_at_ms = MIN(recent_sessions.started_at_ms, excluded.started_at_ms),
                     ended_at_ms = excluded.ended_at_ms,
                     outcome = excluded.outcome",
                params![
                    value.session_id,
                    value.role.wire_name(),
                    value.session_name,
                    value.host_name,
                    value.host_fingerprint,
                    to_sql_millis(value.started_at_ms)?,
                    to_sql_millis(value.ended_at_ms)?,
                    value.outcome.wire_name(),
                ],
            )
            .map_err(|error| storage_error(&error))?;
        transaction.commit().map_err(|error| storage_error(&error))
    }

    /// Returns bounded listener-session history after deleting expired records.
    ///
    /// # Errors
    ///
    /// Returns an error when the query bounds are invalid, cleanup fails, stored rows are
    /// corrupt, or the database query cannot be completed.
    pub fn list_recent_listener_sessions(
        &mut self,
        now_ms: u64,
        max_age_ms: u64,
        limit: u32,
    ) -> Result<Vec<RecentSessionRecord>, P2Error> {
        if now_ms == 0 || max_age_ms == 0 || limit == 0 || limit > MAX_RECENT_QUERY_LIMIT {
            return Err(P2Error::InvalidArgument(
                "recent-session query bounds are invalid".to_owned(),
            ));
        }
        let cutoff = now_ms.saturating_sub(max_age_ms);
        self.cleanup(now_ms, max_age_ms)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT session_id, role, session_name, host_name, host_fingerprint,
                        started_at_ms, ended_at_ms, outcome
                 FROM recent_sessions
                 WHERE role = 'listener' AND ended_at_ms >= ?1
                 ORDER BY ended_at_ms DESC, session_id ASC
                 LIMIT ?2",
            )
            .map_err(|error| storage_error(&error))?;
        let rows = statement
            .query_map(params![to_sql_millis(cutoff)?, i64::from(limit)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|error| storage_error(&error))?;
        let mut values = Vec::new();
        for row in rows {
            let (
                session_id,
                role,
                session_name,
                host_name,
                host_fingerprint,
                started,
                ended,
                outcome,
            ) = row.map_err(|error| storage_error(&error))?;
            let record = RecentSessionRecord {
                session_id,
                role: RecentSessionRole::parse(&role)?,
                session_name,
                host_name,
                host_fingerprint,
                started_at_ms: from_sql_millis(started)?,
                ended_at_ms: from_sql_millis(ended)?,
                outcome: RecentSessionOutcome::parse(&outcome)?,
            };
            record.validate()?;
            values.push(record);
        }
        Ok(values)
    }

    /// Persists a cryptographically verified host identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the host record is invalid or the transaction cannot be committed.
    pub fn trust_host(&mut self, host: &TrustedHostRecord) -> Result<(), P2Error> {
        host.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error(&error))?;
        transaction
            .execute(
                "INSERT INTO trusted_hosts (
                     fingerprint, display_name, public_key_der, first_verified_ms, last_verified_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(fingerprint) DO UPDATE SET
                     display_name = excluded.display_name,
                     public_key_der = excluded.public_key_der,
                     first_verified_ms = MIN(trusted_hosts.first_verified_ms, excluded.first_verified_ms),
                     last_verified_ms = MAX(trusted_hosts.last_verified_ms, excluded.last_verified_ms)",
                params![
                    host.fingerprint,
                    host.display_name,
                    host.public_key_der,
                    to_sql_millis(host.first_verified_ms)?,
                    to_sql_millis(host.last_verified_ms)?,
                ],
            )
            .map_err(|error| storage_error(&error))?;
        transaction.commit().map_err(|error| storage_error(&error))
    }

    /// Returns all trusted host identities in deterministic display order.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails or a persisted host record is corrupt.
    pub fn list_trusted_hosts(&self) -> Result<Vec<TrustedHostRecord>, P2Error> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT fingerprint, display_name, public_key_der, first_verified_ms, last_verified_ms
                 FROM trusted_hosts
                 ORDER BY display_name COLLATE NOCASE ASC, fingerprint ASC",
            )
            .map_err(|error| storage_error(&error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|error| storage_error(&error))?;
        let mut values = Vec::new();
        for row in rows {
            let (fingerprint, display_name, public_key_der, first_verified, last_verified) =
                row.map_err(|error| storage_error(&error))?;
            let host = TrustedHostRecord {
                fingerprint,
                display_name,
                public_key_der,
                first_verified_ms: from_sql_millis(first_verified)?,
                last_verified_ms: from_sql_millis(last_verified)?,
            };
            host.validate()?;
            values.push(host);
        }
        Ok(values)
    }

    /// Deletes the trusted host identified by the exact public-key fingerprint.
    ///
    /// # Errors
    ///
    /// Returns an error when the fingerprint is invalid or the transaction cannot be committed.
    pub fn delete_trusted_host(&mut self, fingerprint: &str) -> Result<bool, P2Error> {
        validate_fingerprint(Some(fingerprint))?;
        let affected = self
            .connection
            .execute(
                "DELETE FROM trusted_hosts WHERE fingerprint = ?1",
                [fingerprint],
            )
            .map_err(|error| storage_error(&error))?;
        Ok(affected == 1)
    }

    /// Validates a signed invitation and atomically consumes its replay-protection nonce.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, expired, replayed, or incorrectly signed invitations,
    /// and when replay-ledger persistence fails.
    pub fn validate_and_consume_qr(
        &mut self,
        payload: &str,
        now_ms: u64,
    ) -> Result<ValidatedQrInvitation, P2Error> {
        if payload.is_empty() || payload.len() > MAX_QR_PAYLOAD_BYTES {
            return Err(P2Error::InvalidQr("payload size is unsupported".to_owned()));
        }
        let signed: SignedQrDocument =
            serde_json::from_str(payload).map_err(|error| P2Error::InvalidQr(error.to_string()))?;
        let signature_text = signed.sig.clone();
        let unsigned: UnsignedQrDocument = signed.into();
        validate_unsigned(&unsigned, now_ms)?;
        let canonical = canonical_unsigned(&unsigned)?;
        let public_key_der = URL_SAFE_NO_PAD
            .decode(unsigned.pk.as_bytes())
            .map_err(|_| P2Error::InvalidQr("host public key is not valid base64url".to_owned()))?;
        if public_key_der.is_empty() || public_key_der.len() > MAX_PUBLIC_KEY_BYTES {
            return Err(P2Error::InvalidQr(
                "host public key size is unsupported".to_owned(),
            ));
        }
        let verifying_key = VerifyingKey::from_public_key_der(&public_key_der)
            .map_err(|_| P2Error::InvalidQr("host public key is not a P-256 key".to_owned()))?;
        let signature_der = URL_SAFE_NO_PAD
            .decode(signature_text.as_bytes())
            .map_err(|_| P2Error::InvalidQr("signature is not valid base64url".to_owned()))?;
        let signature =
            Signature::from_der(&signature_der).map_err(|_| P2Error::InvalidSignature)?;
        verifying_key
            .verify(canonical.as_bytes(), &signature)
            .map_err(|_| P2Error::InvalidSignature)?;

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error(&error))?;
        transaction
            .execute(
                "DELETE FROM consumed_qr_nonces WHERE expires_at_ms < ?1",
                [to_sql_millis(now_ms)?],
            )
            .map_err(|error| storage_error(&error))?;
        let already_used = transaction
            .query_row(
                "SELECT 1 FROM consumed_qr_nonces WHERE nonce = ?1",
                [&unsigned.nonce],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| storage_error(&error))?
            .is_some();
        if already_used {
            return Err(P2Error::ReplayedQr);
        }
        transaction
            .execute(
                "INSERT INTO consumed_qr_nonces(nonce, consumed_at_ms, expires_at_ms)
                 VALUES (?1, ?2, ?3)",
                params![
                    unsigned.nonce,
                    to_sql_millis(now_ms)?,
                    to_sql_millis(unsigned.exp)?,
                ],
            )
            .map_err(|error| storage_error(&error))?;
        transaction.commit().map_err(|error| storage_error(&error))?;

        Ok(ValidatedQrInvitation {
            session_id: unsigned.sid,
            session_name: unsigned.sn,
            host_name: unsigned.hn,
            host_fingerprint: public_key_fingerprint(&public_key_der),
            host_public_key_der: public_key_der,
            approval_mode: unsigned.am,
            invite_code: unsigned.code,
            issued_at_ms: unsigned.iat,
            expires_at_ms: unsigned.exp,
            nonce: unsigned.nonce,
        })
    }

    /// Deletes expired recent sessions and consumed invitation nonces.
    ///
    /// # Errors
    ///
    /// Returns an error when the cleanup bounds are invalid or the transaction fails.
    pub fn cleanup(&mut self, now_ms: u64, recent_max_age_ms: u64) -> Result<(), P2Error> {
        if now_ms == 0 || recent_max_age_ms == 0 {
            return Err(P2Error::InvalidArgument(
                "cleanup bounds are invalid".to_owned(),
            ));
        }
        let cutoff = now_ms.saturating_sub(recent_max_age_ms);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error(&error))?;
        transaction
            .execute(
                "DELETE FROM recent_sessions WHERE ended_at_ms < ?1",
                [to_sql_millis(cutoff)?],
            )
            .map_err(|error| storage_error(&error))?;
        transaction
            .execute(
                "DELETE FROM consumed_qr_nonces WHERE expires_at_ms < ?1",
                [to_sql_millis(now_ms)?],
            )
            .map_err(|error| storage_error(&error))?;
        transaction.commit().map_err(|error| storage_error(&error))
    }
}

/// Builds the canonical unsigned `JSON` document that the host must sign.
///
/// # Errors
///
/// Returns an error when any invitation field is invalid or canonical serialization fails.
pub fn prepare_unsigned_qr(input: &QrInvitationInput) -> Result<String, P2Error> {
    let document = unsigned_document(input)?;
    validate_unsigned(&document, input.issued_at_ms)?;
    canonical_unsigned(&document)
}

/// Attaches a `DER`-encoded `ES256` signature to a canonical unsigned invitation.
///
/// # Errors
///
/// Returns an error when the unsigned document is noncanonical or malformed, or when the
/// signature is not valid unpadded base64url containing a `DER` `ECDSA` signature.
pub fn finalize_qr(unsigned_json: &str, signature_base64url: &str) -> Result<String, P2Error> {
    if unsigned_json.is_empty() || unsigned_json.len() > MAX_QR_PAYLOAD_BYTES {
        return Err(P2Error::InvalidQr(
            "unsigned payload size is unsupported".to_owned(),
        ));
    }
    let unsigned: UnsignedQrDocument = serde_json::from_str(unsigned_json)
        .map_err(|error| P2Error::InvalidQr(error.to_string()))?;
    let canonical = canonical_unsigned(&unsigned)?;
    if canonical != unsigned_json {
        return Err(P2Error::InvalidQr(
            "unsigned payload is not in canonical form".to_owned(),
        ));
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature_base64url.as_bytes())
        .map_err(|_| P2Error::InvalidQr("signature is not valid base64url".to_owned()))?;
    Signature::from_der(&signature).map_err(|_| P2Error::InvalidSignature)?;
    let prefix = canonical
        .strip_suffix('}')
        .ok_or_else(|| P2Error::InvalidQr("canonical payload is malformed".to_owned()))?;
    Ok(format!(
        "{prefix},\"sig\":{}}}",
        json_string(signature_base64url)?
    ))
}

#[must_use]
pub fn public_key_fingerprint(public_key_der: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

    let digest = Sha256::digest(public_key_der);
    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        fingerprint.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        fingerprint.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    fingerprint
}

fn unsigned_document(input: &QrInvitationInput) -> Result<UnsignedQrDocument, P2Error> {
    if input.host_public_key_der.is_empty()
        || input.host_public_key_der.len() > MAX_PUBLIC_KEY_BYTES
    {
        return Err(P2Error::InvalidArgument(
            "host public key has an unsupported size".to_owned(),
        ));
    }
    Ok(UnsignedQrDocument {
        v: 1,
        alg: "ES256".to_owned(),
        sid: input.session_id.clone(),
        sn: input.session_name.clone(),
        hn: input.host_name.clone(),
        pk: URL_SAFE_NO_PAD.encode(&input.host_public_key_der),
        am: input.approval_mode.clone(),
        code: input.invite_code.clone(),
        iat: input.issued_at_ms,
        exp: input.expires_at_ms,
        nonce: input.nonce.clone(),
    })
}

fn validate_unsigned(document: &UnsignedQrDocument, now_ms: u64) -> Result<(), P2Error> {
    if document.v != 1 || document.alg != "ES256" {
        return Err(P2Error::InvalidQr(
            "unsupported invitation version or signature algorithm".to_owned(),
        ));
    }
    SessionId::new(document.sid.clone())
        .map_err(|_| P2Error::InvalidQr("session identifier is invalid".to_owned()))?;
    validate_name(&document.sn, "session name")
        .map_err(|error| P2Error::InvalidQr(error.to_string()))?;
    validate_name(&document.hn, "host name")
        .map_err(|error| P2Error::InvalidQr(error.to_string()))?;
    if !matches!(
        document.am.as_str(),
        "manual" | "invite_code" | "approved_devices"
    ) {
        return Err(P2Error::InvalidQr("approval mode is invalid".to_owned()));
    }
    match document.am.as_str() {
        "invite_code" => {
            let code = document
                .code
                .as_deref()
                .ok_or_else(|| P2Error::InvalidQr("invite code is missing".to_owned()))?;
            if code.trim() != code || !(4..=32).contains(&code.len()) {
                return Err(P2Error::InvalidQr("invite code is invalid".to_owned()));
            }
        }
        _ if document.code.is_some() => {
            return Err(P2Error::InvalidQr(
                "invite code is present for a mode that does not use one".to_owned(),
            ));
        }
        _ => {}
    }
    if document.iat == 0
        || document.exp <= document.iat
        || document.exp - document.iat > MAX_QR_LIFETIME_MS
    {
        return Err(P2Error::InvalidQr(
            "invitation lifetime is invalid".to_owned(),
        ));
    }
    if now_ms.saturating_add(QR_CLOCK_SKEW_MS) < document.iat {
        return Err(P2Error::InvalidQr(
            "invitation was issued too far in the future".to_owned(),
        ));
    }
    if now_ms > document.exp.saturating_add(QR_CLOCK_SKEW_MS) {
        return Err(P2Error::ExpiredQr);
    }
    validate_nonce(&document.nonce)?;
    Ok(())
}

fn canonical_unsigned(document: &UnsignedQrDocument) -> Result<String, P2Error> {
    Ok(format!(
        "{{\"v\":{},\"alg\":{},\"sid\":{},\"sn\":{},\"hn\":{},\"pk\":{},\"am\":{},\"code\":{},\"iat\":{},\"exp\":{},\"nonce\":{}}}",
        document.v,
        json_string(&document.alg)?,
        json_string(&document.sid)?,
        json_string(&document.sn)?,
        json_string(&document.hn)?,
        json_string(&document.pk)?,
        json_string(&document.am)?,
        match &document.code {
            Some(code) => json_string(code)?,
            None => "null".to_owned(),
        },
        document.iat,
        document.exp,
        json_string(&document.nonce)?,
    ))
}

fn json_string(value: &str) -> Result<String, P2Error> {
    serde_json::to_string(value).map_err(|error| P2Error::InvalidQr(error.to_string()))
}

fn validate_name(value: &str, field: &str) -> Result<(), P2Error> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > MAX_NAME_BYTES
        || value.contains('\0')
    {
        return Err(P2Error::InvalidArgument(format!("{field} is invalid")));
    }
    Ok(())
}

fn validate_nonce(value: &str) -> Result<(), P2Error> {
    if !(16..=80).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(P2Error::InvalidQr("nonce is invalid".to_owned()));
    }
    Ok(())
}

fn validate_fingerprint(value: Option<&str>) -> Result<(), P2Error> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(P2Error::InvalidArgument(
            "host fingerprint is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<(), P2Error> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| storage_error(&error))?;
    match version {
        0 => {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| storage_error(&error))?;
            transaction
                .execute_batch(
                    "CREATE TABLE recent_sessions (
                         session_id TEXT NOT NULL,
                         role TEXT NOT NULL CHECK(role IN ('host','listener')),
                         session_name TEXT NOT NULL CHECK(length(session_name) BETWEEN 1 AND 256),
                         host_name TEXT NOT NULL CHECK(length(host_name) BETWEEN 1 AND 256),
                         host_fingerprint TEXT CHECK(host_fingerprint IS NULL OR length(host_fingerprint) = 64),
                         started_at_ms INTEGER NOT NULL CHECK(started_at_ms > 0),
                         ended_at_ms INTEGER NOT NULL CHECK(ended_at_ms >= started_at_ms),
                         outcome TEXT NOT NULL CHECK(outcome IN ('completed','cancelled','failed')),
                         PRIMARY KEY(session_id, role)
                     ) STRICT;
                     CREATE INDEX idx_recent_listener_end
                         ON recent_sessions(role, ended_at_ms DESC, session_id);
                     CREATE TABLE trusted_hosts (
                         fingerprint TEXT PRIMARY KEY CHECK(length(fingerprint) = 64),
                         display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 256),
                         public_key_der BLOB NOT NULL UNIQUE CHECK(length(public_key_der) BETWEEN 1 AND 1024),
                         first_verified_ms INTEGER NOT NULL CHECK(first_verified_ms > 0),
                         last_verified_ms INTEGER NOT NULL CHECK(last_verified_ms >= first_verified_ms)
                     ) STRICT;
                     CREATE TABLE consumed_qr_nonces (
                         nonce TEXT PRIMARY KEY CHECK(length(nonce) BETWEEN 16 AND 80),
                         consumed_at_ms INTEGER NOT NULL CHECK(consumed_at_ms > 0),
                         expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms >= consumed_at_ms)
                     ) STRICT;
                     CREATE INDEX idx_consumed_qr_expiry ON consumed_qr_nonces(expires_at_ms);",
                )
                .map_err(|error| storage_error(&error))?;
            transaction
                .pragma_update(None, "user_version", P2_SCHEMA_VERSION)
                .map_err(|error| storage_error(&error))?;
            transaction.commit().map_err(|error| storage_error(&error))
        }
        value if value == i64::from(P2_SCHEMA_VERSION) => Ok(()),
        value => Err(P2Error::CorruptStore(format!(
            "unsupported P2 schema version {value}"
        ))),
    }
}

fn storage_error(error: &rusqlite::Error) -> P2Error {
    P2Error::Storage(error.to_string())
}

fn to_sql_millis(value: u64) -> Result<i64, P2Error> {
    i64::try_from(value).map_err(|_| P2Error::InvalidArgument("timestamp is too large".to_owned()))
}

fn from_sql_millis(value: i64) -> Result<u64, P2Error> {
    u64::try_from(value).map_err(|_| P2Error::CorruptStore("negative timestamp".to_owned()))
}

#[must_use]
pub fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use p256::{
        ecdsa::{Signature, SigningKey, signature::Signer},
        pkcs8::EncodePublicKey,
    };

    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "silent-disco-p2-{name}-{}-{}.sqlite3",
            std::process::id(),
            current_unix_millis()
        ))
    }

    fn remove_database(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    fn signed_payload(now: u64) -> (String, Vec<u8>) {
        let signing_key = SigningKey::from_bytes((&[7_u8; 32]).into()).expect("fixed key");
        let public_key = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("public DER")
            .as_bytes()
            .to_vec();
        let unsigned = prepare_unsigned_qr(&QrInvitationInput {
            session_id: "550e8400-e29b-41d4-a716-446655440000".to_owned(),
            session_name: "Rooftop Disco".to_owned(),
            host_name: "Phillip's host".to_owned(),
            host_public_key_der: public_key.clone(),
            approval_mode: "invite_code".to_owned(),
            invite_code: Some("4826".to_owned()),
            issued_at_ms: now,
            expires_at_ms: now + 300_000,
            nonce: "nonce-1234567890abcdef".to_owned(),
        })
        .expect("unsigned payload");
        let signature: Signature = signing_key.sign(unsigned.as_bytes());
        let payload = finalize_qr(
            &unsigned,
            &URL_SAFE_NO_PAD.encode(signature.to_der().as_bytes()),
        )
        .expect("signed payload");
        (payload, public_key)
    }

    #[test]
    fn recent_listener_query_is_bounded_and_expires_old_rows() {
        let path = test_path("recent");
        let mut store = P2Store::open(&path).expect("open");
        let now = 2_000_000_000_u64;
        for (index, age) in [1_000_u64, DEFAULT_RECENT_MAX_AGE_MS + 1]
            .into_iter()
            .enumerate()
        {
            store
                .record_session(&RecentSessionRecord {
                    session_id: format!("550e8400-e29b-41d4-a716-44665544000{index}"),
                    role: RecentSessionRole::Listener,
                    session_name: format!("Session {index}"),
                    host_name: "Host".to_owned(),
                    host_fingerprint: None,
                    started_at_ms: now - age - 100,
                    ended_at_ms: now - age,
                    outcome: RecentSessionOutcome::Completed,
                })
                .expect("record");
        }
        let recent = store
            .list_recent_listener_sessions(now, DEFAULT_RECENT_MAX_AGE_MS, 5)
            .expect("recent");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].session_name, "Session 0");
        drop(store);
        remove_database(&path);
    }

    #[test]
    fn trusted_hosts_are_keyed_by_public_key_not_display_name() {
        let path = test_path("trust");
        let mut store = P2Store::open(&path).expect("open");
        for (byte, name) in [(3_u8, "Same name"), (4_u8, "Same name")] {
            let signing_key = SigningKey::from_bytes((&[byte; 32]).into()).expect("key");
            let public_key = signing_key
                .verifying_key()
                .to_public_key_der()
                .expect("DER")
                .as_bytes()
                .to_vec();
            store
                .trust_host(&TrustedHostRecord {
                    fingerprint: public_key_fingerprint(&public_key),
                    display_name: name.to_owned(),
                    public_key_der: public_key,
                    first_verified_ms: 1,
                    last_verified_ms: 2,
                })
                .expect("trust");
        }
        assert_eq!(store.list_trusted_hosts().expect("list").len(), 2);
        drop(store);
        remove_database(&path);
    }

    #[test]
    fn qr_signature_expiry_tampering_and_replay_are_enforced() {
        let path = test_path("qr");
        let now = 2_000_000_000_u64;
        let (payload, public_key) = signed_payload(now);
        let mut store = P2Store::open(&path).expect("open");
        let validated = store
            .validate_and_consume_qr(&payload, now + 1_000)
            .expect("validate");
        assert_eq!(
            validated.host_fingerprint,
            public_key_fingerprint(&public_key)
        );
        assert_eq!(
            store.validate_and_consume_qr(&payload, now + 2_000),
            Err(P2Error::ReplayedQr)
        );

        let (another, _) = signed_payload(now + 10_000);
        let tampered = another.replace("Rooftop Disco", "Basement Disco");
        assert_eq!(
            store.validate_and_consume_qr(&tampered, now + 11_000),
            Err(P2Error::InvalidSignature)
        );
        assert_eq!(
            store.validate_and_consume_qr(
                &another,
                now + MAX_QR_LIFETIME_MS + QR_CLOCK_SKEW_MS + 20_000
            ),
            Err(P2Error::ExpiredQr)
        );
        drop(store);
        remove_database(&path);
    }
}
