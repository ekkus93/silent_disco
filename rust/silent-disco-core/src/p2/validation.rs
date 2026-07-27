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
