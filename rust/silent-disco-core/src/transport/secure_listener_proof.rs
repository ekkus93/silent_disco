//! Listener possession proof carried over an established TLS 1.3 connection.
//!
//! After the host's TLS handshake, the host sends a fresh 32-byte nonce. The
//! listener replies with its SPKI and a signature over the canonical proof
//! transcript from `secure_identity`. That transcript includes the TLS exporter
//! value for this connection, so a proof captured on one connection cannot be
//! replayed on another: the exporter differs and so does the nonce.
//!
//! Callers compute the exporter value from the connection first and pass the
//! 32-byte binding in. The proof code never borrows the TLS connection and the
//! stream together.
//!
//! Scope: this module proves possession of the listener key on one connection.
//! It does not decide whether the listener is trusted; that is the host's
//! durable trust policy. Nothing here is wired into the control socket yet.

use std::io::{Read, Write};

use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::EncodePublicKey,
};
use rustls::{ClientConnection, ServerConnection};

use crate::domain::SessionId;

use super::secure_identity::{ListenerProofError, PendingListenerProof, VerifiedListenerKey};

/// TLS exporter label for the listener possession proof. Changing it changes
/// every binding, so it is part of the protocol version.
pub const LISTENER_PROOF_EXPORTER_LABEL: &[u8] = b"silent-disco listener proof v1";

/// Bound on each length-prefixed field. A P-256 SPKI is about 91 bytes and a
/// DER ECDSA signature is at most 72 bytes; the limit leaves room for variation
/// while refusing hostile lengths before allocation.
const MAX_FIELD_BYTES: usize = 256;

#[derive(Debug)]
pub enum ProofExchangeError {
    Io(std::io::Error),
    Tls(rustls::Error),
    Proof(ListenerProofError),
    MalformedFrame,
    RandomUnavailable,
    InvalidListenerKey,
}

impl From<std::io::Error> for ProofExchangeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rustls::Error> for ProofExchangeError {
    fn from(error: rustls::Error) -> Self {
        Self::Tls(error)
    }
}

impl From<ListenerProofError> for ProofExchangeError {
    fn from(error: ListenerProofError) -> Self {
        Self::Proof(error)
    }
}

/// Exporter binding for the host side of a completed TLS handshake.
///
/// # Errors
/// Returns a rustls error if the handshake is not complete.
pub fn server_binding(connection: &ServerConnection) -> Result<[u8; 32], rustls::Error> {
    let mut binding = [0_u8; 32];
    connection.export_keying_material(&mut binding, LISTENER_PROOF_EXPORTER_LABEL, None)?;
    Ok(binding)
}

/// Exporter binding for the listener side of a completed TLS handshake.
///
/// # Errors
/// Returns a rustls error if the handshake is not complete.
pub fn client_binding(connection: &ClientConnection) -> Result<[u8; 32], rustls::Error> {
    let mut binding = [0_u8; 32];
    connection.export_keying_material(&mut binding, LISTENER_PROOF_EXPORTER_LABEL, None)?;
    Ok(binding)
}

fn fresh_nonce() -> Result<[u8; 32], ProofExchangeError> {
    let mut nonce = [0_u8; 32];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut nonce)
        .map_err(|_| ProofExchangeError::RandomUnavailable)?;
    Ok(nonce)
}

fn write_field<IO: Write>(io: &mut IO, field: &[u8]) -> Result<(), ProofExchangeError> {
    let length = u16::try_from(field.len()).map_err(|_| ProofExchangeError::MalformedFrame)?;
    io.write_all(&length.to_be_bytes())?;
    io.write_all(field)?;
    Ok(())
}

fn read_field<IO: Read>(io: &mut IO) -> Result<Vec<u8>, ProofExchangeError> {
    let mut length_bytes = [0_u8; 2];
    io.read_exact(&mut length_bytes)?;
    let length = usize::from(u16::from_be_bytes(length_bytes));
    if length == 0 || length > MAX_FIELD_BYTES {
        return Err(ProofExchangeError::MalformedFrame);
    }
    let mut field = vec![0_u8; length];
    io.read_exact(&mut field)?;
    Ok(field)
}

/// Builds the listener's response frame for one challenge. Exposed so callers
/// and tests can construct a frame for a specific challenge; a real listener
/// uses [`listener_prove`].
///
/// # Errors
/// Returns a proof error for a malformed key or an invalid challenge.
pub fn build_listener_response(
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
    nonce: [u8; 32],
    binding: [u8; 32],
    listener_spki: &[u8],
    signing_key: &SigningKey,
) -> Result<Vec<u8>, ProofExchangeError> {
    let challenge =
        PendingListenerProof::new(session.clone(), generation, host_spki, nonce, binding)?;
    let transcript = challenge.signing_transcript(listener_spki)?;
    let signature: Signature = signing_key.sign(&transcript);
    let der = signature.to_der();
    let mut frame = Vec::with_capacity(listener_spki.len() + der.as_bytes().len() + 4);
    let mut writer = std::io::Cursor::new(&mut frame);
    write_field(&mut writer, listener_spki)?;
    write_field(&mut writer, der.as_bytes())?;
    Ok(frame)
}

/// Host side: issue a fresh nonce on the established connection and verify the
/// listener's proof. Returns the verified listener key on success.
///
/// `io` must be the encrypted TLS stream, not the raw socket.
///
/// # Errors
/// Returns an error for I/O failure, a malformed frame, or a proof that does
/// not verify for this session, generation, nonce and connection binding.
pub fn host_verify_listener<IO: Read + Write>(
    io: &mut IO,
    binding: [u8; 32],
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
) -> Result<VerifiedListenerKey, ProofExchangeError> {
    let nonce = fresh_nonce()?;
    let mut challenge =
        PendingListenerProof::new(session.clone(), generation, host_spki, nonce, binding)?;
    io.write_all(&nonce)?;
    io.flush()?;
    let listener_spki = read_field(io)?;
    let signature = read_field(io)?;
    Ok(challenge.verify_and_consume(&listener_spki, &signature)?)
}

/// Listener side: read the host's nonce and answer with a proof signed by
/// `signing_key`, announcing `listener_spki` as the key it proves.
///
/// `io` must be the encrypted TLS stream, not the raw socket.
///
/// # Errors
/// Returns an error for I/O failure or an invalid listener key or challenge.
pub fn listener_prove<IO: Read + Write>(
    io: &mut IO,
    binding: [u8; 32],
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
    listener_spki: &[u8],
    signing_key: &SigningKey,
) -> Result<(), ProofExchangeError> {
    let mut nonce = [0_u8; 32];
    io.read_exact(&mut nonce)?;
    let frame = build_listener_response(
        session,
        generation,
        host_spki,
        nonce,
        binding,
        listener_spki,
        signing_key,
    )?;
    io.write_all(&frame)?;
    io.flush()?;
    Ok(())
}

/// Reads one listener response frame without verifying it. Used by tests that
/// capture a frame to replay it.
///
/// # Errors
/// Returns an error for I/O failure or a malformed frame.
pub fn read_listener_frame<IO: Read>(
    io: &mut IO,
) -> Result<(Vec<u8>, Vec<u8>), ProofExchangeError> {
    Ok((read_field(io)?, read_field(io)?))
}

/// Parses the listener's public key from a SPKI, for callers that must reject
/// a malformed key before running the exchange.
///
/// # Errors
/// Returns `InvalidListenerKey` if the SPKI is not a P-256 key.
pub fn validate_listener_spki(listener_spki: &[u8]) -> Result<(), ProofExchangeError> {
    use p256::ecdsa::VerifyingKey;
    use p256::pkcs8::DecodePublicKey;
    VerifyingKey::from_public_key_der(listener_spki)
        .map(|_| ())
        .map_err(|_| ProofExchangeError::InvalidListenerKey)
}

/// The listener's SPKI bytes for a signing key.
///
/// # Errors
/// Returns `InvalidListenerKey` if the key cannot be encoded.
pub fn listener_spki(signing_key: &SigningKey) -> Result<Vec<u8>, ProofExchangeError> {
    signing_key
        .verifying_key()
        .to_public_key_der()
        .map(|der| der.as_bytes().to_vec())
        .map_err(|_| ProofExchangeError::InvalidListenerKey)
}
