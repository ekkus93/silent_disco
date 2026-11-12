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
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::Error as TlsError;

use super::secure_control_tls::{HostTlsIdentity, host_server_config, pinned_client_config};

/// ALPN of the v4 transport. Both sides require it, so a peer speaking any
/// other protocol cannot complete the handshake.
pub const ALPN_V4: &[u8] = b"silent-disco/4";

/// Keep-alive interval on both ends. An approved listener waiting for Play
/// sends nothing of its own, so without keep-alives its idle connection would
/// time out before playback starts.
pub const QUIC_KEEP_ALIVE: Duration = Duration::from_secs(2);

/// A connection that hears nothing (not even a keep-alive) for this long is
/// closed. This is how the v4 host notices a listener that vanished without
/// closing, in place of the socket host's inbound-silence eviction; it
/// matches `DEFAULT_PEER_INBOUND_SILENCE_TIMEOUT`.
pub const QUIC_IDLE_TIMEOUT: Duration = Duration::from_secs(8);

fn liveness_transport_config() -> Result<quinn::TransportConfig, QuicSetupError> {
    let idle = quinn::IdleTimeout::try_from(QUIC_IDLE_TIMEOUT)
        .map_err(|error| QuicSetupError::Quic(error.to_string()))?;
    let mut transport = quinn::TransportConfig::default();
    transport
        .keep_alive_interval(Some(QUIC_KEEP_ALIVE))
        .max_idle_timeout(Some(idle));
    Ok(transport)
}

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
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    config.transport_config(Arc::new(liveness_transport_config()?));
    Ok(config)
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
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(Arc::new(liveness_transport_config()?));
    Ok(config)
}
