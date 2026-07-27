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
