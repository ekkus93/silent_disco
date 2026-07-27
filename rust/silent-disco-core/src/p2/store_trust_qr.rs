impl P2Store {
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
        transaction
            .commit()
            .map_err(|error| storage_error(&error))?;

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
