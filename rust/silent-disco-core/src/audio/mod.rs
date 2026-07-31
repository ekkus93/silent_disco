mod decoder;
mod resampler;
mod source;
mod types;

pub use decoder::StreamingDecodeHandle;
pub use types::{
    AudioFormat, AudioSampleFormat, DecodeError, DecodeErrorKind, DecodeStatistics,
    DecodeStreamInfo, DecodeSummary, DecodeWorkerState, DecodedPcmChunk, StreamingDecodeConfig,
    CANONICAL_CHANNELS, CANONICAL_SAMPLE_RATE_HZ, DEFAULT_DECODE_CHUNK_FRAMES,
    DEFAULT_DECODE_QUEUE_CHUNKS, MAX_DECODE_CHUNK_FRAMES, MAX_DECODE_QUEUE_CHUNKS,
    MAX_SOURCE_BYTES,
};

#[cfg(test)]
mod tests;
