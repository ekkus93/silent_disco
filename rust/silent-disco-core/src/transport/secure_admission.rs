//! Control-connection admission over the owner-thread TLS channel.
//!
//! The host accepts a control socket, completes TLS 1.3 pinned to nothing
//! (the listener pins the host), and then runs the listener possession proof as
//! frames on the channel. A listener that does not prove its pinned-for key is
//! refused before any join reaches the actor. On success the channel is kept
//! open for the join traffic that follows.
//!
//! Scope: admission only. The production host still runs plaintext protocol v3
//! on its existing path; this module is not yet called from `host_workers`.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use rustls::{ClientConnection, Connection, ServerConnection, pki_types::ServerName};

use crate::domain::SessionId;

use super::secure_control_tls::{
    HOST_SERVER_NAME, HostTlsIdentity, host_server_config, pinned_client_config,
};
use super::secure_frame_channel::{SecureFrameChannel, spawn_secure_frame_channel};
use super::secure_identity::{PendingListenerProof, VerifiedListenerKey};
use super::secure_listener_proof::{
    ListenerProofSigner, ProofExchangeError, build_listener_response, client_binding,
    server_binding,
};

/// Proof generation used for control admission. Both ends must agree.
pub const SECURE_ADMISSION_GENERATION: u64 = 1;

/// Bound on connect, handshake and each frame during admission.
const ADMISSION_TIMEOUT: Duration = Duration::from_secs(10);

/// A listener whose proof verified, with its open channel.
pub struct AdmittedListener {
    pub verified: VerifiedListenerKey,
    pub channel: SecureFrameChannel,
}

fn fresh_nonce() -> Result<[u8; 32], ProofExchangeError> {
    let mut nonce = [0_u8; 32];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut nonce)
        .map_err(|_| ProofExchangeError::RandomUnavailable)?;
    Ok(nonce)
}

/// Splits a listener response frame into its SPKI and signature. The frame
/// must hold exactly two length-prefixed fields with nothing after them.
fn parse_response(frame: &[u8]) -> Result<(Vec<u8>, Vec<u8>), ProofExchangeError> {
    let mut rest = frame;
    let mut field = || -> Result<Vec<u8>, ProofExchangeError> {
        if rest.len() < 2 {
            return Err(ProofExchangeError::MalformedFrame);
        }
        let length = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
        if length == 0 || rest.len() < 2 + length {
            return Err(ProofExchangeError::MalformedFrame);
        }
        let value = rest[2..2 + length].to_vec();
        rest = &rest[2 + length..];
        Ok(value)
    };
    let spki = field()?;
    let signature = field()?;
    if !rest.is_empty() {
        return Err(ProofExchangeError::MalformedFrame);
    }
    Ok((spki, signature))
}

/// Host side: accepts one control socket, completes TLS 1.3 with `host`, and
/// verifies the listener's proof for `session` and `generation` before
/// returning. The channel is returned open only on success.
///
/// # Errors
/// Returns an error if the handshake, a frame, or the proof fails or times out.
pub fn admit_listener(
    tcp: TcpStream,
    host: &HostTlsIdentity,
    session: &SessionId,
    generation: u64,
) -> Result<AdmittedListener, ProofExchangeError> {
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(ADMISSION_TIMEOUT))?;
    tcp.set_write_timeout(Some(ADMISSION_TIMEOUT))?;
    let config = host_server_config(host.certificate.clone(), host.signing_key.clone())?;
    let mut connection = ServerConnection::new(config)?;
    connection.complete_io(&mut tcp)?;
    let binding = server_binding(&connection)?;
    let channel = spawn_secure_frame_channel(tcp, Connection::Server(connection))
        .map_err(|error| ProofExchangeError::Channel(error.to_string()))?;

    let nonce = fresh_nonce()?;
    let mut challenge =
        PendingListenerProof::new(session.clone(), generation, &host.spki, nonce, binding)?;
    channel
        .send(nonce.to_vec())
        .map_err(|error| ProofExchangeError::Channel(error.to_string()))?;
    let response = channel
        .recv_timeout(ADMISSION_TIMEOUT)
        .map_err(|error| ProofExchangeError::Channel(error.to_string()))?;
    let (listener_spki, signature) = parse_response(&response)?;
    let verified = challenge.verify_and_consume(&listener_spki, &signature)?;
    Ok(AdmittedListener { verified, channel })
}

/// Listener side: connects to `address`, pins `host_spki`, answers the host's
/// challenge with a proof from `signer`, and returns the open channel.
///
/// # Errors
/// Returns an error if the TLS handshake (including the pin), the challenge or
/// the signer fails.
pub fn dial_and_prove(
    address: SocketAddr,
    host_spki: &[u8],
    session: &SessionId,
    generation: u64,
    signer: &dyn ListenerProofSigner,
) -> Result<SecureFrameChannel, ProofExchangeError> {
    let mut tcp = TcpStream::connect_timeout(&address, ADMISSION_TIMEOUT)?;
    tcp.set_read_timeout(Some(ADMISSION_TIMEOUT))?;
    tcp.set_write_timeout(Some(ADMISSION_TIMEOUT))?;
    let config = pinned_client_config(host_spki)?;
    let name =
        ServerName::try_from(HOST_SERVER_NAME).map_err(|_| ProofExchangeError::MalformedFrame)?;
    let mut connection = ClientConnection::new(config, name)?;
    connection.complete_io(&mut tcp)?;
    let binding = client_binding(&connection)?;
    let channel = spawn_secure_frame_channel(tcp, Connection::Client(connection))
        .map_err(|error| ProofExchangeError::Channel(error.to_string()))?;

    let nonce = channel
        .recv_timeout(ADMISSION_TIMEOUT)
        .map_err(|error| ProofExchangeError::Channel(error.to_string()))?;
    let nonce: [u8; 32] = nonce
        .as_slice()
        .try_into()
        .map_err(|_| ProofExchangeError::MalformedFrame)?;
    let frame = build_listener_response(session, generation, host_spki, nonce, binding, signer)?;
    channel
        .send(frame)
        .map_err(|error| ProofExchangeError::Channel(error.to_string()))?;
    Ok(channel)
}
