use crate::domain::SampleIndex;
use core::fmt;
use std::error::Error;

/// Canonical host-stream sample rate selected by Desktop Block 18.
pub const CANONICAL_SAMPLE_RATE_HZ: u32 = 48_000;
/// Canonical host-stream channel count selected by Desktop Block 18.
pub const CANONICAL_CHANNELS: u16 = 2;
/// Default 20 ms decode chunk at the canonical sample rate.
pub const DEFAULT_DECODE_CHUNK_FRAMES: usize = 960;
/// Default bounded queue capacity for decoded chunks.
pub const DEFAULT_DECODE_QUEUE_CHUNKS: usize = 8;
/// Largest caller-selectable decoded chunk, equal to one canonical second.
pub const MAX_DECODE_CHUNK_FRAMES: usize = 48_000;
/// Largest caller-selectable bounded queue capacity.
pub const MAX_DECODE_QUEUE_CHUNKS: usize = 64;
/// Hard ceiling shared with desktop staged-source inspection.
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Lowest source sample rate accepted by the initial conversion policy.
pub const MIN_SOURCE_SAMPLE_RATE_HZ: u32 = 8_000;
/// Highest source sample rate accepted by the initial conversion policy.
pub const MAX_SOURCE_SAMPLE_RATE_HZ: u32 = 192_000;
/// The initial decoder conversion policy accepts mono and stereo only.
pub const MAX_SOURCE_CHANNELS: usize = 2;

/// Sample representation carried across the shared host packetization boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSampleFormat {
    /// Signed 16-bit little-endian PCM samples.
    PcmS16Le,
}

/// Complete semantic format of one decoded PCM stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Frames per second.
    pub sample_rate_hz: u32,
    /// Interleaved channels per frame.
    pub channels: u16,
    /// Representation of every sample.
    pub sample_format: AudioSampleFormat,
}

impl AudioFormat {
    /// Canonical 48 kHz stereo PCM16 format selected by Desktop Block 18.
    pub const CANONICAL: Self = Self {
        sample_rate_hz: CANONICAL_SAMPLE_RATE_HZ,
        channels: CANONICAL_CHANNELS,
        sample_format: AudioSampleFormat::PcmS16Le,
    };

    /// Returns the interleaved sample count in one frame.
    #[must_use]
    pub const fn samples_per_frame(self) -> usize {
        self.channels as usize
    }
}

/// One bounded canonical PCM chunk produced incrementally by the decoder worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPcmChunk {
    /// Canonical format. Format changes are rejected rather than surfaced here.
    pub format: AudioFormat,
    /// Index of the first frame in this chunk.
    pub first_sample_index: SampleIndex,
    /// Interleaved signed PCM samples. The vector is bounded by the configuration.
    pub frames: Vec<i16>,
    /// True only on the final non-empty chunk.
    pub end_of_stream: bool,
}

impl DecodedPcmChunk {
    /// Returns the number of interleaved PCM frames in this chunk.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len() / self.format.samples_per_frame()
    }
}

/// Resource bounds for one streaming decode worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingDecodeConfig {
    /// Maximum canonical frames in one emitted chunk.
    pub chunk_frames: usize,
    /// Maximum chunks buffered between the worker and consumer.
    pub queue_chunks: usize,
    /// Maximum encoded source bytes accepted by this worker.
    pub max_source_bytes: u64,
}

impl Default for StreamingDecodeConfig {
    fn default() -> Self {
        Self {
            chunk_frames: DEFAULT_DECODE_CHUNK_FRAMES,
            queue_chunks: DEFAULT_DECODE_QUEUE_CHUNKS,
            max_source_bytes: MAX_SOURCE_BYTES,
        }
    }
}

impl StreamingDecodeConfig {
    /// Validates every configured bound before any source is opened.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeErrorKind::InvalidConfiguration`] when a bound is zero
    /// or exceeds its shared hard ceiling.
    pub fn validate(self) -> Result<Self, DecodeError> {
        if self.chunk_frames == 0 || self.chunk_frames > MAX_DECODE_CHUNK_FRAMES {
            return Err(DecodeError::new(
                DecodeErrorKind::InvalidConfiguration,
                "decoder chunk frame count is outside the supported range",
            ));
        }
        if self.queue_chunks == 0 || self.queue_chunks > MAX_DECODE_QUEUE_CHUNKS {
            return Err(DecodeError::new(
                DecodeErrorKind::InvalidConfiguration,
                "decoder queue chunk count is outside the supported range",
            ));
        }
        if self.max_source_bytes == 0 || self.max_source_bytes > MAX_SOURCE_BYTES {
            return Err(DecodeError::new(
                DecodeErrorKind::InvalidConfiguration,
                "decoder source byte limit is outside the supported range",
            ));
        }
        Ok(self)
    }

    /// Returns the maximum decoded duration that the bounded queue can retain.
    #[must_use]
    pub fn maximum_queued_duration_ms(self) -> u64 {
        let chunk_frames = u64::try_from(self.chunk_frames).unwrap_or(u64::MAX);
        let queue_chunks = u64::try_from(self.queue_chunks).unwrap_or(u64::MAX);
        chunk_frames
            .saturating_mul(queue_chunks)
            .saturating_mul(1_000)
            / u64::from(CANONICAL_SAMPLE_RATE_HZ)
    }
}

/// Valid source facts known without inventing duration or position metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeStreamInfo {
    /// Format emitted by the worker.
    pub format: AudioFormat,
    /// Declared source sample rate when the container supplied one.
    pub source_sample_rate_hz: Option<u32>,
    /// Declared source channels when the container supplied them.
    pub source_channels: Option<u16>,
    /// Source duration only when a reliable value is available.
    pub source_duration_ms: Option<u64>,
    /// Encoded source size observed before worker creation.
    pub source_byte_length: u64,
}

/// Lifecycle state of one decode worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeWorkerState {
    /// Worker is decoding or blocked by bounded backpressure.
    Running,
    /// Worker emitted a final chunk and exited successfully.
    Completed,
    /// Worker observed explicit cancellation or consumer disconnect.
    Cancelled,
    /// Worker exited with a typed decode failure.
    Failed,
}

/// Observable bounded-queue and worker progress counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeStatistics {
    /// Current worker lifecycle state.
    pub state: DecodeWorkerState,
    /// Chunks currently retained by the bounded channel.
    pub queued_chunks: usize,
    /// Configured bounded channel capacity.
    pub queue_capacity_chunks: usize,
    /// Maximum decoded milliseconds retained by a full channel.
    pub maximum_queued_duration_ms: u64,
    /// Number of chunks that encountered a full channel at least once.
    pub backpressure_events: u64,
    /// Canonical frames successfully handed to the bounded channel.
    pub emitted_frames: u64,
}

/// Stable failure taxonomy for source preparation and streaming decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeErrorKind {
    /// Caller supplied an invalid resource bound.
    InvalidConfiguration,
    /// Source could not be inspected or read.
    Io,
    /// Container, codec, sample rate, or channel layout is unsupported.
    UnsupportedFormat,
    /// Encoded audio data is malformed or truncated.
    CorruptInput,
    /// Bounded metadata parsing rejected malformed metadata.
    InvalidMetadata,
    /// Source file or decoded stream contains no frames.
    EmptySource,
    /// A checked arithmetic or decoder resource ceiling was exceeded.
    ResourceLimit,
    /// Source format changed after canonical conversion began.
    FormatChanged,
    /// Worker was explicitly cancelled or lost its consumer.
    Cancelled,
    /// Worker thread panicked or violated an internal invariant.
    WorkerPanicked,
}

/// Typed streaming decode failure with a user-safe diagnostic message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    /// Stable semantic failure category.
    pub kind: DecodeErrorKind,
    /// Human-readable diagnostic without native path disclosure.
    pub message: String,
}

impl DecodeError {
    pub(crate) fn new(kind: DecodeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for DecodeError {}

/// Terminal worker accounting returned by [`super::StreamingDecodeHandle::join`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeSummary {
    /// Terminal worker state.
    pub state: DecodeWorkerState,
    /// Total canonical frames handed to the bounded channel.
    pub emitted_frames: u64,
    /// Number of chunks that observed bounded backpressure.
    pub backpressure_events: u64,
}
