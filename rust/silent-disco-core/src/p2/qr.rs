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
