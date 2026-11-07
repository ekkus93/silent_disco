//! Protocol frames over an admitted TLS control channel.
//!
//! Each channel message is exactly one encoded protocol frame, the same bytes
//! the plaintext v3 path uses, so the message set and its validation are
//! unchanged. The session adds the TLS-specific checks: outbound frames must
//! belong to this session, and inbound frames are decoded with the session
//! policy, so a frame for another session is refused.
//!
//! Scope: frame exchange only. Feeding these frames into the host actor, route
//! authorization for datagrams, and the listener's join state machine are the
//! next steps of the TLS join path.

use std::time::Duration;

use crate::domain::SessionId;
use crate::protocol::{DecodePolicy, ProtocolDecoder, ProtocolError, ProtocolFrame, encode_frame};

use super::secure_frame_channel::{SecureChannelError, SecureFrameChannel};

#[derive(Debug)]
pub enum SecureSessionError {
    /// The frame belongs to a different session, or is not valid for this session.
    Protocol(ProtocolError),
    /// The channel failed, timed out, or was closed.
    Channel(SecureChannelError),
}

impl std::fmt::Display for SecureSessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol(error) => write!(formatter, "protocol frame refused: {error:?}"),
            Self::Channel(error) => write!(formatter, "secure control channel failed: {error}"),
        }
    }
}

impl std::error::Error for SecureSessionError {}

/// An admitted control channel bound to one session.
pub struct SecureControlSession {
    channel: SecureFrameChannel,
    session: SessionId,
    decoder: ProtocolDecoder,
}

impl SecureControlSession {
    #[must_use]
    pub fn new(channel: SecureFrameChannel, session: SessionId) -> Self {
        Self {
            channel,
            session,
            decoder: ProtocolDecoder::default(),
        }
    }

    /// Sends one protocol frame. The frame must belong to this session.
    ///
    /// # Errors
    /// Returns `Protocol` for a frame from another session or one that cannot
    /// be encoded, and `Channel` if the channel refuses it.
    pub fn send_frame(&self, frame: &ProtocolFrame) -> Result<(), SecureSessionError> {
        if frame.session_id() != &self.session {
            return Err(SecureSessionError::Protocol(
                ProtocolError::UnauthorizedSession,
            ));
        }
        let bytes = encode_frame(frame).map_err(SecureSessionError::Protocol)?;
        self.channel
            .send(bytes)
            .map_err(SecureSessionError::Channel)
    }

    /// Receives and decodes the next protocol frame within `timeout`.
    ///
    /// # Errors
    /// Returns `Channel` on timeout or close, and `Protocol` for a frame that
    /// fails decoding or belongs to another session.
    pub fn recv_frame(&mut self, timeout: Duration) -> Result<ProtocolFrame, SecureSessionError> {
        let bytes = self
            .channel
            .recv_timeout(timeout)
            .map_err(SecureSessionError::Channel)?;
        let policy = DecodePolicy {
            expected_session_id: Some(&self.session),
            minimum_audio_sequence: None,
        };
        self.decoder
            .decode(&bytes, policy)
            .map_err(SecureSessionError::Protocol)
    }

    /// Sends raw bytes as one channel message, bypassing the session checks.
    /// Test-only: it lets tests model a peer that does not follow the protocol.
    ///
    /// # Errors
    /// Returns `Channel` if the channel refuses the bytes.
    #[cfg(test)]
    pub fn raw_send_for_test(&self, bytes: Vec<u8>) -> Result<(), SecureSessionError> {
        self.channel
            .send(bytes)
            .map_err(SecureSessionError::Channel)
    }

    /// Closes the channel.
    pub fn close(self) {
        self.channel.close();
    }
}
