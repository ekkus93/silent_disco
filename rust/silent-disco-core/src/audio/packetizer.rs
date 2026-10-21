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
///
/// Five milliseconds, not the 20ms this project originally targeted, because
/// 20ms of 48kHz stereo PCM16 is a 3,840-byte payload — a 3,930-byte datagram
/// that IP fragments into three pieces at a 1,500-byte MTU. IP has no partial
/// recovery, so losing any one fragment destroys the whole packet: a single
/// lost link-layer frame cost 20ms of audio rather than the ~6.7ms it
/// actually carried. Measured on a real device, that amplified an underlying
/// ~0.15% fragment loss into 0.46% packet loss.
///
/// At 5ms the datagram is 1,050 bytes and travels unfragmented, so one lost
/// frame costs 5ms and nothing more. Per-packet overhead rises from 2.3% to
/// 8.6% of the wire bytes, which at roughly 1.7Mbps is not a constraint on
/// any network that can carry the audio at all.
pub const DEFAULT_PACKET_DURATION_MS: u32 = 5;
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
#[derive(Clone, PartialEq, Default)]
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
    samples_per_packet: u32,
    host_start_time_ms: MonotonicMillis,
    next_sequence: u64,
    carry: Vec<i16>,
    carry_first_sample_index: Option<u64>,
    contiguous_sample_cursor: u64,
    expected_chunk_sample_index: Option<u64>,
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
        if format.samples_per_frame() == 0 {
            return Err(PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "packetizer format must have at least one audio channel",
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
            samples_per_packet,
            host_start_time_ms,
            next_sequence: 0,
            carry: Vec::new(),
            carry_first_sample_index: None,
            contiguous_sample_cursor: 0,
            expected_chunk_sample_index: None,
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
            current_entry_id: None,
            current_display_name: None,
        }))
    }

    /// Returns true once the final chunk has been processed.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.finished
    }

    /// Returns the first sample index available to the next adjacent track.
    ///
    /// Use this with [`Self::push_contiguous_chunk`] when each source decoder
    /// starts its local sample indices at zero. It advances by decoded frames,
    /// so container duration rounding cannot introduce a gap or overlap.
    #[must_use]
    pub const fn next_contiguous_sample_index(&self) -> u64 {
        self.contiguous_sample_cursor
    }

    /// Returns the owning session identity.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Returns the identity of this continuous stream.
    #[must_use]
    pub fn stream_id(&self) -> &StreamId {
        &self.stream_id
    }

    /// Pushes a decoded chunk from a track in one continuous stream timeline.
    /// The source-local sample index is replaced with the stream cursor while
    /// the decoded PCM payload and end-of-stream flag are preserved.
    ///
    /// Track decoders can therefore be switched without flushing or padding
    /// packetizer carry at the boundary. Only the final queue item should set
    /// `chunk.end_of_stream`.
    ///
    /// # Errors
    ///
    /// Returns [`PacketizerError`] when the stream sample cursor overflows or
    /// the chunk violates the packetizer's format/end-of-stream contract.
    pub fn push_contiguous_chunk(
        &mut self,
        mut chunk: DecodedPcmChunk,
    ) -> Result<PacketizeOutcome, PacketizerError> {
        self.validate_chunk(&chunk)?;
        let frame_count = u64::try_from(chunk.frame_count()).map_err(|_| {
            PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "decoded chunk frame count exceeds the stream timeline",
            )
        })?;
        let next_cursor = self
            .contiguous_sample_cursor
            .checked_add(frame_count)
            .ok_or_else(|| {
                PacketizerError::new(
                    PacketizerErrorKind::InvalidConfiguration,
                    "continuous stream sample index overflowed",
                )
            })?;
        chunk.first_sample_index = crate::domain::SampleIndex::new(self.contiguous_sample_cursor);
        let outcome = self.push_chunk(chunk)?;
        self.contiguous_sample_cursor = next_cursor;
        Ok(outcome)
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
        self.validate_chunk(&chunk)?;
        let first = chunk.first_sample_index.get();
        if self
            .expected_chunk_sample_index
            .is_some_and(|next| first != next)
        {
            return Err(PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "decoded PCM chunks have a gap or overlap in sample indices",
            ));
        }
        let frames = u64::try_from(chunk.frame_count()).map_err(|_| {
            PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "decoded PCM chunk exceeds the sample timeline",
            )
        })?;
        let next_sample_index = first.checked_add(frames).ok_or_else(|| {
            PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "decoded PCM chunk sample index overflowed",
            )
        })?;

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

        self.expected_chunk_sample_index = Some(next_sample_index);
        Ok(outcome)
    }

    fn validate_chunk(&self, chunk: &DecodedPcmChunk) -> Result<(), PacketizerError> {
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
        if !chunk
            .frames
            .len()
            .is_multiple_of(self.format.samples_per_frame())
        {
            return Err(PacketizerError::new(
                PacketizerErrorKind::InvalidConfiguration,
                "decoded PCM chunk does not contain complete interleaved sample frames",
            ));
        }
        Ok(())
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
        // Nominal packet duration is not the exact duration of its integer
        // frame count (e.g. 220 samples at 44.1 kHz is not quite 5 ms).
        // Derive deadlines from emitted stream-relative PCM frame counts,
        // not absolute source sample indices (which may start nonzero) or
        // nominal packet milliseconds. Round using wide integer arithmetic.
        let sample_offset_ms = (u128::from(sequence) * u128::from(self.samples_per_packet) * 1_000
            + u128::from(self.format.sample_rate_hz / 2))
            / u128::from(self.format.sample_rate_hz);
        let host_presentation_time_ms = self
            .host_start_time_ms
            .get()
            .saturating_add(u64::try_from(sample_offset_ms).unwrap_or(u64::MAX));
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
