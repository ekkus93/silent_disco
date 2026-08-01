use core::fmt;
use std::error::Error;

use super::types::{AudioFormat, DecodedPcmChunk};
use crate::domain::{MonotonicMillis, PacketSequence, SampleIndex, SessionId, StreamId};
use crate::protocol::{
    AudioCodec, AudioDatagram, ControlMessage, FRAME_HEADER_BYTES, MAX_AUDIO_DATAGRAM_BYTES,
    ProtocolFrame, StreamStart,
};

/// Fixed-width fields written by the wire encoder for every audio datagram,
/// excluding the two length-prefixed session/stream identifiers and the
/// payload itself. Kept in lockstep with `protocol::codec::encode_audio`,
/// whose fixed fields (in byte order) are: sequence (8), codec (1),
/// sample rate (4), channels (2), samples per packet (4), first sample
/// index (8), host presentation time (8), payload length (2), and a CRC32
/// checksum (4).
const FIXED_AUDIO_DATAGRAM_FIELD_BYTES: usize = 8 + 1 + 4 + 2 + 4 + 8 + 8 + 2 + 4;
/// Each length-prefixed identifier costs a 2-byte length plus its bytes.
const IDENTIFIER_LENGTH_PREFIX_BYTES: usize = 2;

/// Default packet duration used when a caller does not select one explicitly.
pub const DEFAULT_PACKET_DURATION_MS: u32 = 20;
/// Smallest accepted explicit packet duration.
pub const MIN_PACKET_DURATION_MS: u32 = 1;
/// Largest accepted explicit packet duration.
pub const MAX_PACKET_DURATION_MS: u32 = 1_000;

/// Stable failure taxonomy for the streaming packetizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketizerErrorKind {
    /// Caller supplied an invalid packet-duration or resulting datagram size.
    InvalidConfiguration,
    /// A later chunk's format no longer matches the packetizer's fixed format.
    FormatMismatch,
    /// A chunk was pushed after the stream already reported end-of-stream.
    AlreadyFinished,
}

/// Typed packetizer failure with a user-safe diagnostic message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketizerError {
    /// Stable semantic failure category.
    pub kind: PacketizerErrorKind,
    /// Human-readable diagnostic.
    pub message: String,
}

impl PacketizerError {
    fn new(kind: PacketizerErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for PacketizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for PacketizerError {}

/// Result of pushing one decoded chunk through the packetizer.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct PacketizeOutcome {
    /// Zero or more full-width datagrams ready to broadcast, in order.
    pub frames: Vec<ProtocolFrame>,
    /// True once the final (possibly silence-padded) packet has been emitted.
    pub end_of_stream: bool,
}

impl fmt::Debug for PacketizeOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PacketizeOutcome")
            .field("frame_count", &self.frames.len())
            .field("end_of_stream", &self.end_of_stream)
            .finish()
    }
}

/// Bounded, incremental transform from decoded PCM chunks to shared-protocol
/// audio datagrams for exactly one host stream.
///
/// The packetizer never concatenates a full track: it retains at most one
/// packet's worth of leftover interleaved samples ("carry") between calls to
/// [`Self::push_chunk`]. A short final chunk is padded with silence to the
/// configured packet width so every emitted datagram satisfies the wire
/// encoder's exact `payload.len() == samples_per_packet * channels * 2`
/// requirement; there is no partial-width datagram on the wire.
#[derive(Debug)]
pub struct Packetizer {
    session_id: SessionId,
    stream_id: StreamId,
    format: AudioFormat,
    packet_duration_ms: u32,
    samples_per_packet: u32,
    host_start_time_ms: MonotonicMillis,
    next_sequence: u64,
    carry: Vec<i16>,
    carry_first_sample_index: Option<u64>,
    finished: bool,
}

impl Packetizer {
    /// Creates a packetizer for one stream, validating the resulting datagram
    /// fits the shared protocol's bounded audio-datagram size.
    ///
    /// # Errors
    ///
    /// Returns [`PacketizerErrorKind::InvalidConfiguration`] when
    /// `packet_duration_ms` is out of range, produces zero samples per
    /// packet, or the resulting datagram would exceed
    /// [`MAX_AUDIO_DATAGRAM_BYTES`] for these exact identifiers.
    pub fn new(
        session_id: SessionId,
        stream_id: StreamId,
        format: AudioFormat,
        packet_duration_ms: u32,
        host_start_time_ms: MonotonicMillis,
    ) -> Result<Self, PacketizerError> {
        if !(MIN_PACKET_DURATION_MS..=MAX_PACKET_DURATION_MS).contains(&packet_duration_ms) {
            return Err(PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "packet duration is outside the supported range",
            ));
        }
        let samples_per_packet =
            u32::try_from(u64::from(format.sample_rate_hz) * u64::from(packet_duration_ms) / 1_000)
                .unwrap_or(u32::MAX);
        if samples_per_packet == 0 {
            return Err(PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "packet duration produces zero samples per packet at this sample rate",
            ));
        }
        let payload_bytes =
            samples_per_packet as usize * format.samples_per_frame() * 2 /* i16 bytes */;
        let identifier_overhead = 2 * IDENTIFIER_LENGTH_PREFIX_BYTES
            + session_id.as_str().len()
            + stream_id.as_str().len();
        let encoded_bytes = FIXED_AUDIO_DATAGRAM_FIELD_BYTES + identifier_overhead + payload_bytes;
        let maximum_encoded_bytes = MAX_AUDIO_DATAGRAM_BYTES - FRAME_HEADER_BYTES;
        if encoded_bytes > maximum_encoded_bytes {
            return Err(PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                format!(
                    "packet duration produces a {encoded_bytes}-byte datagram, exceeding the \
                     {maximum_encoded_bytes}-byte bounded audio datagram limit"
                ),
            ));
        }
        Ok(Self {
            session_id,
            stream_id,
            format,
            packet_duration_ms,
            samples_per_packet,
            host_start_time_ms,
            next_sequence: 0,
            carry: Vec::new(),
            carry_first_sample_index: None,
            finished: false,
        })
    }

    /// Returns the exact `StreamStart` control message announcing this
    /// packetizer's session/stream identity and packet geometry.
    #[must_use]
    pub fn stream_start_message(&self) -> ProtocolFrame {
        ProtocolFrame::Control(ControlMessage::StreamStart(StreamStart {
            session_id: self.session_id.clone(),
            stream_id: self.stream_id.clone(),
            host_start_time_ms: self.host_start_time_ms,
            sample_rate: self.format.sample_rate_hz,
            channels: self.format.channels,
            samples_per_packet: self.samples_per_packet,
        }))
    }

    /// Returns true once the final chunk has been processed.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.finished
    }

    /// Consumes one decoded chunk, returning every full-width datagram it
    /// completes. A short final chunk is padded with silence to the
    /// configured packet width and its datagram is included here.
    ///
    /// # Errors
    ///
    /// Returns [`PacketizerErrorKind::FormatMismatch`] if the chunk's format
    /// no longer matches the format this packetizer was created with, or
    /// [`PacketizerErrorKind::AlreadyFinished`] if end-of-stream was already
    /// reached by an earlier chunk.
    pub fn push_chunk(
        &mut self,
        chunk: DecodedPcmChunk,
    ) -> Result<PacketizeOutcome, PacketizerError> {
        if self.finished {
            return Err(PacketizerError::new(
                PacketizerErrorKind::AlreadyFinished,
                "cannot push a chunk after end-of-stream was already emitted",
            ));
        }
        if chunk.format != self.format {
            return Err(PacketizerError::new(
                PacketizerErrorKind::FormatMismatch,
                "decoded chunk format no longer matches the packetizer's fixed format",
            ));
        }

        if self.carry.is_empty() {
            self.carry_first_sample_index = Some(chunk.first_sample_index.get());
        }
        let end_of_stream = chunk.end_of_stream;
        self.carry.extend(chunk.frames);

        let mut outcome = PacketizeOutcome::default();
        let samples_per_frame = self.format.samples_per_frame();
        let samples_per_full_packet = self.samples_per_packet as usize * samples_per_frame;

        while self.carry.len() >= samples_per_full_packet {
            let payload_samples: Vec<i16> = self.carry.drain(..samples_per_full_packet).collect();
            outcome.frames.push(self.build_datagram(&payload_samples));
            self.advance_carry_start(self.samples_per_packet);
        }

        if end_of_stream {
            if !self.carry.is_empty() {
                let mut payload_samples = std::mem::take(&mut self.carry);
                payload_samples.resize(samples_per_full_packet, 0);
                outcome.frames.push(self.build_datagram(&payload_samples));
            }
            self.finished = true;
            outcome.end_of_stream = true;
        }

        Ok(outcome)
    }

    fn advance_carry_start(&mut self, samples_per_packet: u32) {
        if let Some(start) = self.carry_first_sample_index {
            self.carry_first_sample_index = Some(start + u64::from(samples_per_packet));
        }
    }

    fn build_datagram(&mut self, payload_samples: &[i16]) -> ProtocolFrame {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        let first_sample_index = self.carry_first_sample_index.unwrap_or(0);
        let host_presentation_time_ms =
            self.host_start_time_ms.get() + sequence * u64::from(self.packet_duration_ms);
        let payload = payload_samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        ProtocolFrame::Audio(AudioDatagram {
            session_id: self.session_id.clone(),
            stream_id: self.stream_id.clone(),
            sequence: PacketSequence::new(sequence),
            codec: AudioCodec::PcmS16Le,
            sample_rate: self.format.sample_rate_hz,
            channels: self.format.channels,
            samples_per_packet: self.samples_per_packet,
            first_sample_index: SampleIndex::new(first_sample_index),
            host_presentation_time_ms: MonotonicMillis::new(host_presentation_time_ms),
            payload,
        })
    }
}
