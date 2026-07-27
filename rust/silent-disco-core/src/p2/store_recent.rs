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
}
