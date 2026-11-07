//! Listener possession proof carried over a TLS 1.3 connection.
//!
//! After the TLS handshake the host sends a fresh 32-byte nonce. The listener
//! replies with its SPKI and a DER ECDSA signature over the canonical proof
//! transcript from `secure_identity`. The transcript includes the TLS exporter
//! value for this connection, so a proof captured on one connection cannot be
//! replayed on another.
//!
//! The listener signs through [`ListenerProofSigner`], not a raw key. A desktop
//! listener can implement it with a key file; an Android listener implements
//! it over the Keystore key, which never leaves Keystore.
//!
//! Scope: this module proves possession of the listener key on one connection.
//! It does not decide whether the listener is trusted; that is the host's
//! durable trust policy.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use p256::{
    ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer},
    pkcs8::EncodePublicKey,
};
use rustls::pki_types::CertificateDer;
use rustls::{
    ClientConnection, ServerConnection, StreamOwned, pki_types::ServerName,
    sign::SigningKey as RustlsSigningKey,
};

use crate::domain::SessionId;

use super::secure_control_tls::{HOST_SERVER_NAME, host_server_config, pinned_client_config};
use super::secure_identity::{ListenerProofError, PendingListenerProof, VerifiedListenerKey};

/// TLS exporter label for the listener possession proof. Changing it changes
/// every binding, so it is part of the protocol version.
pub const LISTENER_PROOF_EXPORTER_LABEL: &[u8] = b"silent-disco listener proof v1";

/// Bound on each length-prefixed field. A P-256 SPKI is about 91 bytes and a
/// DER ECDSA signature is at most 72 bytes; the limit leaves room for variation
/// while refusing hostile lengths before allocation.
const MAX_FIELD_BYTES: usize = 256;

/// Socket timeouts for the proof exchange, so a stalled peer cannot hold a
/// thread indefinitely.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(10);

/// The listener's proof key, held wherever the platform keeps it.
pub trait ListenerProofSigner {
    /// `SubjectPublicKeyInfo` DER of the key the listener proves.
    ///
    /// # Errors
    /// Returns [`ProofExchangeError::SignerFailed`] if the key cannot be read.
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError>;

    /// DER ECDSA-SHA256 signature over `transcript`.
    ///
    /// # Errors
    /// Returns [`ProofExchangeError::SignerFailed`] if signing fails.
    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError>;
}

impl ListenerProofSigner for SigningKey {
    fn spki_der(&self) -> Result<Vec<u8>, ProofExchangeError> {
        self.verifying_key()
            .to_public_key_der()
            .map(|der| der.as_bytes().to_vec())
            .map_err(|_| ProofExchangeError::InvalidListenerKey)
    }

    fn sign_der(&self, transcript: &[u8]) -> Result<Vec<u8>, ProofExchangeError> {
        let signature: Signature = self.sign(transcript);
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

#[derive(Debug)]
pub enum ProofExchangeError {
    Io(std::io::Error),
    Tls(rustls::Error),
    Proof(ListenerProofError),
    MalformedFrame,
    RandomUnavailable,
    InvalidListenerKey,
    /// The platform signer could not read its key or sign.
    SignerFailed(String),
    /// The frame channel failed or timed out during the exchange.
    Channel(String),
}

impl std::fmt::Display for ProofExchangeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "proof exchange I/O failed: {error}"),
            Self::Tls(error) => write!(formatter, "TLS failed during the proof exchange: {error}"),
            Self::Proof(error) => write!(formatter, "listener proof rejected: {error:?}"),
            Self::MalformedFrame => formatter.write_str("malformed proof frame"),
            Self::RandomUnavailable => formatter.write_str("system random source is unavailable"),
            Self::InvalidListenerKey => formatter.write_str("listener key is not a P-256 key"),
            Self::SignerFailed(reason) => write!(formatter, "listener key signer failed: {reason}"),
            Self::Channel(reason) => write!(formatter, "proof channel failed: {reason}"),
        }
    }
}

impl std::error::Error for ProofExchangeError {}

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

/// Builds the listener's response frame for one challenge.
///
/// # Errors
/// Returns a proof error for an invalid challenge, a malformed key, or a
/// signer failure.
pub fn build_listener_response(
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
    nonce: [u8; 32],
    binding: [u8; 32],
    signer: &dyn ListenerProofSigner,
) -> Result<Vec<u8>, ProofExchangeError> {
    let listener_spki = signer.spki_der()?;
    let challenge =
        PendingListenerProof::new(session.clone(), generation, host_spki, nonce, binding)?;
    let transcript = challenge.signing_transcript(&listener_spki)?;
    let signature = signer.sign_der(&transcript)?;
    let mut frame = Vec::with_capacity(listener_spki.len() + signature.len() + 4);
    write_field(&mut frame, &listener_spki)?;
    write_field(&mut frame, &signature)?;
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

/// Listener side: read the host's nonce and answer with a proof from `signer`.
///
/// `io` must be the encrypted TLS stream, not the raw socket.
///
/// # Errors
/// Returns an error for I/O failure, an invalid challenge, or a signer failure.
pub fn listener_prove<IO: Read + Write>(
    io: &mut IO,
    binding: [u8; 32],
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
    signer: &dyn ListenerProofSigner,
) -> Result<(), ProofExchangeError> {
    let mut nonce = [0_u8; 32];
    io.read_exact(&mut nonce)?;
    let frame = build_listener_response(session, generation, host_spki, nonce, binding, signer)?;
    io.write_all(&frame)?;
    io.flush()?;
    Ok(())
}

/// Listener side, end to end: connect to `address`, pin `host_spki` in TLS 1.3,
/// and prove the listener key over that connection.
///
/// # Errors
/// Returns an error if the TLS handshake fails (including a host that does not
/// hold the pinned key), the exchange fails, or the signer fails.
pub fn connect_and_prove(
    address: SocketAddr,
    host_spki: &[u8],
    session: &SessionId,
    generation: u64,
    signer: &dyn ListenerProofSigner,
) -> Result<(), ProofExchangeError> {
    let mut tcp = TcpStream::connect_timeout(&address, EXCHANGE_TIMEOUT)?;
    tcp.set_read_timeout(Some(EXCHANGE_TIMEOUT))?;
    tcp.set_write_timeout(Some(EXCHANGE_TIMEOUT))?;
    let config = pinned_client_config(host_spki)?;
    let name =
        ServerName::try_from(HOST_SERVER_NAME).map_err(|_| ProofExchangeError::MalformedFrame)?;
    let mut connection = ClientConnection::new(config, name)?;
    connection.complete_io(&mut tcp)?;
    let binding = client_binding(&connection)?;
    let mut stream = StreamOwned::new(connection, tcp);
    listener_prove(&mut stream, binding, session, generation, host_spki, signer)
}

/// Host side, end to end: accept one listener on `listener`, complete TLS 1.3
/// with the host certificate and signing key, and verify the listener's proof.
///
/// # Errors
/// Returns an error if the handshake fails or the listener's proof does not
/// verify for this session and generation.
pub fn accept_and_verify(
    listener: &TcpListener,
    host_certificate: &CertificateDer<'static>,
    host_signing_key: &Arc<dyn RustlsSigningKey>,
    host_spki: &[u8],
    session: &SessionId,
    generation: u64,
) -> Result<VerifiedListenerKey, ProofExchangeError> {
    let (mut tcp, _) = listener.accept()?;
    tcp.set_read_timeout(Some(EXCHANGE_TIMEOUT))?;
    tcp.set_write_timeout(Some(EXCHANGE_TIMEOUT))?;
    let config = host_server_config(host_certificate.clone(), Arc::clone(host_signing_key))?;
    let mut connection = ServerConnection::new(config)?;
    connection.complete_io(&mut tcp)?;
    let binding = server_binding(&connection)?;
    let mut stream = StreamOwned::new(connection, tcp);
    host_verify_listener(&mut stream, binding, session, generation, host_spki)
}

/// The listener's SPKI bytes for an in-memory signing key (tests and the
/// desktop key file).
///
/// # Errors
/// Returns `InvalidListenerKey` if the key cannot be encoded.
pub fn listener_spki(signing_key: &SigningKey) -> Result<Vec<u8>, ProofExchangeError> {
    signing_key.spki_der()
}

/// Checks that SPKI bytes are a P-256 key.
///
/// # Errors
/// Returns `InvalidListenerKey` if the SPKI is not a P-256 key.
pub fn validate_listener_spki(listener_spki: &[u8]) -> Result<(), ProofExchangeError> {
    use p256::pkcs8::DecodePublicKey;
    VerifyingKey::from_public_key_der(listener_spki)
        .map(|_| ())
        .map_err(|_| ProofExchangeError::InvalidListenerKey)
}
