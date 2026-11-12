//! Length-prefixed control frames on a QUIC stream.
//!
//! Each frame is a 4-byte big-endian length followed by that many bytes. The
//! length is bounded by [`MAX_QUIC_CONTROL_FRAME_BYTES`], so a peer cannot make
//! the reader allocate without limit.

use crate::protocol::{FRAME_HEADER_BYTES, MAX_CONTROL_PAYLOAD_BYTES};

/// Largest control frame accepted on a v4 control stream: one protocol frame
/// header plus the largest control payload.
pub const MAX_QUIC_CONTROL_FRAME_BYTES: usize = FRAME_HEADER_BYTES + MAX_CONTROL_PAYLOAD_BYTES;

/// Why a control frame could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuicFrameError {
    /// The peer finished or reset the stream at a frame boundary.
    Finished,
    /// The frame is empty or longer than [`MAX_QUIC_CONTROL_FRAME_BYTES`].
    Oversized(usize),
    /// The stream or connection failed.
    Stream(String),
}

impl std::fmt::Display for QuicFrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finished => formatter.write_str("control stream finished"),
            Self::Oversized(length) => write!(
                formatter,
                "control frame of {length} bytes is outside 1..={MAX_QUIC_CONTROL_FRAME_BYTES}"
            ),
            Self::Stream(reason) => write!(formatter, "control stream failed: {reason}"),
        }
    }
}

impl std::error::Error for QuicFrameError {}

/// Writes one length-prefixed frame.
///
/// # Errors
/// Returns [`QuicFrameError::Oversized`] for an empty or too-long frame, or
/// [`QuicFrameError::Stream`] if the write fails.
pub async fn write_frame(send: &mut quinn::SendStream, frame: &[u8]) -> Result<(), QuicFrameError> {
    if frame.is_empty() || frame.len() > MAX_QUIC_CONTROL_FRAME_BYTES {
        return Err(QuicFrameError::Oversized(frame.len()));
    }
    let length = u32::try_from(frame.len()).map_err(|_| QuicFrameError::Oversized(frame.len()))?;
    send.write_all(&length.to_be_bytes())
        .await
        .map_err(|error| QuicFrameError::Stream(error.to_string()))?;
    send.write_all(frame)
        .await
        .map_err(|error| QuicFrameError::Stream(error.to_string()))
}

/// Reads one length-prefixed frame.
///
/// # Errors
/// Returns [`QuicFrameError::Finished`] when the stream ends cleanly before a
/// new frame, [`QuicFrameError::Oversized`] for a bad length, or
/// [`QuicFrameError::Stream`] for a failure or a frame cut off mid-way.
pub async fn read_frame(recv: &mut quinn::RecvStream) -> Result<Vec<u8>, QuicFrameError> {
    let mut length_bytes = [0_u8; 4];
    match recv.read_exact(&mut length_bytes).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Err(QuicFrameError::Finished),
        Err(error) => return Err(QuicFrameError::Stream(error.to_string())),
    }
    let length = usize::try_from(u32::from_be_bytes(length_bytes)).unwrap_or(usize::MAX);
    if length == 0 || length > MAX_QUIC_CONTROL_FRAME_BYTES {
        return Err(QuicFrameError::Oversized(length));
    }
    let mut frame = vec![0_u8; length];
    recv.read_exact(&mut frame)
        .await
        .map_err(|error| QuicFrameError::Stream(error.to_string()))?;
    Ok(frame)
}
