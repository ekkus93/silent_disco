//! QUIC endpoint configuration for the v4 secure transport (ADR "UDP PCM and
//! synchronization design", phase 1).
//!
//! One QUIC connection per host-listener pair, host as server. PCM and sync
//! probes are to travel as QUIC DATAGRAM frames, protected by the connection's
//! TLS 1.3 traffic secrets, so there is no hand-built UDP AEAD, nonce or key
//! derivation. This module only builds the endpoint configurations; it reuses
//! the host identity and the pinned-key verifier of the TLS control path, so a
//! QUIC handshake authenticates the host exactly as the TCP one does. Nothing
//! in production uses it yet.

use std::sync::Arc;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::Error as TlsError;

use super::secure_control_tls::{HostTlsIdentity, host_server_config, pinned_client_config};

/// ALPN of the v4 transport. Both sides require it, so a peer speaking any
/// other protocol cannot complete the handshake.
pub const ALPN_V4: &[u8] = b"silent-disco/4";

/// Why a QUIC endpoint configuration could not be built.
#[derive(Debug)]
pub enum QuicSetupError {
    /// The underlying rustls configuration was rejected.
    Tls(TlsError),
    /// The rustls configuration is not usable for QUIC (for example, it does
    /// not offer TLS 1.3 or its initial cipher suite).
    Quic(String),
}

impl std::fmt::Display for QuicSetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tls(error) => write!(formatter, "QUIC TLS configuration failed: {error}"),
            Self::Quic(message) => write!(formatter, "QUIC configuration failed: {message}"),
        }
    }
}

impl std::error::Error for QuicSetupError {}

impl From<TlsError> for QuicSetupError {
    fn from(error: TlsError) -> Self {
        Self::Tls(error)
    }
}

/// Host (server) endpoint configuration presenting `identity`, the same TLS
/// identity whose SPKI invitations pin.
///
/// # Errors
/// Returns [`QuicSetupError`] if the TLS configuration cannot be built or is
/// not usable for QUIC.
pub fn quic_server_config(
    identity: &HostTlsIdentity,
) -> Result<quinn::ServerConfig, QuicSetupError> {
    let mut tls = (*host_server_config(
        identity.certificate.clone(),
        Arc::clone(&identity.signing_key),
    )?)
    .clone();
    tls.alpn_protocols = vec![ALPN_V4.to_vec()];
    let quic =
        QuicServerConfig::try_from(tls).map_err(|error| QuicSetupError::Quic(error.to_string()))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic)))
}

/// Listener (client) endpoint configuration that accepts only the host holding
/// `host_spki`, the key pinned from its invitation.
///
/// # Errors
/// Returns [`QuicSetupError`] if the TLS configuration cannot be built or is
/// not usable for QUIC.
pub fn quic_client_config(host_spki: &[u8]) -> Result<quinn::ClientConfig, QuicSetupError> {
    let mut tls = (*pinned_client_config(host_spki)?).clone();
    tls.alpn_protocols = vec![ALPN_V4.to_vec()];
    let quic =
        QuicClientConfig::try_from(tls).map_err(|error| QuicSetupError::Quic(error.to_string()))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic)))
}
