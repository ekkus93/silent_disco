//! v4 admission over QUIC (ADR "UDP PCM and synchronization design", phase 2,
//! first step): the listener's possession proof on the connection's first
//! bidirectional stream, bound to that QUIC connection's TLS exporter.
//!
//! The exchange is the TCP one (`secure_listener_proof`) with two changes:
//! the listener opens the stream and starts it with [`V4_HELLO`], so a peer
//! that is not a v4 listener is refused before any challenge is issued; and
//! the binding comes from the QUIC connection's exporter, so a proof made on
//! one connection cannot be replayed on another. After admission the same
//! stream stays open as the control channel.

use std::time::Duration;

use super::secure_identity::{PendingListenerProof, VerifiedListenerKey};
use super::secure_listener_proof::{
    LISTENER_PROOF_EXPORTER_LABEL, ListenerProofSigner, MAX_FIELD_BYTES, ProofExchangeError,
    build_listener_response, fresh_nonce,
};
use crate::domain::SessionId;

/// First bytes on a v4 listener's control stream: protocol name and version.
pub const V4_HELLO: &[u8; 16] = b"silent-disco/v4\0";

/// Longest the whole admission exchange may take, so a stalled or silent
/// peer cannot hold a connection open.
pub const QUIC_ADMISSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The connection's v4 control stream, open after admission.
#[derive(Debug)]
pub struct QuicControlStream {
    /// Sending half.
    pub send: quinn::SendStream,
    /// Receiving half.
    pub recv: quinn::RecvStream,
}

fn channel(reason: impl std::fmt::Display) -> ProofExchangeError {
    ProofExchangeError::Channel(reason.to_string())
}

/// Exporter binding of a QUIC connection for the listener proof.
///
/// # Errors
/// Returns [`ProofExchangeError::Channel`] if the exporter is unavailable.
pub fn quic_binding(connection: &quinn::Connection) -> Result<[u8; 32], ProofExchangeError> {
    let mut binding = [0_u8; 32];
    connection
        .export_keying_material(&mut binding, LISTENER_PROOF_EXPORTER_LABEL, b"")
        .map_err(|_| channel("QUIC exporter is unavailable"))?;
    Ok(binding)
}

async fn read_field(recv: &mut quinn::RecvStream) -> Result<Vec<u8>, ProofExchangeError> {
    let mut length_bytes = [0_u8; 2];
    recv.read_exact(&mut length_bytes).await.map_err(channel)?;
    let length = usize::from(u16::from_be_bytes(length_bytes));
    if length == 0 || length > MAX_FIELD_BYTES {
        return Err(ProofExchangeError::MalformedFrame);
    }
    let mut field = vec![0_u8; length];
    recv.read_exact(&mut field).await.map_err(channel)?;
    Ok(field)
}

/// Host side: accept the listener's control stream, check its v4 hello, issue
/// a fresh nonce and verify the proof against this connection's binding.
///
/// # Errors
/// Returns an error for a non-v4 hello, a malformed frame, a proof that does
/// not verify for this session, generation, nonce and connection, a stream
/// failure, or the exchange exceeding [`QUIC_ADMISSION_TIMEOUT`].
pub async fn quic_host_admit(
    connection: &quinn::Connection,
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
) -> Result<(VerifiedListenerKey, QuicControlStream), ProofExchangeError> {
    tokio::time::timeout(QUIC_ADMISSION_TIMEOUT, async {
        let (mut send, mut recv) = connection.accept_bi().await.map_err(channel)?;
        let mut hello = [0_u8; V4_HELLO.len()];
        recv.read_exact(&mut hello).await.map_err(channel)?;
        if &hello != V4_HELLO {
            return Err(channel("peer did not open a v4 control stream"));
        }
        let binding = quic_binding(connection)?;
        let nonce = fresh_nonce()?;
        let mut challenge =
            PendingListenerProof::new(session.clone(), generation, host_spki, nonce, binding)?;
        send.write_all(&nonce).await.map_err(channel)?;
        let listener_spki = read_field(&mut recv).await?;
        let signature = read_field(&mut recv).await?;
        let verified = challenge.verify_and_consume(&listener_spki, &signature)?;
        Ok((verified, QuicControlStream { send, recv }))
    })
    .await
    .map_err(|_| channel("admission timed out"))?
}

/// Listener side: open the control stream, send the v4 hello, and answer the
/// host's nonce with a proof from `signer` bound to this connection.
///
/// # Errors
/// Returns an error for a stream failure, an invalid challenge, a signer
/// failure, or the exchange exceeding [`QUIC_ADMISSION_TIMEOUT`].
pub async fn quic_listener_prove(
    connection: &quinn::Connection,
    session: &SessionId,
    generation: u64,
    host_spki: &[u8],
    signer: &dyn ListenerProofSigner,
) -> Result<QuicControlStream, ProofExchangeError> {
    tokio::time::timeout(QUIC_ADMISSION_TIMEOUT, async {
        let (mut send, mut recv) = connection.open_bi().await.map_err(channel)?;
        send.write_all(V4_HELLO).await.map_err(channel)?;
        let mut nonce = [0_u8; 32];
        recv.read_exact(&mut nonce).await.map_err(channel)?;
        let binding = quic_binding(connection)?;
        let frame =
            build_listener_response(session, generation, host_spki, nonce, binding, signer)?;
        send.write_all(&frame).await.map_err(channel)?;
        Ok(QuicControlStream { send, recv })
    })
    .await
    .map_err(|_| channel("admission timed out"))?
}
