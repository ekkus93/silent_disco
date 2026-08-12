//! Bounded persisted evidence for real Lab transport packets and receive-side
//! fault decisions (Block 41.1).
//!
//! The recorder stores bounded metadata and SHA-256 evidence only. Secret-bearing
//! join requests are canonically redacted before their frame hash is computed;
//! sync/audio and non-secret control frames use their full canonical encoding.
//! It never stores a raw encoded frame, an audio payload, or a peer device
//! identifier (only whether the peer had been identified). A single shared
//! mutex owns sequence allocation and append order so packet observations and
//! the decisions made from them cannot be reordered across separate logs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use silent_disco_core::protocol::{ControlMessage, ProtocolError, ProtocolFrame, encode_frame};
use silent_disco_core::transport::TransportEvent;
use std::fmt;
use std::sync::{Arc, Mutex};

pub(crate) const MAX_TRANSPORT_FACTS: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransportTrace {
    pub(crate) facts: Vec<TransportFact>,
    pub(crate) dropped_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransportFact {
    pub(crate) sequence: u64,
    pub(crate) entry: TransportFactKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum TransportFactKind {
    Packet {
        receiver_node: String,
        channel: String,
        observed_at_ms: u64,
        peer_identified: bool,
        message_kind: String,
        message_code: u16,
        session_id: String,
        encoded_length: u64,
        frame_sha256: String,
        frame_hash_scope: RecordedFrameHashScope,
        audio: Option<Box<RecordedAudioPacket>>,
    },
    FaultDecision {
        receiver_node: String,
        channel: String,
        observed_at_ms: u64,
        frame_sha256: String,
        packet_fact_sequence: u64,
        decided_at_ms: u64,
        profile: RecordedFaultProfile,
        decision: RecordedFaultDecision,
        deadline_ms: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordedAudioPacket {
    pub(crate) stream_id: String,
    pub(crate) sequence: u64,
    pub(crate) codec: u8,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) samples_per_packet: u32,
    pub(crate) first_sample_index: u64,
    pub(crate) host_presentation_time_ms: u64,
    pub(crate) payload_length: u64,
    pub(crate) payload_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecordedFrameHashScope {
    FullFrame,
    RedactedSensitiveFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordedFaultProfile {
    pub(crate) fixed_latency_ms: u64,
    pub(crate) jitter_ms: u64,
    pub(crate) loss_permille: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecordedFaultDecision {
    Pass,
    Drop,
    Hold,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PacketTraceIdentity {
    pub(super) receiver_node: String,
    pub(super) channel: String,
    pub(super) observed_at_ms: u64,
    pub(super) frame_sha256: String,
    pub(super) packet_fact_sequence: u64,
}

#[derive(Debug)]
pub(crate) enum TransportTraceError {
    StatePoisoned,
    SequenceExhausted,
    DropCounterExhausted,
    LengthOutOfRange,
    Encode(ProtocolError),
}

impl fmt::Display for TransportTraceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StatePoisoned => formatter.write_str("transport trace state mutex was poisoned"),
            Self::SequenceExhausted => formatter.write_str("transport trace sequence exhausted"),
            Self::DropCounterExhausted => {
                formatter.write_str("transport trace dropped-fact counter exhausted")
            }
            Self::LengthOutOfRange => {
                formatter.write_str("transport trace byte length did not fit in u64")
            }
            Self::Encode(error) => write!(formatter, "canonical frame encoding failed: {error}"),
        }
    }
}

impl std::error::Error for TransportTraceError {}

#[derive(Default)]
struct TransportTraceState {
    next_sequence: u64,
    facts: Vec<TransportFact>,
    dropped_count: u64,
}

#[derive(Clone, Default)]
pub(crate) struct TransportTraceRecorder {
    state: Arc<Mutex<TransportTraceState>>,
}

impl TransportTraceRecorder {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_packet(
        &self,
        receiver_node: &str,
        event: &TransportEvent,
    ) -> Result<Option<PacketTraceIdentity>, TransportTraceError> {
        let TransportEvent::FrameReceived {
            channel,
            peer,
            frame,
            received_at,
        } = event
        else {
            return Ok(None);
        };
        let encoded = encode_frame(frame).map_err(TransportTraceError::Encode)?;
        let (frame_sha256, frame_hash_scope) = recording_frame_hash(frame, &encoded)?;
        let audio = match frame {
            ProtocolFrame::Audio(packet) => Some(Box::new(RecordedAudioPacket {
                stream_id: packet.stream_id.to_string(),
                sequence: packet.sequence.get(),
                codec: packet.codec.code(),
                sample_rate: packet.sample_rate,
                channels: packet.channels,
                samples_per_packet: packet.samples_per_packet,
                first_sample_index: packet.first_sample_index.get(),
                host_presentation_time_ms: packet.host_presentation_time_ms.get(),
                payload_length: u64::try_from(packet.payload.len())
                    .map_err(|_| TransportTraceError::LengthOutOfRange)?,
                payload_sha256: sha256_hex(&packet.payload),
            })),
            ProtocolFrame::Control(_)
            | ProtocolFrame::SyncRequest(_)
            | ProtocolFrame::SyncResponse(_) => None,
        };
        let packet_fact_sequence = self.push(TransportFactKind::Packet {
            receiver_node: receiver_node.to_owned(),
            channel: channel.stable_name().to_owned(),
            observed_at_ms: received_at.get(),
            peer_identified: peer.device_id.is_some(),
            message_kind: frame.kind().stable_name().to_owned(),
            message_code: frame.kind().code(),
            session_id: frame.session_id().to_string(),
            encoded_length: u64::try_from(encoded.len())
                .map_err(|_| TransportTraceError::LengthOutOfRange)?,
            frame_sha256: frame_sha256.clone(),
            frame_hash_scope,
            audio,
        })?;
        Ok(Some(PacketTraceIdentity {
            receiver_node: receiver_node.to_owned(),
            channel: channel.stable_name().to_owned(),
            observed_at_ms: received_at.get(),
            frame_sha256,
            packet_fact_sequence,
        }))
    }

    pub(super) fn record_fault_decision(
        &self,
        packet: &PacketTraceIdentity,
        profile: RecordedFaultProfile,
        decision: RecordedFaultDecision,
        decided_at_ms: u64,
        deadline_ms: Option<u64>,
    ) -> Result<(), TransportTraceError> {
        self.push(TransportFactKind::FaultDecision {
            receiver_node: packet.receiver_node.clone(),
            channel: packet.channel.clone(),
            observed_at_ms: packet.observed_at_ms,
            frame_sha256: packet.frame_sha256.clone(),
            packet_fact_sequence: packet.packet_fact_sequence,
            decided_at_ms,
            profile,
            decision,
            deadline_ms,
        })?;
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<TransportTrace, TransportTraceError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TransportTraceError::StatePoisoned)?;
        Ok(TransportTrace {
            facts: state.facts.clone(),
            dropped_count: state.dropped_count,
        })
    }

    fn push(&self, entry: TransportFactKind) -> Result<u64, TransportTraceError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| TransportTraceError::StatePoisoned)?;
        let sequence = state.next_sequence;
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(TransportTraceError::SequenceExhausted)?;
        if state.facts.len() >= MAX_TRANSPORT_FACTS {
            state.dropped_count = state
                .dropped_count
                .checked_add(1)
                .ok_or(TransportTraceError::DropCounterExhausted)?;
            return Ok(sequence);
        }
        state.facts.push(TransportFact { sequence, entry });
        Ok(sequence)
    }
}

fn recording_frame_hash(
    frame: &ProtocolFrame,
    encoded: &[u8],
) -> Result<(String, RecordedFrameHashScope), TransportTraceError> {
    if let ProtocolFrame::Control(ControlMessage::JoinRequest(request)) = frame
        && request.invite_code.is_some()
    {
        let mut redacted_request = request.clone();
        redacted_request.invite_code = None;
        let redacted = ProtocolFrame::Control(ControlMessage::JoinRequest(redacted_request));
        let redacted_encoded = encode_frame(&redacted).map_err(TransportTraceError::Encode)?;
        return Ok((
            sha256_hex(&redacted_encoded),
            RecordedFrameHashScope::RedactedSensitiveFields,
        ));
    }
    Ok((sha256_hex(encoded), RecordedFrameHashScope::FullFrame))
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests;
