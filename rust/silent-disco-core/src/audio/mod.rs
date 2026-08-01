mod decoder;
mod jitter_buffer;
mod packetizer;
mod packetizer_worker;
mod resampler;
mod source;
mod types;

pub use decoder::StreamingDecodeHandle;
pub use jitter_buffer::{
    DEFAULT_MAX_BUFFERED_DURATION_MS, DEFAULT_MAX_REORDER_WINDOW, JitterBuffer, JitterBufferConfig,
    JitterBufferConfigError, JitterBufferConfigErrorKind, JitterBufferRejection,
    JitterBufferRejectionKind, JitterBufferStatistics, MAX_BUFFERED_DURATION_LIMIT_MS,
    MAX_REORDER_WINDOW_LIMIT,
};
pub use packetizer::{
    DEFAULT_PACKET_DURATION_MS, MAX_PACKET_DURATION_MS, MIN_PACKET_DURATION_MS, PacketizeOutcome,
    Packetizer, PacketizerError, PacketizerErrorKind,
};
pub use packetizer_worker::{
    DEFAULT_PACKETIZER_QUEUE_CAPACITY, MAX_PACKETIZER_QUEUE_CAPACITY, PacketizerSummary,
    PacketizerWorkerError, PacketizerWorkerErrorKind, PacketizerWorkerState,
    StreamingPacketizeConfig, StreamingPacketizeHandle,
};
pub use types::{
    AudioFormat, AudioSampleFormat, CANONICAL_CHANNELS, CANONICAL_SAMPLE_RATE_HZ,
    DEFAULT_DECODE_CHUNK_FRAMES, DEFAULT_DECODE_QUEUE_CHUNKS, DecodeError, DecodeErrorKind,
    DecodeStatistics, DecodeStreamInfo, DecodeSummary, DecodeWorkerState, DecodedPcmChunk,
    MAX_DECODE_CHUNK_FRAMES, MAX_DECODE_QUEUE_CHUNKS, MAX_SOURCE_BYTES, StreamingDecodeConfig,
};

#[cfg(test)]
mod jitter_buffer_tests;
#[cfg(test)]
mod packetizer_tests;
#[cfg(test)]
mod tests;
